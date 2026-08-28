use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use db_core::{DbError, KvEngine, MAX_KEY_BYTES};
use tempfile::{tempdir, TempDir};

use super::manifest::{CURRENT_FILE_NAME, CURRENT_SLOT_BYTES};
use super::{CompactionFaultMode, CompactionWriteKind, LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT};

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

fn rewrite_v4_table_id_high_watermark(path: &Path, high_watermark: u64) {
    let manifest_id = current_manifest_id(path);
    let manifest_path = path.join(format!("MANIFEST-{manifest_id:016}"));
    let mut bytes = fs::read(&manifest_path).expect("read v4 manifest for corruption fixture");
    assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 4);
    bytes[64..72].copy_from_slice(&high_watermark.to_le_bytes());
    let header_crc = crc32fast::hash(&bytes[..76]);
    bytes[76..80].copy_from_slice(&header_crc.to_le_bytes());
    let file_crc_offset = bytes.len() - 4;
    let file_crc = crc32fast::hash(&bytes[..file_crc_offset]);
    bytes[file_crc_offset..].copy_from_slice(&file_crc.to_le_bytes());
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(manifest_path)
        .expect("open v4 corruption fixture");
    file.write_all(&bytes).expect("write v4 corruption fixture");
    file.sync_all().expect("sync v4 corruption fixture");
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
        engine
            .put(b"legacy-v3-a", &large_value(0x21))
            .expect("put a");
        engine
            .put(b"legacy-v3-b", &large_value(0x22))
            .expect("flush a/b");
        assert_eq!(engine.stats().expect("source stats").level0_sstables, 1);
    }
    rewrite_current_manifest_as_v3(&path);

    let mut reopened = LsmEngine::open(&path).expect("open v3 manifest");
    assert_eq!(
        reopened.get(b"legacy-v3-a").expect("get legacy a"),
        Some(large_value(0x21))
    );
    reopened
        .put(b"new-a", &large_value(0x23))
        .expect("put new a");
    reopened
        .put(b"new-b", &large_value(0x24))
        .expect("publish v4 state");
    reopened.reopen().expect("reopen v4 upgrade");
    let manifest_id = current_manifest_id(&path);
    let bytes =
        fs::read(path.join(format!("MANIFEST-{manifest_id:016}"))).expect("read v4 manifest");
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
        assert_eq!(
            engine.get(&tombstone_key(index)).expect("get elided key"),
            None
        );
        assert_eq!(
            engine
                .current_entry(&tombstone_key(index))
                .expect("entry lookup"),
            None,
            "tombstone must be physically absent after full-set compaction"
        );
    }

    let manifest_id = current_manifest_id(&path);
    let manifest =
        fs::read(path.join(format!("MANIFEST-{manifest_id:016}"))).expect("read empty manifest");
    assert_eq!(u16::from_le_bytes(manifest[8..10].try_into().unwrap()), 4);
    assert_eq!(u64::from_le_bytes(manifest[24..32].try_into().unwrap()), 64);
    assert_eq!(u64::from_le_bytes(manifest[32..40].try_into().unwrap()), 0);
    let high_watermark = u64::from_le_bytes(manifest[64..72].try_into().unwrap());
    assert_eq!(
        high_watermark, 4,
        "four flushed SSTable ids must stay reserved"
    );

    engine.reopen().expect("reopen durable-empty checkpoint");
    assert_eq!(engine.stats().expect("reopened empty stats"), empty);
    assert_eq!(
        LsmEngine::verify(&path)
            .expect("verify empty checkpoint")
            .memtables,
        empty
    );

    engine
        .put(b"after-empty", b"alive")
        .expect("put after empty checkpoint");
    engine
        .put(b"fill-a", &large_value(0x51))
        .expect("put fill a");
    engine
        .put(b"fill-b", &large_value(0x52))
        .expect("flush after empty checkpoint");
    let after = engine.stats().expect("post-empty flush stats");
    assert_eq!(after.level0_sstables, 1);
    assert_eq!(after.level1_sstables, 0);
    assert_eq!(after.sstable_entries, 3);
    engine.reopen().expect("reopen post-empty flush");
    assert_eq!(
        engine.get(b"after-empty").expect("get post-empty value"),
        Some(b"alive".to_vec())
    );
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
                assert_eq!(
                    engine
                        .delete(&tombstone_key(index))
                        .expect("pre-trigger delete"),
                    None
                );
            }
            let error = engine
                .delete(&tombstone_key(63))
                .expect_err("last tombstone must trigger injected compaction failure");
            assert!(
                matches!(error, DbError::Io(_)),
                "{kind:?} {mode:?}: {error}"
            );
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
                    reopened
                        .get(&tombstone_key(index))
                        .expect("read deleted key"),
                    None,
                    "{kind:?} {mode:?}"
                );
            }
            assert_eq!(
                LsmEngine::verify(&path)
                    .expect("verify fault state")
                    .memtables,
                stats,
                "{kind:?} {mode:?}"
            );
            case += 1;
        }
    }
}

#[test]
fn durable_empty_retry_preserves_observed_orphan_id_floor_across_cleanup() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    build_three_tombstone_l0(&path);

    {
        let mut engine = LsmEngine::open(&path).expect("open three-L0 baseline");
        engine.inject_compaction_fault_for_test(
            CompactionWriteKind::Manifest,
            CompactionFaultMode::BeforeWrite,
        );
        for index in 48_u64..63 {
            assert_eq!(
                engine
                    .delete(&tombstone_key(index))
                    .expect("pre-trigger delete"),
                None
            );
        }
        assert!(matches!(
            engine.delete(&tombstone_key(63)),
            Err(DbError::Io(_))
        ));
    }

    assert_eq!(canonical_count(&path, "sst-", ".sst"), 4);
    let orphan = path.join("sst-0000000000000099.sst");
    fs::write(&orphan, b"ambiguous crash orphan").expect("write canonical orphan id 99");

    let mut reopened = LsmEngine::open(&path).expect("open four-L0 state plus orphan 99");
    reopened
        .put(b"tail", b"v")
        .expect("retry empty compaction while retaining a WAL tail");
    let checkpoint = reopened.stats().expect("retry checkpoint stats");
    assert_eq!(checkpoint.durable_sequence, 64);
    assert_eq!(checkpoint.sstables, 0);
    assert_eq!(checkpoint.mutable_entries, 1);
    assert_eq!(canonical_count(&path, "sst-", ".sst"), 0);
    assert!(
        !orphan.exists(),
        "cleanup may remove orphan 99 only after persisting its id floor"
    );

    reopened.reopen().expect("reopen after orphan cleanup");
    reopened
        .put(b"fill-a", &large_value(0x71))
        .expect("put first post-checkpoint filler");
    reopened
        .put(b"fill-b", &large_value(0x72))
        .expect("flush post-checkpoint table");
    assert!(
        path.join("sst-0000000000000100.sst").exists(),
        "table allocation must continue above the removed ambiguous orphan id"
    );
    assert_eq!(
        reopened.get(b"tail").expect("read WAL-tail value"),
        Some(b"v".to_vec())
    );
    reopened.reopen().expect("reopen table 100");
    assert_eq!(
        reopened.get(b"tail").expect("read persisted tail"),
        Some(b"v".to_vec())
    );
}

#[test]
fn v4_rejects_table_id_high_watermark_below_active_descriptor() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create engine");
        engine.put(b"a", &large_value(0x31)).expect("put a");
        engine.put(b"b", &large_value(0x32)).expect("flush one L0");
        assert_eq!(engine.stats().expect("one-L0 stats").sstables, 1);
    }
    rewrite_v4_table_id_high_watermark(&path, 0);
    let error =
        LsmEngine::open(&path).expect_err("high watermark below active table must fail closed");
    assert!(
        error
            .to_string()
            .contains("table-id high watermark is below an active descriptor id"),
        "unexpected error: {error}"
    );
}

#[test]
fn v4_rejects_durable_empty_checkpoint_without_id_history() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create engine");
        delete_tombstone_range(&mut engine, 0, 64);
        let stats = engine.stats().expect("empty checkpoint stats");
        assert_eq!(stats.sstables, 0);
        assert_eq!(stats.durable_sequence, 64);
    }
    rewrite_v4_table_id_high_watermark(&path, 0);
    let error =
        LsmEngine::open(&path).expect_err("durable-empty checkpoint without id floor must fail");
    assert!(
        error
            .to_string()
            .contains("durable-empty checkpoint requires a nonzero table-id high watermark"),
        "unexpected error: {error}"
    );
}
