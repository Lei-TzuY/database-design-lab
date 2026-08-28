use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use db_core::KvEngine;
use tempfile::tempdir;

use super::manifest::{CURRENT_FILE_NAME, CURRENT_SLOT_BYTES};
use super::{LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT};

fn large_value(byte: u8) -> Vec<u8> {
    vec![byte; MUTABLE_MEMTABLE_BYTES_LIMIT / 2 + 1_024]
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

fn populate_four_flushes(engine: &mut LsmEngine) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut expected = Vec::new();
    for index in 0_u8..8 {
        let key = format!("k-{index:02}").into_bytes();
        let value = large_value(0x20 + index);
        engine.put(&key, &value).expect("populate compaction input");
        expected.push((key, value));
    }
    expected
}

#[test]
fn four_overlapping_l0_flush_slots_compact_to_one_l1_and_reopen() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
    let expected = populate_four_flushes(&mut engine);

    let stats = engine.stats().expect("stats after compaction");
    assert_eq!(stats.sstables, 1);
    assert_eq!(stats.level0_sstables, 0);
    assert_eq!(stats.level1_sstables, 1);
    assert_eq!(stats.sstable_entries, 8);
    assert_eq!(stats.durable_sequence, 8);
    assert_eq!(stats.wal_records, 0);
    assert_eq!(canonical_count(&path, "sst-", ".sst"), 1);
    assert_eq!(canonical_count(&path, "MANIFEST-", ""), 1);

    for (key, value) in &expected {
        assert_eq!(
            engine.get(key).expect("get compacted key"),
            Some(value.clone())
        );
    }
    engine.reopen().expect("reopen compacted L1");
    assert_eq!(engine.stats().expect("stats after reopen"), stats);
    for (key, value) in &expected {
        assert_eq!(
            engine.get(key).expect("get reopened key"),
            Some(value.clone())
        );
    }
    let verified = LsmEngine::verify(&path).expect("verify compacted engine");
    assert_eq!(verified.memtables, stats);
}

#[test]
fn compaction_keeps_newest_tombstone_and_new_l0_can_override_l1() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");

    engine
        .put(b"victim", &large_value(0x31))
        .expect("put victim old value");
    engine
        .put(b"a-fill", &large_value(0x32))
        .expect("flush first L0");
    assert!(engine.delete(b"victim").expect("delete victim").is_some());
    engine.put(b"b-fill", &large_value(0x33)).expect("put b");
    engine
        .put(b"c-fill", &large_value(0x34))
        .expect("flush tombstone L0");
    engine.put(b"d-fill", &large_value(0x35)).expect("put d");
    engine
        .put(b"e-fill", &large_value(0x36))
        .expect("flush third L0");
    engine.put(b"f-fill", &large_value(0x37)).expect("put f");
    engine
        .put(b"g-fill", &large_value(0x38))
        .expect("flush fourth L0 and compact");

    let tombstone = engine
        .current_entry(b"victim")
        .expect("read compacted entry")
        .expect("tombstone must remain represented");
    assert_eq!(tombstone.sequence, 3);
    assert_eq!(tombstone.value, None);
    assert_eq!(engine.get(b"victim").expect("deleted victim"), None);
    assert_eq!(engine.stats().expect("L1 stats").level1_sstables, 1);
    engine.reopen().expect("reopen tombstone L1");
    assert_eq!(engine.get(b"victim").expect("reopened tombstone"), None);

    engine.put(b"victim", b"revived").expect("revive victim");
    engine.put(b"h-fill", &large_value(0x39)).expect("put h");
    engine
        .put(b"i-fill", &large_value(0x3a))
        .expect("flush new L0 over L1");
    let stats = engine.stats().expect("mixed-level stats");
    assert_eq!(stats.level0_sstables, 1);
    assert_eq!(stats.level1_sstables, 1);
    assert_eq!(
        engine.get(b"victim").expect("newest L0 wins"),
        Some(b"revived".to_vec())
    );
    engine.reopen().expect("reopen mixed levels");
    assert_eq!(
        engine.get(b"victim").expect("reopened newest L0"),
        Some(b"revived".to_vec())
    );
}

#[test]
fn compaction_moves_both_current_mirrors_before_obsolete_file_cleanup() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
    let expected = populate_four_flushes(&mut engine);
    drop(engine);

    assert_eq!(canonical_count(&path, "sst-", ".sst"), 1);
    assert_eq!(canonical_count(&path, "MANIFEST-", ""), 1);
    let current_path = path.join(CURRENT_FILE_NAME);
    let current = fs::read(&current_path).expect("read mirrored CURRENT");
    assert_eq!(current.len(), CURRENT_SLOT_BYTES * 2);
    let generation0 = u64::from_le_bytes(current[16..24].try_into().expect("slot0 generation"));
    let generation1 = u64::from_le_bytes(
        current[CURRENT_SLOT_BYTES + 16..CURRENT_SLOT_BYTES + 24]
            .try_into()
            .expect("slot1 generation"),
    );
    let manifest0 = u64::from_le_bytes(current[24..32].try_into().expect("slot0 manifest"));
    let manifest1 = u64::from_le_bytes(
        current[CURRENT_SLOT_BYTES + 24..CURRENT_SLOT_BYTES + 32]
            .try_into()
            .expect("slot1 manifest"),
    );
    assert_eq!(
        manifest0, manifest1,
        "both mirrors must name the cleanup-safe manifest"
    );
    assert_eq!(generation0.abs_diff(generation1), 1);

    let newer_slot = usize::from(generation1 > generation0);
    let corrupt_offset = newer_slot * CURRENT_SLOT_BYTES + 100;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&current_path)
        .expect("open CURRENT to tear newest mirror");
    file.seek(SeekFrom::Start(corrupt_offset as u64))
        .expect("seek CURRENT corruption");
    file.write_all(&[0x5a])
        .expect("corrupt newest CURRENT slot");
    file.sync_all().expect("sync CURRENT corruption");
    drop(file);

    let mut reopened = LsmEngine::open(&path).expect("older mirror must remain self-contained");
    let stats = reopened.stats().expect("fallback stats");
    assert_eq!(stats.level0_sstables, 0);
    assert_eq!(stats.level1_sstables, 1);
    assert_eq!(canonical_count(&path, "sst-", ".sst"), 1);
    assert_eq!(canonical_count(&path, "MANIFEST-", ""), 1);
    for (key, value) in expected {
        assert_eq!(reopened.get(&key).expect("fallback get"), Some(value));
    }
}
