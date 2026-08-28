from pathlib import Path

lib_path = Path("crates/db-storage-lsm/src/lib.rs")
lib = lib_path.read_text()
if "mod bloom;" not in lib:
    lib = lib.replace(
        "mod manifest;\nmod memtable;\nmod sstable;\nmod wal;",
        "mod bloom;\nmod manifest;\nmod memtable;\nmod sstable;\nmod wal;",
        1,
    )
    lib = lib.replace(
        "//! The WAL deliberately retains complete history in this phase; `durable_sequence` lets reopen skip the\n//! prefix already represented by published SSTables. Bloom filters, WAL reclamation, levels, and\n//! compaction remain later Phase 3 work.",
        "//! WAL segments rotate only after published SSTables cover their complete sequence range. SSTable v2\n//! embeds a checksummed Bloom filter for point-read rejection; levels and compaction remain later\n//! Phase 3 work.",
        1,
    )
    lib_path.write_text(lib)

path = Path("crates/db-storage-lsm/src/sstable.rs")
text = path.read_text()
if "LEGACY_FORMAT_VERSION" not in text:
    def replace_once(old: str, new: str) -> None:
        global text
        if old not in text:
            raise SystemExit(f"missing expected block:\n{old[:240]}")
        text = text.replace(old, new, 1)

    replace_once(
        "use crate::memtable::VersionedEntry;",
        "use crate::bloom::BloomFilter;\nuse crate::memtable::VersionedEntry;",
    )
    replace_once(
        "const FORMAT_VERSION: u16 = 1;",
        "const LEGACY_FORMAT_VERSION: u16 = 1;\nconst FORMAT_VERSION: u16 = 2;",
    )
    replace_once(
        "    index: Vec<IndexEntry>,\n}",
        "    index: Vec<IndexEntry>,\n    bloom: Option<BloomFilter>,\n}",
    )

    start = text.index("    pub(super) fn create_new(")
    end = text.index("    pub(super) fn open(", start)
    create_impl = '''    pub(super) fn create_new(\n        directory: &Path,\n        table_id: u64,\n        durable_sequence: u64,\n        entries: &BTreeMap<Vec<u8>, VersionedEntry>,\n    ) -> Result<Self> {\n        Self::create_new_with_format(\n            directory,\n            table_id,\n            durable_sequence,\n            entries,\n            FORMAT_VERSION,\n        )\n    }\n\n    #[cfg(test)]\n    pub(super) fn create_legacy_v1_for_test(\n        directory: &Path,\n        table_id: u64,\n        durable_sequence: u64,\n        entries: &BTreeMap<Vec<u8>, VersionedEntry>,\n    ) -> Result<Self> {\n        Self::create_new_with_format(\n            directory,\n            table_id,\n            durable_sequence,\n            entries,\n            LEGACY_FORMAT_VERSION,\n        )\n    }\n\n    fn create_new_with_format(\n        directory: &Path,\n        table_id: u64,\n        durable_sequence: u64,\n        entries: &BTreeMap<Vec<u8>, VersionedEntry>,\n        format_version: u16,\n    ) -> Result<Self> {\n        if entries.is_empty() {\n            return Err(corruption(0, "cannot create an empty SSTable"));\n        }\n        if format_version != LEGACY_FORMAT_VERSION && format_version != FORMAT_VERSION {\n            return Err(corruption(0, "cannot create unsupported SSTable format"));\n        }\n        let entry_count = u64::try_from(entries.len())\n            .map_err(|_| corruption(0, "SSTable entry count does not fit u64"))?;\n        let mut bytes = vec![0_u8; SSTABLE_HEADER_LEN];\n        if format_version == FORMAT_VERSION {\n            let bloom = BloomFilter::build(entries.keys().map(Vec::as_slice), entries.len())?;\n            bytes.extend_from_slice(&bloom.encode()?);\n        }\n        let data_offset = u64::try_from(bytes.len())\n            .map_err(|_| corruption(0, "SSTable data offset does not fit u64"))?;\n        let mut record_offsets = Vec::with_capacity(entries.len());\n\n        for (key, entry) in entries {\n            validate_entry_bounds(key, entry.value.as_deref())?;\n            let record_offset = u64::try_from(bytes.len())\n                .map_err(|_| corruption(0, "SSTable record offset does not fit u64"))?;\n            record_offsets.push(record_offset);\n            bytes.extend_from_slice(&encode_record(key, entry)?);\n        }\n\n        let index_offset = u64::try_from(bytes.len())\n            .map_err(|_| corruption(0, "SSTable index offset does not fit u64"))?;\n        for ((key, entry), record_offset) in entries.iter().zip(record_offsets) {\n            bytes.extend_from_slice(&encode_index_entry(key, entry, record_offset)?);\n        }\n        let footer_offset = u64::try_from(bytes.len())\n            .map_err(|_| corruption(0, "SSTable footer offset does not fit u64"))?;\n        bytes.resize(\n            bytes\n                .len()\n                .checked_add(SSTABLE_FOOTER_LEN)\n                .ok_or_else(|| corruption(0, "SSTable size overflowed usize"))?,\n            0,\n        );\n\n        let header = encode_header(\n            format_version,\n            table_id,\n            entry_count,\n            data_offset,\n            index_offset,\n            footer_offset,\n        );\n        bytes[..SSTABLE_HEADER_LEN].copy_from_slice(&header);\n        let whole_crc = crc32fast::hash(&bytes[..usize_from_u64(footer_offset, 0)?]);\n        let footer = encode_footer(\n            format_version,\n            table_id,\n            entry_count,\n            index_offset,\n            footer_offset,\n            durable_sequence,\n            whole_crc,\n        );\n        let footer_start = usize_from_u64(footer_offset, 0)?;\n        bytes[footer_start..footer_start + SSTABLE_FOOTER_LEN].copy_from_slice(&footer);\n\n        let path = directory.join(file_name(table_id));\n        let mut file = OpenOptions::new()\n            .write(true)\n            .create_new(true)\n            .open(&path)?;\n        file.write_all(&bytes)?;\n        file.sync_all()?;\n\n        let descriptor = SstableDescriptor {\n            table_id,\n            file_bytes: u64::try_from(bytes.len())\n                .map_err(|_| corruption(0, "SSTable file size does not fit u64"))?,\n            entry_count,\n            durable_sequence,\n            smallest_key: entries\n                .first_key_value()\n                .expect("nonempty checked above")\n                .0\n                .clone(),\n            largest_key: entries\n                .last_key_value()\n                .expect("nonempty checked above")\n                .0\n                .clone(),\n        };\n        Self::open(&path, descriptor)\n    }\n\n'''
    text = text[:start] + create_impl + text[end:]

    replace_once(
        '''        let footer = parse_footer(&bytes[footer_start..footer_end], header.footer_offset)?;\n        if footer.table_id != header.table_id\n            || footer.entry_count != header.entry_count''',
        '''        let footer = parse_footer(&bytes[footer_start..footer_end], header.footer_offset)?;\n        if footer.format_version != header.format_version\n            || footer.table_id != header.table_id\n            || footer.entry_count != header.entry_count''',
    )
    replace_once(
        '''        let data_start = usize_from_u64(header.data_offset, 0)?;\n        let index_start = usize_from_u64(header.index_offset, 0)?;\n        if data_start != SSTABLE_HEADER_LEN\n            || index_start > footer_start\n            || index_start < data_start\n        {\n            return Err(corruption(0, "invalid SSTable data/index extent ordering"));\n        }\n\n        let expected_count = usize_from_u64(header.entry_count, 0)?;''',
        '''        let data_start = usize_from_u64(header.data_offset, 0)?;\n        let index_start = usize_from_u64(header.index_offset, 0)?;\n        if index_start > footer_start || index_start < data_start {\n            return Err(corruption(0, "invalid SSTable data/index extent ordering"));\n        }\n        let bloom = match header.format_version {\n            LEGACY_FORMAT_VERSION => {\n                if data_start != SSTABLE_HEADER_LEN {\n                    return Err(corruption(\n                        32,\n                        "legacy SSTable v1 data must begin immediately after the header",\n                    ));\n                }\n                None\n            }\n            FORMAT_VERSION => {\n                if data_start <= SSTABLE_HEADER_LEN {\n                    return Err(corruption(32, "SSTable v2 is missing its Bloom section"));\n                }\n                Some(BloomFilter::decode(\n                    &bytes[SSTABLE_HEADER_LEN..data_start],\n                    SSTABLE_HEADER_LEN as u64,\n                    header.entry_count,\n                )?)\n            }\n            _ => unreachable!("header version validated before extent parsing"),\n        };\n\n        let expected_count = usize_from_u64(header.entry_count, 0)?;''',
    )
    replace_once(
        '''        if index.iter().map(|entry| entry.sequence).max().unwrap_or(0) > descriptor.durable_sequence\n        {\n            return Err(corruption(\n                0,\n                "SSTable entry sequence exceeds durable watermark",\n            ));\n        }\n\n        Ok(Self {\n            path: path.to_path_buf(),\n            descriptor,\n            bytes,\n            index,\n        })''',
        '''        if index.iter().map(|entry| entry.sequence).max().unwrap_or(0) > descriptor.durable_sequence\n        {\n            return Err(corruption(\n                0,\n                "SSTable entry sequence exceeds durable watermark",\n            ));\n        }\n        if bloom\n            .as_ref()\n            .is_some_and(|filter| index.iter().any(|entry| !filter.may_contain(&entry.key)))\n        {\n            return Err(corruption(\n                SSTABLE_HEADER_LEN as u64,\n                "SSTable Bloom filter has a false negative for an indexed key",\n            ));\n        }\n\n        Ok(Self {\n            path: path.to_path_buf(),\n            descriptor,\n            bytes,\n            index,\n            bloom,\n        })''',
    )
    replace_once(
        '''    pub(super) fn get(&self, key: &[u8]) -> Result<Option<VersionedEntry>> {\n        let index = match self''',
        '''    pub(super) fn get(&self, key: &[u8]) -> Result<Option<VersionedEntry>> {\n        if key < self.descriptor.smallest_key.as_slice()\n            || key > self.descriptor.largest_key.as_slice()\n            || self\n                .bloom\n                .as_ref()\n                .is_some_and(|filter| !filter.may_contain(key))\n        {\n            return Ok(None);\n        }\n        let index = match self''',
    )
    replace_once(
        '''    #[allow(dead_code)]\n    pub(super) fn path(&self) -> &Path {\n        &self.path\n    }\n}''',
        '''    #[cfg(test)]\n    pub(super) fn bloom_may_contain(&self, key: &[u8]) -> Option<bool> {\n        self.bloom.as_ref().map(|filter| filter.may_contain(key))\n    }\n\n    #[cfg(test)]\n    pub(super) fn format_version(&self) -> u16 {\n        if self.bloom.is_some() {\n            FORMAT_VERSION\n        } else {\n            LEGACY_FORMAT_VERSION\n        }\n    }\n\n    #[allow(dead_code)]\n    pub(super) fn path(&self) -> &Path {\n        &self.path\n    }\n}''',
    )
    replace_once(
        '''struct Header {\n    table_id: u64,''',
        '''struct Header {\n    format_version: u16,\n    table_id: u64,''',
    )
    replace_once(
        '''struct Footer {\n    table_id: u64,''',
        '''struct Footer {\n    format_version: u16,\n    table_id: u64,''',
    )
    replace_once(
        '''fn encode_header(\n    table_id: u64,\n    entry_count: u64,\n    index_offset: u64,\n    footer_offset: u64,\n) -> [u8; 64] {''',
        '''fn encode_header(\n    format_version: u16,\n    table_id: u64,\n    entry_count: u64,\n    data_offset: u64,\n    index_offset: u64,\n    footer_offset: u64,\n) -> [u8; 64] {''',
    )
    replace_once(
        '''    header[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());''',
        '''    header[8..10].copy_from_slice(&format_version.to_le_bytes());''',
    )
    replace_once(
        '''    header[32..40].copy_from_slice(&(SSTABLE_HEADER_LEN as u64).to_le_bytes());''',
        '''    header[32..40].copy_from_slice(&data_offset.to_le_bytes());''',
    )
    replace_once(
        '''    if version != FORMAT_VERSION {\n        return Err(DbError::UnsupportedVersion {\n            format: "LSM SSTable",\n            found: u64::from(version),\n            supported: u64::from(FORMAT_VERSION),\n        });\n    }''',
        '''    if version != LEGACY_FORMAT_VERSION && version != FORMAT_VERSION {\n        return Err(DbError::UnsupportedVersion {\n            format: "LSM SSTable",\n            found: u64::from(version),\n            supported: u64::from(FORMAT_VERSION),\n        });\n    }''',
    )
    replace_once(
        '''    Ok(Header {\n        table_id:''',
        '''    Ok(Header {\n        format_version: version,\n        table_id:''',
    )
    replace_once(
        '''fn encode_footer(\n    table_id: u64,''',
        '''fn encode_footer(\n    format_version: u16,\n    table_id: u64,''',
    )
    replace_once(
        '''    footer[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());''',
        '''    footer[8..10].copy_from_slice(&format_version.to_le_bytes());''',
    )
    # Replace the second identical version-validation block (footer) after the header block changed above.
    footer_old = '''    if version != FORMAT_VERSION {\n        return Err(DbError::UnsupportedVersion {\n            format: "LSM SSTable",\n            found: u64::from(version),\n            supported: u64::from(FORMAT_VERSION),\n        });\n    }'''
    if footer_old not in text:
        raise SystemExit("missing footer version validation block")
    text = text.replace(
        footer_old,
        '''    if version != LEGACY_FORMAT_VERSION && version != FORMAT_VERSION {\n        return Err(DbError::UnsupportedVersion {\n            format: "LSM SSTable",\n            found: u64::from(version),\n            supported: u64::from(FORMAT_VERSION),\n        });\n    }''',
        1,
    )
    replace_once(
        '''    Ok(Footer {\n        table_id:''',
        '''    Ok(Footer {\n        format_version: version,\n        table_id:''',
    )

    path.write_text(text)

# Add focused SSTable integration tests once.
tests_path = Path("crates/db-storage-lsm/src/sstable_tests.rs")
tests = tests_path.read_text()
if "sstable_v2_embeds_bloom" not in tests:
    tests = tests.replace(
        "use std::fs::{self, OpenOptions};",
        "use std::collections::BTreeMap;\nuse std::fs::{self, OpenOptions};",
        1,
    )
    tests = tests.replace(
        "use super::manifest::{CURRENT_FILE_NAME, CURRENT_SLOT_BYTES};\nuse super::{LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT};",
        "use super::manifest::{CURRENT_FILE_NAME, CURRENT_SLOT_BYTES};\nuse super::memtable::VersionedEntry;\nuse super::sstable::SsTable;\nuse super::{LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT};",
        1,
    )
    tests += r'''

#[test]
fn sstable_v2_embeds_bloom_without_false_negatives_for_values_or_tombstones() {
    let directory = tempdir().expect("temporary directory");
    let mut entries = BTreeMap::new();
    for sequence in 1_u64..=512 {
        let key = format!("key-{sequence:04}").into_bytes();
        let value = (sequence % 7 != 0).then(|| sequence.to_le_bytes().to_vec());
        entries.insert(key, VersionedEntry { sequence, value });
    }
    let table = SsTable::create_new(directory.path(), 1, 512, &entries).expect("create SSTable v2");
    assert_eq!(table.format_version(), 2);
    for key in entries.keys() {
        assert_eq!(table.bloom_may_contain(key), Some(true));
        assert_eq!(table.get(key).expect("point read"), entries.get(key).cloned());
    }

    let absent = (0_u64..10_000)
        .map(|value| format!("absent-{value:05}").into_bytes())
        .find(|key| table.bloom_may_contain(key) == Some(false))
        .expect("Bloom filter must reject at least one deterministic absent key");
    assert_eq!(table.get(&absent).expect("Bloom-negative read"), None);
}

#[test]
fn legacy_sstable_v1_remains_readable_without_a_filter() {
    let directory = tempdir().expect("temporary directory");
    let mut entries = BTreeMap::new();
    entries.insert(
        b"alpha".to_vec(),
        VersionedEntry {
            sequence: 1,
            value: Some(b"one".to_vec()),
        },
    );
    entries.insert(
        b"tombstone".to_vec(),
        VersionedEntry {
            sequence: 2,
            value: None,
        },
    );
    let table = SsTable::create_legacy_v1_for_test(directory.path(), 7, 2, &entries)
        .expect("create/read legacy SSTable v1");
    assert_eq!(table.format_version(), 1);
    assert_eq!(table.bloom_may_contain(b"alpha"), None);
    assert_eq!(
        table.get(b"alpha").expect("legacy point read"),
        entries.get(b"alpha".as_slice()).cloned()
    );
    assert_eq!(
        table.get(b"tombstone").expect("legacy tombstone read"),
        entries.get(b"tombstone".as_slice()).cloned()
    );
}
'''
    tests_path.write_text(tests)
