from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing block in {path}:\n{old[:300]}")
    path.write_text(text.replace(old, new, 1))

# Add level metadata to SSTable descriptors. New flushes are always L0.
sstable = Path("crates/db-storage-lsm/src/sstable.rs")
replace_once(
    sstable,
    '''    pub(super) durable_sequence: u64,\n    pub(super) smallest_key: Vec<u8>,''',
    '''    pub(super) durable_sequence: u64,\n    pub(super) level: u32,\n    pub(super) smallest_key: Vec<u8>,''',
)
replace_once(
    sstable,
    '''            durable_sequence,\n            smallest_key: entries''',
    '''            durable_sequence,\n            level: 0,\n            smallest_key: entries''',
)

# Manifest v3 keeps the 80-byte header but expands each descriptor with level + reserved fields.
manifest = Path("crates/db-storage-lsm/src/manifest.rs")
text = manifest.read_text()
text = text.replace(
    '''const MANIFEST_FORMAT_VERSION_V1: u16 = 1;\nconst MANIFEST_FORMAT_VERSION: u16 = 2;\nconst MANIFEST_HEADER_LEN_V1: usize = 64;\nconst MANIFEST_HEADER_LEN: usize = 80;\nconst DESCRIPTOR_PREFIX_LEN: usize = 40;''',
    '''const MANIFEST_FORMAT_VERSION_V1: u16 = 1;\nconst MANIFEST_FORMAT_VERSION_V2: u16 = 2;\nconst MANIFEST_FORMAT_VERSION: u16 = 3;\nconst MANIFEST_HEADER_LEN_V1: usize = 64;\nconst MANIFEST_HEADER_LEN: usize = 80;\nconst LEGACY_DESCRIPTOR_PREFIX_LEN: usize = 40;\nconst DESCRIPTOR_PREFIX_LEN: usize = 48;''',
    1,
)
text = text.replace(
    '''use crate::sstable::SstableDescriptor;''',
    '''use crate::compaction::validate_level_invariants;\nuse crate::sstable::SstableDescriptor;''',
    1,
)
text = text.replace(
    '''        body.extend_from_slice(&encode_descriptor(descriptor)?);''',
    '''        body.extend_from_slice(&encode_descriptor(descriptor)?);''',
    1,
)
# Replace manifest version decoding block so v2 remains readable and both v2/v3 use the 80-byte header.
old = '''    let format_version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"));\n    let (header_len, wal_id, wal_first_sequence) = match format_version {\n        MANIFEST_FORMAT_VERSION_V1 => {\n            if u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize\n                != MANIFEST_HEADER_LEN_V1\n                || bytes[12..16] != [0; 4]\n                || bytes[48..60].iter().any(|byte| *byte != 0)\n            {\n                return Err(corruption(10, "invalid v1 manifest header fields"));\n            }\n            let expected_header_crc =\n                u32::from_le_bytes(bytes[60..64].try_into().expect("fixed slice"));\n            if crc32fast::hash(&bytes[..60]) != expected_header_crc {\n                return Err(corruption(60, "manifest header checksum mismatch"));\n            }\n            (MANIFEST_HEADER_LEN_V1, 1, 1)\n        }\n        MANIFEST_FORMAT_VERSION => {\n            if bytes.len() < MANIFEST_HEADER_LEN + 4\n                || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize\n                    != MANIFEST_HEADER_LEN\n                || bytes[12..16] != [0; 4]\n                || bytes[64..76].iter().any(|byte| *byte != 0)\n            {\n                return Err(corruption(10, "invalid v2 manifest header fields"));\n            }\n            let expected_header_crc =\n                u32::from_le_bytes(bytes[76..80].try_into().expect("fixed slice"));\n            if crc32fast::hash(&bytes[..76]) != expected_header_crc {\n                return Err(corruption(76, "manifest header checksum mismatch"));\n            }\n            (\n                MANIFEST_HEADER_LEN,\n                u64::from_le_bytes(bytes[48..56].try_into().expect("fixed slice")),\n                u64::from_le_bytes(bytes[56..64].try_into().expect("fixed slice")),\n            )\n        }\n        _ => {\n            return Err(DbError::UnsupportedVersion {\n                format: "LSM manifest",\n                found: u64::from(format_version),\n                supported: u64::from(MANIFEST_FORMAT_VERSION),\n            });\n        }\n    };'''
new = '''    let format_version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"));\n    let (header_len, wal_id, wal_first_sequence) = match format_version {\n        MANIFEST_FORMAT_VERSION_V1 => {\n            if u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize\n                != MANIFEST_HEADER_LEN_V1\n                || bytes[12..16] != [0; 4]\n                || bytes[48..60].iter().any(|byte| *byte != 0)\n            {\n                return Err(corruption(10, "invalid v1 manifest header fields"));\n            }\n            let expected_header_crc =\n                u32::from_le_bytes(bytes[60..64].try_into().expect("fixed slice"));\n            if crc32fast::hash(&bytes[..60]) != expected_header_crc {\n                return Err(corruption(60, "manifest header checksum mismatch"));\n            }\n            (MANIFEST_HEADER_LEN_V1, 1, 1)\n        }\n        MANIFEST_FORMAT_VERSION_V2 | MANIFEST_FORMAT_VERSION => {\n            if bytes.len() < MANIFEST_HEADER_LEN + 4\n                || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize\n                    != MANIFEST_HEADER_LEN\n                || bytes[12..16] != [0; 4]\n                || bytes[64..76].iter().any(|byte| *byte != 0)\n            {\n                return Err(corruption(\n                    10,\n                    format!("invalid v{format_version} manifest header fields"),\n                ));\n            }\n            let expected_header_crc =\n                u32::from_le_bytes(bytes[76..80].try_into().expect("fixed slice"));\n            if crc32fast::hash(&bytes[..76]) != expected_header_crc {\n                return Err(corruption(76, "manifest header checksum mismatch"));\n            }\n            (\n                MANIFEST_HEADER_LEN,\n                u64::from_le_bytes(bytes[48..56].try_into().expect("fixed slice")),\n                u64::from_le_bytes(bytes[56..64].try_into().expect("fixed slice")),\n            )\n        }\n        _ => {\n            return Err(DbError::UnsupportedVersion {\n                format: "LSM manifest",\n                found: u64::from(format_version),\n                supported: u64::from(MANIFEST_FORMAT_VERSION),\n            });\n        }\n    };'''
if old not in text:
    raise SystemExit("manifest version decode block missing")
text = text.replace(old, new, 1)
text = text.replace(
    '''        let (descriptor, next) = decode_descriptor(&bytes, offset, body_end)?;''',
    '''        let (descriptor, next) =\n            decode_descriptor(&bytes, offset, body_end, format_version)?;''',
    1,
)
# Replace descriptor encoder.
start = text.index("fn encode_descriptor(")
end = text.index("fn decode_descriptor(", start)
encoder = '''fn encode_descriptor(descriptor: &SstableDescriptor) -> Result<Vec<u8>> {\n    if descriptor.smallest_key.len() > MAX_KEY_BYTES || descriptor.largest_key.len() > MAX_KEY_BYTES\n    {\n        return Err(corruption(\n            0,\n            "manifest SSTable key bound exceeds common key limit",\n        ));\n    }\n    let smallest_len = u32::try_from(descriptor.smallest_key.len())\n        .map_err(|_| corruption(0, "manifest smallest-key length does not fit u32"))?;\n    let largest_len = u32::try_from(descriptor.largest_key.len())\n        .map_err(|_| corruption(0, "manifest largest-key length does not fit u32"))?;\n    let capacity = DESCRIPTOR_PREFIX_LEN\n        .checked_add(descriptor.smallest_key.len())\n        .and_then(|len| len.checked_add(descriptor.largest_key.len()))\n        .and_then(|len| len.checked_add(4))\n        .ok_or_else(|| corruption(0, "manifest descriptor size overflowed usize"))?;\n    let mut bytes = Vec::with_capacity(capacity);\n    bytes.extend_from_slice(&descriptor.table_id.to_le_bytes());\n    bytes.extend_from_slice(&descriptor.file_bytes.to_le_bytes());\n    bytes.extend_from_slice(&descriptor.entry_count.to_le_bytes());\n    bytes.extend_from_slice(&descriptor.durable_sequence.to_le_bytes());\n    bytes.extend_from_slice(&descriptor.level.to_le_bytes());\n    bytes.extend_from_slice(&0_u32.to_le_bytes());\n    bytes.extend_from_slice(&smallest_len.to_le_bytes());\n    bytes.extend_from_slice(&largest_len.to_le_bytes());\n    bytes.extend_from_slice(&descriptor.smallest_key);\n    bytes.extend_from_slice(&descriptor.largest_key);\n    let crc = crc32fast::hash(&bytes);\n    bytes.extend_from_slice(&crc.to_le_bytes());\n    Ok(bytes)\n}\n\n'''
text = text[:start] + encoder + text[end:]
# Replace descriptor decoder through validate_version_set.
start = text.index("fn decode_descriptor(")
end = text.index("fn validate_version_set(", start)
decoder = '''fn decode_descriptor(\n    bytes: &[u8],\n    offset: usize,\n    limit: usize,\n    manifest_format_version: u16,\n) -> Result<(SstableDescriptor, usize)> {\n    let prefix_len = if manifest_format_version >= MANIFEST_FORMAT_VERSION {\n        DESCRIPTOR_PREFIX_LEN\n    } else {\n        LEGACY_DESCRIPTOR_PREFIX_LEN\n    };\n    let prefix_end = offset\n        .checked_add(prefix_len)\n        .ok_or_else(|| corruption(offset as u64, "manifest descriptor extent overflowed"))?;\n    if prefix_end > limit {\n        return Err(corruption(\n            offset as u64,\n            "truncated manifest descriptor prefix",\n        ));\n    }\n    let prefix = &bytes[offset..prefix_end];\n    let (level, smallest_len_offset, largest_len_offset) =\n        if manifest_format_version >= MANIFEST_FORMAT_VERSION {\n            if u32::from_le_bytes(prefix[36..40].try_into().expect("fixed slice")) != 0 {\n                return Err(corruption(\n                    offset as u64 + 36,\n                    "manifest SSTable descriptor reserved field is nonzero",\n                ));\n            }\n            (\n                u32::from_le_bytes(prefix[32..36].try_into().expect("fixed slice")),\n                40,\n                44,\n            )\n        } else {\n            (0, 32, 36)\n        };\n    let smallest_len = usize::try_from(u32::from_le_bytes(\n        prefix[smallest_len_offset..smallest_len_offset + 4]\n            .try_into()\n            .expect("fixed slice"),\n    ))\n    .map_err(|_| corruption(offset as u64 + smallest_len_offset as u64, "smallest-key length does not fit usize"))?;\n    let largest_len = usize::try_from(u32::from_le_bytes(\n        prefix[largest_len_offset..largest_len_offset + 4]\n            .try_into()\n            .expect("fixed slice"),\n    ))\n    .map_err(|_| corruption(offset as u64 + largest_len_offset as u64, "largest-key length does not fit usize"))?;\n    if smallest_len > MAX_KEY_BYTES || largest_len > MAX_KEY_BYTES {\n        return Err(corruption(\n            offset as u64 + smallest_len_offset as u64,\n            "manifest key bound exceeds common key limit",\n        ));\n    }\n    let smallest_end = prefix_end\n        .checked_add(smallest_len)\n        .ok_or_else(|| corruption(offset as u64, "manifest smallest-key extent overflowed"))?;\n    let largest_end = smallest_end\n        .checked_add(largest_len)\n        .ok_or_else(|| corruption(offset as u64, "manifest largest-key extent overflowed"))?;\n    let descriptor_end = largest_end.checked_add(4).ok_or_else(|| {\n        corruption(\n            offset as u64,\n            "manifest descriptor checksum extent overflowed",\n        )\n    })?;\n    if descriptor_end > limit {\n        return Err(corruption(offset as u64, "truncated manifest descriptor"));\n    }\n    let expected_crc = u32::from_le_bytes(\n        bytes[largest_end..descriptor_end]\n            .try_into()\n            .expect("fixed slice"),\n    );\n    if crc32fast::hash(&bytes[offset..largest_end]) != expected_crc {\n        return Err(corruption(\n            largest_end as u64,\n            "manifest descriptor checksum mismatch",\n        ));\n    }\n    let descriptor = SstableDescriptor {\n        table_id: u64::from_le_bytes(prefix[0..8].try_into().expect("fixed slice")),\n        file_bytes: u64::from_le_bytes(prefix[8..16].try_into().expect("fixed slice")),\n        entry_count: u64::from_le_bytes(prefix[16..24].try_into().expect("fixed slice")),\n        durable_sequence: u64::from_le_bytes(prefix[24..32].try_into().expect("fixed slice")),\n        level,\n        smallest_key: bytes[prefix_end..smallest_end].to_vec(),\n        largest_key: bytes[smallest_end..largest_end].to_vec(),\n    };\n    if descriptor.table_id == 0\n        || descriptor.file_bytes == 0\n        || descriptor.entry_count == 0\n        || descriptor.durable_sequence == 0\n        || descriptor.smallest_key > descriptor.largest_key\n    {\n        return Err(corruption(\n            offset as u64,\n            "invalid manifest SSTable descriptor values",\n        ));\n    }\n    Ok((descriptor, descriptor_end))\n}\n\n'''
text = text[:start] + decoder + text[end:]
# Replace version-set validation: compaction breaks the old strict durable-sequence-by-table-id assumption.
start = text.index("fn validate_version_set(")
end = text.index("fn corruption(", start)
validator = '''fn validate_version_set(\n    durable_sequence: u64,\n    wal_id: u64,\n    wal_first_sequence: u64,\n    tables: &[SstableDescriptor],\n) -> Result<()> {\n    if wal_id == 0 || wal_first_sequence == 0 {\n        return Err(corruption(\n            0,\n            "manifest WAL id and first sequence must both be nonzero",\n        ));\n    }\n    if wal_first_sequence > durable_sequence.saturating_add(1) {\n        return Err(corruption(\n            0,\n            format!(\n                "manifest WAL first sequence {wal_first_sequence} is beyond durable sequence {durable_sequence} + 1"\n            ),\n        ));\n    }\n\n    let mut previous_table_id = 0_u64;\n    let mut max_table_sequence = 0_u64;\n    for descriptor in tables {\n        if descriptor.table_id <= previous_table_id {\n            return Err(corruption(\n                0,\n                "manifest table ids are not strictly increasing",\n            ));\n        }\n        if descriptor.durable_sequence > durable_sequence {\n            return Err(corruption(\n                0,\n                format!(\n                    "SSTable {} sequence watermark {} exceeds manifest durable sequence {durable_sequence}",\n                    descriptor.table_id, descriptor.durable_sequence\n                ),\n            ));\n        }\n        previous_table_id = descriptor.table_id;\n        max_table_sequence = max_table_sequence.max(descriptor.durable_sequence);\n    }\n    if durable_sequence != max_table_sequence {\n        return Err(corruption(\n            0,\n            format!(\n                "manifest durable sequence {durable_sequence} does not equal maximum SSTable watermark {max_table_sequence}"\n            ),\n        ));\n    }\n    validate_level_invariants(tables)?;\n    Ok(())\n}\n\n'''
text = text[:start] + validator + text[end:]
manifest.write_text(text)

# Wire planner and expose structural readiness in stats. Point reads now compare table sequence numbers,
# removing the future compaction dependency on manifest table order.
lib = Path("crates/db-storage-lsm/src/lib.rs")
replace_once(
    lib,
    '''mod bloom;\nmod manifest;''',
    '''mod bloom;\nmod compaction;\nmod manifest;''',
)
replace_once(
    lib,
    '''use manifest::{VersionSet, CURRENT_FILE_NAME};''',
    '''use compaction::plan_l0_to_l1;\nuse manifest::{VersionSet, CURRENT_FILE_NAME};''',
)
replace_once(
    lib,
    '''    /// Immutable sorted tables referenced by the authoritative manifest.\n    pub sstables: usize,\n    /// Total indexed entries''',
    '''    /// Immutable sorted tables referenced by the authoritative manifest.\n    pub sstables: usize,\n    /// Level-zero tables, where key-range overlap is permitted.\n    pub level0_sstables: usize,\n    /// Tables in levels 1+, whose same-level key ranges must be disjoint.\n    pub leveled_sstables: usize,\n    /// Whether the deterministic L0-to-L1 planner currently has enough input tables to produce a plan.\n    pub compaction_pending: bool,\n    /// Total indexed entries''',
)
# Verify-report stats construction.
replace_once(
    lib,
    '''                sstables: version.tables.len(),\n                sstable_entries,''',
    '''                sstables: version.tables.len(),\n                level0_sstables: version.tables.iter().filter(|table| table.level == 0).count(),\n                leveled_sstables: version.tables.iter().filter(|table| table.level > 0).count(),\n                compaction_pending: plan_l0_to_l1(&version.tables)?.is_some(),\n                sstable_entries,''',
)
# Live stats construction.
replace_once(
    lib,
    '''            sstables: self.tables.len(),\n            sstable_entries,''',
    '''            sstables: self.tables.len(),\n            level0_sstables: self.version.tables.iter().filter(|table| table.level == 0).count(),\n            leveled_sstables: self.version.tables.iter().filter(|table| table.level > 0).count(),\n            compaction_pending: plan_l0_to_l1(&self.version.tables)?.is_some(),\n            sstable_entries,''',
)
# Replace first-match table point read with max-sequence selection.
replace_once(
    lib,
    '''        for table in self.tables.iter().rev() {\n            if let Some(entry) = table.get(key)? {\n                return Ok(Some(entry));\n            }\n        }\n        Ok(None)''',
    '''        let mut newest = None;\n        for table in &self.tables {\n            if let Some(entry) = table.get(key)? {\n                let replace = newest\n                    .as_ref()\n                    .is_none_or(|current: &VersionedEntry| entry.sequence > current.sequence);\n                if replace {\n                    newest = Some(entry);\n                }\n            }\n        }\n        Ok(newest)''',
)
lib.write_text(lib.read_text())

# Focused integration: four actual flushes become L0 and expose planner readiness without compacting data.
tests = Path("crates/db-storage-lsm/src/sstable_tests.rs")
text = tests.read_text()
if "four_flushes_expose_l0_compaction_readiness" not in text:
    text += r'''

#[test]
fn four_flushes_expose_l0_compaction_readiness_without_rewriting_tables() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");

    for flush in 0_u8..4 {
        let left = [b'a' + flush * 2];
        let right = [b'b' + flush * 2];
        engine
            .put(&left, &large_value(0x70 + flush))
            .expect("first large put");
        engine
            .put(&right, &large_value(0x80 + flush))
            .expect("second large put and flush");
    }

    let stats = engine.stats().expect("planner stats");
    assert_eq!(stats.sstables, 4);
    assert_eq!(stats.level0_sstables, 4);
    assert_eq!(stats.leveled_sstables, 0);
    assert!(stats.compaction_pending);
    engine.reopen().expect("reopen planner-ready state");
    assert_eq!(engine.stats().expect("reopened stats"), stats);
}
'''
    tests.write_text(text)
