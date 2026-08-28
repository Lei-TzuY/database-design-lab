from pathlib import Path


def replace_once(path: str, old: str, new: str, label: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        raise SystemExit(f"missing {label} in {path}: {old[:180]!r}")
    p.write_text(text.replace(old, new, 1))

# Manifest v4: preserve table-id allocation history even when full-set compaction produces no SSTable.
p = Path("crates/db-storage-lsm/src/manifest.rs")
text = p.read_text()
text = text.replace(
    "const MANIFEST_FORMAT_VERSION_V2: u16 = 2;\nconst MANIFEST_FORMAT_VERSION: u16 = 3;",
    "const MANIFEST_FORMAT_VERSION_V2: u16 = 2;\nconst MANIFEST_FORMAT_VERSION_V3: u16 = 3;\nconst MANIFEST_FORMAT_VERSION: u16 = 4;",
    1,
)
text = text.replace(
    "    pub(super) wal_first_sequence: u64,\n    pub(super) tables: Vec<SstableDescriptor>,",
    "    pub(super) wal_first_sequence: u64,\n    pub(super) table_id_high_watermark: u64,\n    pub(super) tables: Vec<SstableDescriptor>,",
    1,
)
text = text.replace(
    "        wal_id,\n        wal_first_sequence,\n        tables: Vec::new(),",
    "        wal_id,\n        wal_first_sequence,\n        table_id_high_watermark: 0,\n        tables: Vec::new(),",
    1,
)
old = '''    validate_version_set(durable_sequence, wal_id, wal_first_sequence, &tables)?;
    let next = VersionSet {
        current_generation: generation,
        manifest_id: new_manifest_id,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        tables,
    };
'''
new = '''    let table_id_high_watermark = tables.iter().fold(
        current.table_id_high_watermark,
        |high, descriptor| high.max(descriptor.table_id),
    );
    validate_version_set(
        MANIFEST_FORMAT_VERSION,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        table_id_high_watermark,
        &tables,
    )?;
    let next = VersionSet {
        current_generation: generation,
        manifest_id: new_manifest_id,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        table_id_high_watermark,
        tables,
    };
'''
if old not in text:
    raise SystemExit("missing prepare-install validation block")
text = text.replace(old, new, 1)
old = '''    validate_version_set(
        version.durable_sequence,
        version.wal_id,
        version.wal_first_sequence,
        &version.tables,
    )?;
'''
new = '''    validate_version_set(
        MANIFEST_FORMAT_VERSION,
        version.durable_sequence,
        version.wal_id,
        version.wal_first_sequence,
        version.table_id_high_watermark,
        &version.tables,
    )?;
'''
if old not in text:
    raise SystemExit("missing write-manifest validation block")
text = text.replace(old, new, 1)
old = '''    bytes[48..56].copy_from_slice(&version.wal_id.to_le_bytes());
    bytes[56..64].copy_from_slice(&version.wal_first_sequence.to_le_bytes());
    let header_crc = crc32fast::hash(&bytes[..76]);
'''
new = '''    bytes[48..56].copy_from_slice(&version.wal_id.to_le_bytes());
    bytes[56..64].copy_from_slice(&version.wal_first_sequence.to_le_bytes());
    bytes[64..72].copy_from_slice(&version.table_id_high_watermark.to_le_bytes());
    let header_crc = crc32fast::hash(&bytes[..76]);
'''
if old not in text:
    raise SystemExit("missing manifest header WAL fields")
text = text.replace(old, new, 1)
# Replace read header dispatch with v1/v2/v3 compatibility plus v4 high-watermark decoding.
old = '''    let (header_len, wal_id, wal_first_sequence) = match format_version {
        MANIFEST_FORMAT_VERSION_V1 => {
            if u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize
                != MANIFEST_HEADER_LEN_V1
                || bytes[12..16] != [0; 4]
                || bytes[48..60].iter().any(|byte| *byte != 0)
            {
                return Err(corruption(10, "invalid v1 manifest header fields"));
            }
            let expected_header_crc =
                u32::from_le_bytes(bytes[60..64].try_into().expect("fixed slice"));
            if crc32fast::hash(&bytes[..60]) != expected_header_crc {
                return Err(corruption(60, "manifest header checksum mismatch"));
            }
            (MANIFEST_HEADER_LEN_V1, 1, 1)
        }
        MANIFEST_FORMAT_VERSION_V2 | MANIFEST_FORMAT_VERSION => {
            if bytes.len() < MANIFEST_HEADER_LEN + 4
                || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize
                    != MANIFEST_HEADER_LEN
                || bytes[12..16] != [0; 4]
                || bytes[64..76].iter().any(|byte| *byte != 0)
            {
                return Err(corruption(10, "invalid v2/v3 manifest header fields"));
            }
            let expected_header_crc =
                u32::from_le_bytes(bytes[76..80].try_into().expect("fixed slice"));
            if crc32fast::hash(&bytes[..76]) != expected_header_crc {
                return Err(corruption(76, "manifest header checksum mismatch"));
            }
            (
                MANIFEST_HEADER_LEN,
                u64::from_le_bytes(bytes[48..56].try_into().expect("fixed slice")),
                u64::from_le_bytes(bytes[56..64].try_into().expect("fixed slice")),
            )
        }
        _ => {
            return Err(DbError::UnsupportedVersion {
                format: "LSM manifest",
                found: u64::from(format_version),
                supported: u64::from(MANIFEST_FORMAT_VERSION),
            });
        }
    };
'''
new = '''    let (header_len, wal_id, wal_first_sequence, encoded_table_id_high_watermark) =
        match format_version {
            MANIFEST_FORMAT_VERSION_V1 => {
                if u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize
                    != MANIFEST_HEADER_LEN_V1
                    || bytes[12..16] != [0; 4]
                    || bytes[48..60].iter().any(|byte| *byte != 0)
                {
                    return Err(corruption(10, "invalid v1 manifest header fields"));
                }
                let expected_header_crc =
                    u32::from_le_bytes(bytes[60..64].try_into().expect("fixed slice"));
                if crc32fast::hash(&bytes[..60]) != expected_header_crc {
                    return Err(corruption(60, "manifest header checksum mismatch"));
                }
                (MANIFEST_HEADER_LEN_V1, 1, 1, None)
            }
            MANIFEST_FORMAT_VERSION_V2 | MANIFEST_FORMAT_VERSION_V3 => {
                if bytes.len() < MANIFEST_HEADER_LEN + 4
                    || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize
                        != MANIFEST_HEADER_LEN
                    || bytes[12..16] != [0; 4]
                    || bytes[64..76].iter().any(|byte| *byte != 0)
                {
                    return Err(corruption(10, "invalid v2/v3 manifest header fields"));
                }
                let expected_header_crc =
                    u32::from_le_bytes(bytes[76..80].try_into().expect("fixed slice"));
                if crc32fast::hash(&bytes[..76]) != expected_header_crc {
                    return Err(corruption(76, "manifest header checksum mismatch"));
                }
                (
                    MANIFEST_HEADER_LEN,
                    u64::from_le_bytes(bytes[48..56].try_into().expect("fixed slice")),
                    u64::from_le_bytes(bytes[56..64].try_into().expect("fixed slice")),
                    None,
                )
            }
            MANIFEST_FORMAT_VERSION => {
                if bytes.len() < MANIFEST_HEADER_LEN + 4
                    || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) as usize
                        != MANIFEST_HEADER_LEN
                    || bytes[12..16] != [0; 4]
                    || bytes[72..76].iter().any(|byte| *byte != 0)
                {
                    return Err(corruption(10, "invalid v4 manifest header fields"));
                }
                let expected_header_crc =
                    u32::from_le_bytes(bytes[76..80].try_into().expect("fixed slice"));
                if crc32fast::hash(&bytes[..76]) != expected_header_crc {
                    return Err(corruption(76, "manifest header checksum mismatch"));
                }
                (
                    MANIFEST_HEADER_LEN,
                    u64::from_le_bytes(bytes[48..56].try_into().expect("fixed slice")),
                    u64::from_le_bytes(bytes[56..64].try_into().expect("fixed slice")),
                    Some(u64::from_le_bytes(
                        bytes[64..72].try_into().expect("fixed slice"),
                    )),
                )
            }
            _ => {
                return Err(DbError::UnsupportedVersion {
                    format: "LSM manifest",
                    found: u64::from(format_version),
                    supported: u64::from(MANIFEST_FORMAT_VERSION),
                });
            }
        };
'''
if old not in text:
    raise SystemExit("missing manifest read-version dispatch")
text = text.replace(old, new, 1)
old = '''    validate_version_set(durable_sequence, wal_id, wal_first_sequence, &tables)?;
    Ok(VersionSet {
        current_generation: 0,
        manifest_id,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        tables,
    })
'''
new = '''    let table_id_high_watermark = encoded_table_id_high_watermark.unwrap_or_else(|| {
        tables
            .iter()
            .map(|descriptor| descriptor.table_id)
            .max()
            .unwrap_or(0)
    });
    validate_version_set(
        format_version,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        table_id_high_watermark,
        &tables,
    )?;
    Ok(VersionSet {
        current_generation: 0,
        manifest_id,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        table_id_high_watermark,
        tables,
    })
'''
if old not in text:
    raise SystemExit("missing read manifest result")
text = text.replace(old, new, 1)
old = '''fn validate_version_set(
    durable_sequence: u64,
    wal_id: u64,
    wal_first_sequence: u64,
    tables: &[SstableDescriptor],
) -> Result<()> {
'''
new = '''fn validate_version_set(
    format_version: u16,
    durable_sequence: u64,
    wal_id: u64,
    wal_first_sequence: u64,
    table_id_high_watermark: u64,
    tables: &[SstableDescriptor],
) -> Result<()> {
'''
if old not in text:
    raise SystemExit("missing version-set validation signature")
text = text.replace(old, new, 1)
old = '''    let expected = tables.last().map_or(0, |table| table.durable_sequence);
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
'''
new = '''    let max_table_id = tables
        .iter()
        .map(|descriptor| descriptor.table_id)
        .max()
        .unwrap_or(0);
    if table_id_high_watermark < max_table_id {
        return Err(corruption(
            0,
            "manifest table-id high watermark is below an active descriptor id",
        ));
    }

    let expected = tables.last().map_or(0, |table| table.durable_sequence);
    if tables.is_empty() && format_version >= MANIFEST_FORMAT_VERSION {
        if durable_sequence > 0 && table_id_high_watermark == 0 {
            return Err(corruption(
                0,
                "durable-empty checkpoint requires a nonzero table-id high watermark",
            ));
        }
    } else if durable_sequence != expected {
        return Err(corruption(
            0,
            format!(
                "manifest durable sequence {durable_sequence} does not equal latest table watermark {expected}"
            ),
        ));
    }
    Ok(())
}
'''
if old not in text:
    raise SystemExit("missing version-set durable watermark tail")
text = text.replace(old, new, 1)
p.write_text(text)

# Engine: use the persisted high watermark on reopen and drop tombstones only during full-set compaction.
p = Path("crates/db-storage-lsm/src/lib.rs")
text = p.read_text()
text = text.replace(
    "                wal_first_sequence: INITIAL_FIRST_SEQUENCE,\n                tables: Vec::new(),",
    "                wal_first_sequence: INITIAL_FIRST_SEQUENCE,\n                table_id_high_watermark: 0,\n                tables: Vec::new(),",
    2,
)
old = '''            next_table_id: checked_next_id(layout.max_table_id, "SSTable")?,
            next_manifest_id: checked_next_id(layout.max_manifest_id, "manifest")?,
'''
new = '''            next_table_id: checked_next_id(
                layout.max_table_id.max(version.table_id_high_watermark),
                "SSTable",
            )?,
            next_manifest_id: checked_next_id(layout.max_manifest_id, "manifest")?,
'''
if old not in text:
    raise SystemExit("missing reopen next-table allocation")
text = text.replace(old, new, 1)
text = text.replace('name: "lsm-level1-compaction-v3",', 'name: "lsm-level1-compaction-v4",', 1)
old = '''        if merged.is_empty() {
            return Err(corruption(
                "full-set compaction unexpectedly produced no entries",
            ));
        }

        let table_id = self.next_table_id;
        let manifest_id = self.next_manifest_id;
        let next_table_id = checked_next_id(table_id, "SSTable")?;
        let next_manifest_id = checked_next_id(manifest_id, "manifest")?;
        let durable_sequence = self.version.durable_sequence;
        #[cfg(test)]
        self.compaction_before_write_for_test(CompactionWriteKind::L1Sstable)?;
        let table =
            SsTable::create_new_at_level(&self.path, table_id, 1, durable_sequence, &merged)?;
        #[cfg(test)]
        self.compaction_after_file_write_for_test(
            CompactionWriteKind::L1Sstable,
            &self.path.join(sstable_file_name(table_id)),
        )?;

        #[cfg(test)]
        self.compaction_before_write_for_test(CompactionWriteKind::Manifest)?;
        let compacted = manifest::prepare_install(
            &self.path,
            &self.version,
            manifest_id,
            durable_sequence,
            vec![table.descriptor().clone()],
            self.version.wal_id,
            self.version.wal_first_sequence,
        )?;
'''
new = '''        merged.retain(|_, entry| entry.value.is_some());

        let table_id = self.next_table_id;
        let manifest_id = self.next_manifest_id;
        let next_manifest_id = checked_next_id(manifest_id, "manifest")?;
        let durable_sequence = self.version.durable_sequence;
        let (replacement_table, descriptors, next_table_id) = if merged.is_empty() {
            (None, Vec::new(), table_id)
        } else {
            #[cfg(test)]
            self.compaction_before_write_for_test(CompactionWriteKind::L1Sstable)?;
            let table =
                SsTable::create_new_at_level(&self.path, table_id, 1, durable_sequence, &merged)?;
            #[cfg(test)]
            self.compaction_after_file_write_for_test(
                CompactionWriteKind::L1Sstable,
                &self.path.join(sstable_file_name(table_id)),
            )?;
            let descriptor = table.descriptor().clone();
            (
                Some(table),
                vec![descriptor],
                checked_next_id(table_id, "SSTable")?,
            )
        };

        #[cfg(test)]
        self.compaction_before_write_for_test(CompactionWriteKind::Manifest)?;
        let compacted = manifest::prepare_install(
            &self.path,
            &self.version,
            manifest_id,
            durable_sequence,
            descriptors,
            self.version.wal_id,
            self.version.wal_first_sequence,
        )?;
'''
if old not in text:
    raise SystemExit("missing compaction output preparation block")
text = text.replace(old, new, 1)
old = '''        let active_table_id = table.descriptor().table_id;
        let active_manifest_id = mirrored.manifest_id;

        let old_tables = std::mem::replace(&mut self.tables, vec![table]);
        self.version = mirrored;
        self.next_table_id = next_table_id;
        self.next_manifest_id = next_manifest_id;
        drop(old_tables);
        self.reclaim_obsolete_sstables(active_table_id);
'''
new = '''        let active_table_id = replacement_table
            .as_ref()
            .map(|table| table.descriptor().table_id);
        let active_manifest_id = mirrored.manifest_id;

        let replacement_tables = replacement_table.into_iter().collect();
        let old_tables = std::mem::replace(&mut self.tables, replacement_tables);
        self.version = mirrored;
        self.next_table_id = next_table_id;
        self.next_manifest_id = next_manifest_id;
        drop(old_tables);
        self.reclaim_obsolete_sstables(active_table_id);
'''
if old not in text:
    raise SystemExit("missing compaction installation tail")
text = text.replace(old, new, 1)
old = '''    fn reclaim_obsolete_sstables(&self, active_table_id: u64) {
'''
new = '''    fn reclaim_obsolete_sstables(&self, active_table_id: Option<u64>) {
'''
if old not in text:
    raise SystemExit("missing SSTable reclaim signature")
text = text.replace(old, new, 1)
text = text.replace(
    "            if table_id != active_table_id {\n                let _ = fs::remove_file(entry.path());\n            }",
    "            if active_table_id != Some(table_id) {\n                let _ = fs::remove_file(entry.path());\n            }",
    1,
)
text = text.replace(
    "#[cfg(test)]\nmod compaction_tests;",
    "#[cfg(test)]\nmod compaction_tests;\n#[cfg(test)]\nmod tombstone_elision_tests;",
    1,
)
p.write_text(text)

# Align existing v3-specific regressions with the new writer and new tombstone semantics.
p = Path("crates/db-storage-lsm/src/compaction_tests.rs")
text = p.read_text()
text = text.replace("rewrite_single_table_manifest_as_v2", "rewrite_single_table_manifest_as_v2", 1)
text = text.replace('expect("read v3 manifest fixture")', 'expect("read v4 manifest fixture")', 1)
text = text.replace(
    'u16::from_le_bytes(source[8..10].try_into().expect("manifest version")),\n        3',
    'u16::from_le_bytes(source[8..10].try_into().expect("manifest version")),\n        4',
    1,
)
text = text.replace('"only an L0 v3 descriptor can be represented by legacy Manifest v2"', '"only an L0 v4 descriptor can be represented by legacy Manifest v2"', 1)
text = text.replace("let upgraded = reopened.stats().expect(\"v3 upgraded stats\");", "let upgraded = reopened.stats().expect(\"v4 upgraded stats\");", 1)
text = text.replace("reopen upgraded v3 L1", "reopen upgraded v4 L1", 1)
text = text.replace("reopened v3 stats", "reopened v4 stats", 1)
text = text.replace("fn manifest_v2_descriptor_reopens_as_l0_and_upgrades_through_v3_compaction()", "fn manifest_v2_descriptor_reopens_as_l0_and_upgrades_through_v4_compaction()", 1)
text = text.replace("let stats = engine.stats().expect(\"v3 source stats\");", "let stats = engine.stats().expect(\"v4 source stats\");", 1)
old = '''    let tombstone = engine
        .current_entry(b"victim")
        .expect("read compacted entry")
        .expect("tombstone must remain represented");
    assert_eq!(tombstone.sequence, 3);
    assert_eq!(tombstone.value, None);
    assert_eq!(engine.get(b"victim").expect("deleted victim"), None);
    assert_eq!(engine.stats().expect("L1 stats").level1_sstables, 1);
'''
new = '''    assert_eq!(
        engine.current_entry(b"victim").expect("read compacted entry"),
        None,
        "full-set compaction must physically elide the obsolete tombstone"
    );
    assert_eq!(engine.get(b"victim").expect("deleted victim"), None);
    let compacted = engine.stats().expect("L1 stats");
    assert_eq!(compacted.level1_sstables, 1);
    assert_eq!(compacted.sstable_entries, 7);
'''
if old not in text:
    raise SystemExit("missing retained-tombstone regression block")
text = text.replace(old, new, 1)
text = text.replace(
    "fn compaction_keeps_newest_tombstone_and_new_l0_can_override_l1()",
    "fn compaction_elides_safe_tombstone_and_new_l0_can_override_l1()",
    1,
)
p.write_text(text)

p = Path("crates/db-storage-lsm/src/wal_rotation_tests.rs")
text = p.read_text()
old = '''    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("version")),
        3
    );
'''
new = '''    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("version")),
        4
    );
'''
if old not in text:
    raise SystemExit("missing WAL rotation manifest version assertion")
p.write_text(text.replace(old, new, 1))

# New proof suite: v3 compatibility, mixed tombstone elision, durable-empty checkpoint, id watermark,
# and empty-output crash-state matrix.
Path("crates/db-storage-lsm/src/tombstone_elision_tests.rs").write_text(r'''use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use db_core::{DbError, KvEngine, MAX_KEY_BYTES};
use tempfile::{tempdir, TempDir};

use super::manifest::{CURRENT_FILE_NAME, CURRENT_SLOT_BYTES};
use super::{
    CompactionFaultMode, CompactionWriteKind, LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT,
};

fn large_value(byte: u8) -> Vec<u8> {
    vec![byte; MUTABLE_MEMTABLE_BYTES_LIMIT / 2 + 1_024]
}

fn tombstone_key(index: u64) -> Vec<u8> {
    let mut key = vec![0x7f; MAX_KEY_BYTES];
    key[..8].copy_from_slice(&index.to_be_bytes());
    key
}

fn canonical_count(path: &Path, prefix: &str, suffix: &str) -> usize {
    fs::read_dir(path)
        .expect("read engine directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(prefix) && name.ends_with(suffix)
        })
        .count()
}

fn current_manifest_id(path: &Path) -> u64 {
    let current = fs::read(path.join(CURRENT_FILE_NAME)).expect("read CURRENT");
    assert_eq!(current.len(), CURRENT_SLOT_BYTES * 2);
    let mut valid = Vec::new();
    for slot in 0..2 {
        let base = slot * CURRENT_SLOT_BYTES;
        let generation = u64::from_le_bytes(
            current[base + 16..base + 24]
                .try_into()
                .expect("CURRENT generation"),
        );
        let manifest_id = u64::from_le_bytes(
            current[base + 24..base + 32]
                .try_into()
                .expect("CURRENT manifest"),
        );
        valid.push((generation, manifest_id));
    }
    valid.into_iter().max().expect("CURRENT slot").1
}

fn rewrite_current_manifest_as_v3(path: &Path) {
    let manifest_id = current_manifest_id(path);
    let manifest_path = path.join(format!("MANIFEST-{manifest_id:016}"));
    let mut bytes = fs::read(&manifest_path).expect("read v4 manifest");
    assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 4);
    assert!(u64::from_le_bytes(bytes[64..72].try_into().unwrap()) > 0);
    bytes[8..10].copy_from_slice(&3_u16.to_le_bytes());
    bytes[64..76].fill(0);
    let header_crc = crc32fast::hash(&bytes[..76]);
    bytes[76..80].copy_from_slice(&header_crc.to_le_bytes());
    let file_crc_offset = bytes.len() - 4;
    let file_crc = crc32fast::hash(&bytes[..file_crc_offset]);
    bytes[file_crc_offset..].copy_from_slice(&file_crc.to_le_bytes());
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(manifest_path)
        .expect("open v3 fixture");
    file.write_all(&bytes).expect("write v3 fixture");
    file.sync_all().expect("sync v3 fixture");
}

fn delete_tombstone_range(engine: &mut LsmEngine, start: u64, count: u64) {
    for index in start..start + count {
        assert_eq!(
            engine
                .delete(&tombstone_key(index))
                .expect("append missing-key tombstone"),
            None
        );
    }
}

fn build_three_tombstone_l0(path: &Path) {
    let mut engine = LsmEngine::create_new(path).expect("create tombstone baseline");
    delete_tombstone_range(&mut engine, 0, 48);
    let stats = engine.stats().expect("three tombstone L0 stats");
    assert_eq!(stats.level0_sstables, 3);
    assert_eq!(stats.level1_sstables, 0);
    assert_eq!(stats.sstable_entries, 48);
    assert_eq!(stats.durable_sequence, 48);
}

fn clone_engine(source: &Path, directory: &TempDir, name: &str) -> PathBuf {
    let target = directory.path().join(name);
    fs::create_dir(&target).expect("create clone directory");
    for entry in fs::read_dir(source).expect("read baseline") {
        let entry = entry.expect("baseline entry");
        assert!(entry.file_type().expect("entry type").is_file());
        fs::copy(entry.path(), target.join(entry.file_name())).expect("copy baseline file");
    }
    target
}

fn expected_empty_checkpoint(kind: CompactionWriteKind, mode: CompactionFaultMode) -> bool {
    kind == CompactionWriteKind::MirrorCurrent
        || (kind == CompactionWriteKind::FirstCurrent && mode == CompactionFaultMode::AfterSync)
}

#[test]
fn manifest_v3_remains_readable_and_next_publication_upgrades_to_v4() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create engine");
        engine.put(b"legacy-v3-a", &large_value(0x21)).expect("put a");
        engine.put(b"legacy-v3-b", &large_value(0x22)).expect("flush a/b");
        assert_eq!(engine.stats().expect("source stats").level0_sstables, 1);
    }
    rewrite_current_manifest_as_v3(&path);

    let mut reopened = LsmEngine::open(&path).expect("open v3 manifest");
    assert_eq!(
        reopened.get(b"legacy-v3-a").expect("get legacy a"),
        Some(large_value(0x21))
    );
    reopened.put(b"new-a", &large_value(0x23)).expect("put new a");
    reopened.put(b"new-b", &large_value(0x24)).expect("publish v4 state");
    reopened.reopen().expect("reopen v4 upgrade");
    let manifest_id = current_manifest_id(&path);
    let bytes = fs::read(path.join(format!("MANIFEST-{manifest_id:016}"))).expect("read v4 manifest");
    assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 4);
    assert!(u64::from_le_bytes(bytes[64..72].try_into().unwrap()) >= 2);
}

#[test]
fn all_tombstones_compact_to_durable_empty_checkpoint_and_preserve_id_floor() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create engine");
    delete_tombstone_range(&mut engine, 0, 64);

    let empty = engine.stats().expect("empty checkpoint stats");
    assert_eq!(empty.durable_sequence, 64);
    assert_eq!(empty.sstables, 0);
    assert_eq!(empty.level0_sstables, 0);
    assert_eq!(empty.level1_sstables, 0);
    assert_eq!(empty.sstable_entries, 0);
    assert_eq!(empty.wal_records, 0);
    assert_eq!(canonical_count(&path, "sst-", ".sst"), 0);
    assert_eq!(canonical_count(&path, "MANIFEST-", ""), 1);
    assert_eq!(canonical_count(&path, "wal-", ".log"), 1);
    for index in [0_u64, 15, 31, 47, 63] {
        assert_eq!(engine.get(&tombstone_key(index)).expect("get elided key"), None);
        assert_eq!(
            engine.current_entry(&tombstone_key(index)).expect("entry lookup"),
            None,
            "tombstone must be physically absent after full-set compaction"
        );
    }

    let manifest_id = current_manifest_id(&path);
    let manifest = fs::read(path.join(format!("MANIFEST-{manifest_id:016}"))).expect("read empty manifest");
    assert_eq!(u16::from_le_bytes(manifest[8..10].try_into().unwrap()), 4);
    assert_eq!(u64::from_le_bytes(manifest[24..32].try_into().unwrap()), 64);
    assert_eq!(u64::from_le_bytes(manifest[32..40].try_into().unwrap()), 0);
    let high_watermark = u64::from_le_bytes(manifest[64..72].try_into().unwrap());
    assert_eq!(high_watermark, 4, "four flushed SSTable ids must stay reserved");

    engine.reopen().expect("reopen durable-empty checkpoint");
    assert_eq!(engine.stats().expect("reopened empty stats"), empty);
    assert_eq!(LsmEngine::verify(&path).expect("verify empty checkpoint").memtables, empty);

    engine.put(b"after-empty", b"alive").expect("put after empty checkpoint");
    engine.put(b"fill-a", &large_value(0x51)).expect("put fill a");
    engine.put(b"fill-b", &large_value(0x52)).expect("flush after empty checkpoint");
    let after = engine.stats().expect("post-empty flush stats");
    assert_eq!(after.level0_sstables, 1);
    assert_eq!(after.level1_sstables, 0);
    assert_eq!(after.sstable_entries, 3);
    engine.reopen().expect("reopen post-empty flush");
    assert_eq!(engine.get(b"after-empty").expect("get post-empty value"), Some(b"alive".to_vec()));
    assert!(
        path.join("sst-0000000000000005.sst").exists(),
        "first table after reopen must continue after the persisted id high watermark"
    );
}

#[test]
fn empty_output_compaction_trace_skips_l1_sstable_write() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    build_three_tombstone_l0(&path);
    let mut engine = LsmEngine::open(&path).expect("open baseline");
    engine.begin_compaction_fault_trace_for_test();
    delete_tombstone_range(&mut engine, 48, 16);
    assert_eq!(
        engine.compaction_fault_trace_for_test(),
        &[
            CompactionWriteKind::Manifest,
            CompactionWriteKind::FirstCurrent,
            CompactionWriteKind::MirrorCurrent,
        ]
    );
    assert_eq!(engine.stats().expect("empty compacted stats").sstables, 0);
}

#[test]
fn empty_output_compaction_fault_matrix_reopens_old_or_durable_empty_checkpoint() {
    let directory = tempdir().expect("temporary directory");
    let baseline = directory.path().join("baseline");
    build_three_tombstone_l0(&baseline);
    let kinds = [
        CompactionWriteKind::Manifest,
        CompactionWriteKind::FirstCurrent,
        CompactionWriteKind::MirrorCurrent,
    ];
    let modes = [
        CompactionFaultMode::BeforeWrite,
        CompactionFaultMode::TornWrite,
        CompactionFaultMode::AfterSync,
    ];

    let mut case = 0_usize;
    for kind in kinds {
        for mode in modes {
            let path = clone_engine(&baseline, &directory, &format!("fault-{case}"));
            let mut engine = LsmEngine::open(&path).expect("open fault fixture");
            engine.inject_compaction_fault_for_test(kind, mode);
            for index in 48_u64..63 {
                assert_eq!(engine.delete(&tombstone_key(index)).expect("pre-trigger delete"), None);
            }
            let error = engine
                .delete(&tombstone_key(63))
                .expect_err("last tombstone must trigger injected compaction failure");
            assert!(matches!(error, DbError::Io(_)), "{kind:?} {mode:?}: {error}");
            assert!(matches!(engine.get(b"anything"), Err(DbError::Poisoned)));
            drop(engine);

            let mut reopened = LsmEngine::open(&path).expect("reopen fault fixture");
            let stats = reopened.stats().expect("fault reopen stats");
            assert_eq!(stats.durable_sequence, 64, "{kind:?} {mode:?}");
            if expected_empty_checkpoint(kind, mode) {
                assert_eq!(stats.sstables, 0, "{kind:?} {mode:?}");
                assert_eq!(stats.sstable_entries, 0, "{kind:?} {mode:?}");
                assert_eq!(stats.level0_sstables, 0, "{kind:?} {mode:?}");
            } else {
                assert_eq!(stats.sstables, 4, "{kind:?} {mode:?}");
                assert_eq!(stats.sstable_entries, 64, "{kind:?} {mode:?}");
                assert_eq!(stats.level0_sstables, 4, "{kind:?} {mode:?}");
            }
            for index in [0_u64, 15, 31, 47, 63] {
                assert_eq!(
                    reopened.get(&tombstone_key(index)).expect("read deleted key"),
                    None,
                    "{kind:?} {mode:?}"
                );
            }
            assert_eq!(
                LsmEngine::verify(&path).expect("verify fault state").memtables,
                stats,
                "{kind:?} {mode:?}"
            );
            case += 1;
        }
    }
}
''')
