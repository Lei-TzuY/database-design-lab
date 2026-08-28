use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use db_core::{DbError, Result, MAX_KEY_BYTES};

use crate::sstable::SstableDescriptor;

pub(super) const CURRENT_FILE_NAME: &str = "CURRENT";
pub(super) const CURRENT_SLOT_BYTES: usize = 4096;
const CURRENT_FILE_BYTES: usize = CURRENT_SLOT_BYTES * 2;
const CURRENT_MAGIC: [u8; 8] = *b"DBLSMCUR";
const MANIFEST_MAGIC: [u8; 8] = *b"DBLSMMAN";
const FORMAT_VERSION: u16 = 1;
const MANIFEST_HEADER_LEN: usize = 64;
const DESCRIPTOR_PREFIX_LEN: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VersionSet {
    pub(super) current_generation: u64,
    pub(super) manifest_id: u64,
    pub(super) durable_sequence: u64,
    pub(super) tables: Vec<SstableDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CurrentSlot {
    generation: u64,
    manifest_id: u64,
}

pub(super) fn manifest_file_name(manifest_id: u64) -> String {
    format!("MANIFEST-{manifest_id:016}")
}

pub(super) fn create_initial(directory: &Path) -> Result<VersionSet> {
    let version = VersionSet {
        current_generation: 0,
        manifest_id: 1,
        durable_sequence: 0,
        tables: Vec::new(),
    };
    write_manifest_new(directory, &version)?;

    let current_path = directory.join(CURRENT_FILE_NAME);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&current_path)?;
    let mut bytes = vec![0_u8; CURRENT_FILE_BYTES];
    let slot0 = encode_current_slot(0, version.current_generation, version.manifest_id);
    let slot1 = encode_current_slot(1, version.current_generation, version.manifest_id);
    bytes[..CURRENT_SLOT_BYTES].copy_from_slice(&slot0);
    bytes[CURRENT_SLOT_BYTES..].copy_from_slice(&slot1);
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(version)
}

pub(super) fn load(directory: &Path) -> Result<VersionSet> {
    let current = read_current(&directory.join(CURRENT_FILE_NAME))?;
    let mut version = read_manifest(directory, current.manifest_id)?;
    version.current_generation = current.generation;
    Ok(version)
}

pub(super) fn install(
    directory: &Path,
    current: &VersionSet,
    new_manifest_id: u64,
    durable_sequence: u64,
    tables: Vec<SstableDescriptor>,
) -> Result<VersionSet> {
    let generation = current
        .current_generation
        .checked_add(1)
        .ok_or_else(|| corruption(0, "CURRENT generation exhausted"))?;
    validate_table_set(durable_sequence, &tables)?;
    let next = VersionSet {
        current_generation: generation,
        manifest_id: new_manifest_id,
        durable_sequence,
        tables,
    };
    write_manifest_new(directory, &next)?;

    let slot_id = usize::try_from(generation % 2).expect("modulo two fits usize");
    let slot = encode_current_slot(slot_id, generation, new_manifest_id);
    let mut current_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join(CURRENT_FILE_NAME))?;
    current_file.seek(SeekFrom::Start(
        u64::try_from(slot_id * CURRENT_SLOT_BYTES).expect("CURRENT offset fits u64"),
    ))?;
    current_file.write_all(&slot)?;
    current_file.sync_data()?;
    Ok(next)
}

fn read_current(path: &Path) -> Result<CurrentSlot> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() != CURRENT_FILE_BYTES {
        return Err(corruption(
            0,
            format!(
                "CURRENT has {} bytes; expected {CURRENT_FILE_BYTES}",
                bytes.len()
            ),
        ));
    }
    let first = parse_current_slot(&bytes[..CURRENT_SLOT_BYTES], 0).ok();
    let second = parse_current_slot(&bytes[CURRENT_SLOT_BYTES..], 1).ok();
    match (first, second) {
        (None, None) => Err(corruption(0, "both CURRENT slots are invalid")),
        (Some(slot), None) | (None, Some(slot)) => Ok(slot),
        (Some(left), Some(right)) => {
            if left.generation == right.generation {
                if left.manifest_id != right.manifest_id {
                    return Err(corruption(
                        0,
                        "equal-generation CURRENT slots disagree on manifest id",
                    ));
                }
                Ok(left)
            } else {
                let (older, newer) = if left.generation < right.generation {
                    (left, right)
                } else {
                    (right, left)
                };
                if newer.generation - older.generation != 1 {
                    return Err(corruption(
                        0,
                        "CURRENT slot generations differ by more than one",
                    ));
                }
                Ok(newer)
            }
        }
    }
}

fn encode_current_slot(
    slot_id: usize,
    generation: u64,
    manifest_id: u64,
) -> [u8; CURRENT_SLOT_BYTES] {
    let mut bytes = [0_u8; CURRENT_SLOT_BYTES];
    bytes[0..8].copy_from_slice(&CURRENT_MAGIC);
    bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(slot_id as u16).to_le_bytes());
    bytes[16..24].copy_from_slice(&generation.to_le_bytes());
    bytes[24..32].copy_from_slice(&manifest_id.to_le_bytes());
    let crc = crc32fast::hash(&bytes[..CURRENT_SLOT_BYTES - 4]);
    bytes[CURRENT_SLOT_BYTES - 4..].copy_from_slice(&crc.to_le_bytes());
    bytes
}

fn parse_current_slot(bytes: &[u8], physical_slot: usize) -> Result<CurrentSlot> {
    let base = u64::try_from(physical_slot * CURRENT_SLOT_BYTES).expect("CURRENT offset fits u64");
    if bytes.len() != CURRENT_SLOT_BYTES || bytes[0..8] != CURRENT_MAGIC {
        return Err(corruption(base, "invalid CURRENT slot magic/length"));
    }
    let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"));
    if version != FORMAT_VERSION {
        return Err(DbError::UnsupportedVersion {
            format: "LSM CURRENT",
            found: u64::from(version),
            supported: u64::from(FORMAT_VERSION),
        });
    }
    let slot_id = u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice"));
    if usize::from(slot_id) != physical_slot {
        return Err(corruption(
            base + 10,
            "CURRENT slot id does not match physical slot",
        ));
    }
    if bytes[12..16] != [0; 4]
        || bytes[32..CURRENT_SLOT_BYTES - 4]
            .iter()
            .any(|byte| *byte != 0)
    {
        return Err(corruption(base + 12, "CURRENT reserved bytes are nonzero"));
    }
    let expected = u32::from_le_bytes(
        bytes[CURRENT_SLOT_BYTES - 4..]
            .try_into()
            .expect("fixed slice"),
    );
    if crc32fast::hash(&bytes[..CURRENT_SLOT_BYTES - 4]) != expected {
        return Err(corruption(
            base + CURRENT_SLOT_BYTES as u64 - 4,
            "CURRENT checksum mismatch",
        ));
    }
    let manifest_id = u64::from_le_bytes(bytes[24..32].try_into().expect("fixed slice"));
    if manifest_id == 0 {
        return Err(corruption(base + 24, "CURRENT manifest id must be nonzero"));
    }
    Ok(CurrentSlot {
        generation: u64::from_le_bytes(bytes[16..24].try_into().expect("fixed slice")),
        manifest_id,
    })
}

fn write_manifest_new(directory: &Path, version: &VersionSet) -> Result<()> {
    validate_table_set(version.durable_sequence, &version.tables)?;
    let mut body = Vec::new();
    for descriptor in &version.tables {
        body.extend_from_slice(&encode_descriptor(descriptor)?);
    }
    let body_len = u64::try_from(body.len())
        .map_err(|_| corruption(0, "manifest body length does not fit u64"))?;
    let mut bytes = vec![0_u8; MANIFEST_HEADER_LEN];
    bytes.extend_from_slice(&body);

    bytes[0..8].copy_from_slice(&MANIFEST_MAGIC);
    bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(MANIFEST_HEADER_LEN as u16).to_le_bytes());
    bytes[16..24].copy_from_slice(&version.manifest_id.to_le_bytes());
    bytes[24..32].copy_from_slice(&version.durable_sequence.to_le_bytes());
    bytes[32..40].copy_from_slice(
        &u64::try_from(version.tables.len())
            .map_err(|_| corruption(0, "manifest table count does not fit u64"))?
            .to_le_bytes(),
    );
    bytes[40..48].copy_from_slice(&body_len.to_le_bytes());
    let header_crc = crc32fast::hash(&bytes[..60]);
    bytes[60..64].copy_from_slice(&header_crc.to_le_bytes());
    let file_crc = crc32fast::hash(&bytes);
    bytes.extend_from_slice(&file_crc.to_le_bytes());

    let path = directory.join(manifest_file_name(version.manifest_id));
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_manifest(directory: &Path, manifest_id: u64) -> Result<VersionSet> {
    let path = directory.join(manifest_file_name(manifest_id));
    let bytes = fs::read(&path)?;
    if bytes.len() < MANIFEST_HEADER_LEN + 4 {
        return Err(corruption(0, "truncated manifest"));
    }
    if bytes[0..8] != MANIFEST_MAGIC {
        return Err(corruption(0, "invalid manifest magic"));
    }
    let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"));
    if version != FORMAT_VERSION {
        return Err(DbError::UnsupportedVersion {
            format: "LSM manifest",
            found: u64::from(version),
            supported: u64::from(FORMAT_VERSION),
        });
    }
    if u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize
        != MANIFEST_HEADER_LEN
        || bytes[12..16] != [0; 4]
        || bytes[48..60].iter().any(|byte| *byte != 0)
    {
        return Err(corruption(10, "invalid manifest header fields"));
    }
    let expected_header_crc = u32::from_le_bytes(bytes[60..64].try_into().expect("fixed slice"));
    if crc32fast::hash(&bytes[..60]) != expected_header_crc {
        return Err(corruption(60, "manifest header checksum mismatch"));
    }
    let encoded_manifest_id = u64::from_le_bytes(bytes[16..24].try_into().expect("fixed slice"));
    if encoded_manifest_id != manifest_id {
        return Err(corruption(
            16,
            "manifest id does not match filename/CURRENT",
        ));
    }
    let durable_sequence = u64::from_le_bytes(bytes[24..32].try_into().expect("fixed slice"));
    let table_count = usize::try_from(u64::from_le_bytes(
        bytes[32..40].try_into().expect("fixed slice"),
    ))
    .map_err(|_| corruption(32, "manifest table count does not fit usize"))?;
    let body_len = usize::try_from(u64::from_le_bytes(
        bytes[40..48].try_into().expect("fixed slice"),
    ))
    .map_err(|_| corruption(40, "manifest body length does not fit usize"))?;
    let expected_len = MANIFEST_HEADER_LEN
        .checked_add(body_len)
        .and_then(|len| len.checked_add(4))
        .ok_or_else(|| corruption(40, "manifest extent arithmetic overflowed usize"))?;
    if expected_len != bytes.len() {
        return Err(corruption(
            40,
            "manifest body length does not match physical file",
        ));
    }
    let crc_offset = bytes.len() - 4;
    let expected_file_crc =
        u32::from_le_bytes(bytes[crc_offset..].try_into().expect("fixed slice"));
    if crc32fast::hash(&bytes[..crc_offset]) != expected_file_crc {
        return Err(corruption(
            crc_offset as u64,
            "manifest file checksum mismatch",
        ));
    }

    let mut offset = MANIFEST_HEADER_LEN;
    let body_end = crc_offset;
    let mut tables = Vec::with_capacity(table_count);
    for _ in 0..table_count {
        let (descriptor, next) = decode_descriptor(&bytes, offset, body_end)?;
        tables.push(descriptor);
        offset = next;
    }
    if offset != body_end {
        return Err(corruption(
            offset as u64,
            "manifest contains unexplained descriptor bytes",
        ));
    }
    validate_table_set(durable_sequence, &tables)?;
    Ok(VersionSet {
        current_generation: 0,
        manifest_id,
        durable_sequence,
        tables,
    })
}

fn encode_descriptor(descriptor: &SstableDescriptor) -> Result<Vec<u8>> {
    if descriptor.smallest_key.len() > MAX_KEY_BYTES || descriptor.largest_key.len() > MAX_KEY_BYTES
    {
        return Err(corruption(
            0,
            "manifest SSTable key bound exceeds common key limit",
        ));
    }
    let smallest_len = u32::try_from(descriptor.smallest_key.len())
        .map_err(|_| corruption(0, "manifest smallest-key length does not fit u32"))?;
    let largest_len = u32::try_from(descriptor.largest_key.len())
        .map_err(|_| corruption(0, "manifest largest-key length does not fit u32"))?;
    let capacity = DESCRIPTOR_PREFIX_LEN
        .checked_add(descriptor.smallest_key.len())
        .and_then(|len| len.checked_add(descriptor.largest_key.len()))
        .and_then(|len| len.checked_add(4))
        .ok_or_else(|| corruption(0, "manifest descriptor size overflowed usize"))?;
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(&descriptor.table_id.to_le_bytes());
    bytes.extend_from_slice(&descriptor.file_bytes.to_le_bytes());
    bytes.extend_from_slice(&descriptor.entry_count.to_le_bytes());
    bytes.extend_from_slice(&descriptor.durable_sequence.to_le_bytes());
    bytes.extend_from_slice(&smallest_len.to_le_bytes());
    bytes.extend_from_slice(&largest_len.to_le_bytes());
    bytes.extend_from_slice(&descriptor.smallest_key);
    bytes.extend_from_slice(&descriptor.largest_key);
    let crc = crc32fast::hash(&bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
    Ok(bytes)
}

fn decode_descriptor(
    bytes: &[u8],
    offset: usize,
    limit: usize,
) -> Result<(SstableDescriptor, usize)> {
    let prefix_end = offset
        .checked_add(DESCRIPTOR_PREFIX_LEN)
        .ok_or_else(|| corruption(offset as u64, "manifest descriptor extent overflowed"))?;
    if prefix_end > limit {
        return Err(corruption(
            offset as u64,
            "truncated manifest descriptor prefix",
        ));
    }
    let prefix = &bytes[offset..prefix_end];
    let smallest_len = usize::try_from(u32::from_le_bytes(
        prefix[32..36].try_into().expect("fixed slice"),
    ))
    .map_err(|_| corruption(offset as u64 + 32, "smallest-key length does not fit usize"))?;
    let largest_len = usize::try_from(u32::from_le_bytes(
        prefix[36..40].try_into().expect("fixed slice"),
    ))
    .map_err(|_| corruption(offset as u64 + 36, "largest-key length does not fit usize"))?;
    if smallest_len > MAX_KEY_BYTES || largest_len > MAX_KEY_BYTES {
        return Err(corruption(
            offset as u64 + 32,
            "manifest key bound exceeds common key limit",
        ));
    }
    let smallest_end = prefix_end
        .checked_add(smallest_len)
        .ok_or_else(|| corruption(offset as u64, "manifest smallest-key extent overflowed"))?;
    let largest_end = smallest_end
        .checked_add(largest_len)
        .ok_or_else(|| corruption(offset as u64, "manifest largest-key extent overflowed"))?;
    let descriptor_end = largest_end.checked_add(4).ok_or_else(|| {
        corruption(
            offset as u64,
            "manifest descriptor checksum extent overflowed",
        )
    })?;
    if descriptor_end > limit {
        return Err(corruption(offset as u64, "truncated manifest descriptor"));
    }
    let expected_crc = u32::from_le_bytes(
        bytes[largest_end..descriptor_end]
            .try_into()
            .expect("fixed slice"),
    );
    if crc32fast::hash(&bytes[offset..largest_end]) != expected_crc {
        return Err(corruption(
            largest_end as u64,
            "manifest descriptor checksum mismatch",
        ));
    }
    let descriptor = SstableDescriptor {
        table_id: u64::from_le_bytes(prefix[0..8].try_into().expect("fixed slice")),
        file_bytes: u64::from_le_bytes(prefix[8..16].try_into().expect("fixed slice")),
        entry_count: u64::from_le_bytes(prefix[16..24].try_into().expect("fixed slice")),
        durable_sequence: u64::from_le_bytes(prefix[24..32].try_into().expect("fixed slice")),
        smallest_key: bytes[prefix_end..smallest_end].to_vec(),
        largest_key: bytes[smallest_end..largest_end].to_vec(),
    };
    if descriptor.table_id == 0
        || descriptor.file_bytes == 0
        || descriptor.entry_count == 0
        || descriptor.durable_sequence == 0
        || descriptor.smallest_key > descriptor.largest_key
    {
        return Err(corruption(
            offset as u64,
            "invalid manifest SSTable descriptor values",
        ));
    }
    Ok((descriptor, descriptor_end))
}

fn validate_table_set(durable_sequence: u64, tables: &[SstableDescriptor]) -> Result<()> {
    let mut previous_table_id = 0_u64;
    let mut previous_durable = 0_u64;
    for descriptor in tables {
        if descriptor.table_id <= previous_table_id {
            return Err(corruption(
                0,
                "manifest table ids are not strictly increasing",
            ));
        }
        if descriptor.durable_sequence <= previous_durable {
            return Err(corruption(
                0,
                "manifest SSTable durable sequences are not strictly increasing",
            ));
        }
        previous_table_id = descriptor.table_id;
        previous_durable = descriptor.durable_sequence;
    }
    let expected = tables.last().map_or(0, |table| table.durable_sequence);
    if durable_sequence != expected {
        return Err(corruption(
            0,
            format!(
                "manifest durable sequence {durable_sequence} does not equal latest table watermark {expected}"
            ),
        ));
    }
    Ok(())
}

fn corruption(offset: u64, reason: impl Into<String>) -> DbError {
    DbError::Corruption {
        offset,
        reason: reason.into(),
    }
}
