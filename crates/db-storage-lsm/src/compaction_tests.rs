use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use db_core::{KvEngine, MAX_KEY_BYTES};
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

fn only_manifest(path: &Path) -> PathBuf {
    let mut manifests = fs::read_dir(path)
        .expect("read engine directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("MANIFEST-"));
    let manifest = manifests.next().expect("one authoritative manifest").path();
    assert!(
        manifests.next().is_none(),
        "obsolete manifests were reclaimed"
    );
    manifest
}

fn rewrite_manifest_checksums(bytes: &mut [u8]) {
    let header_crc = crc32fast::hash(&bytes[..76]);
    bytes[76..80].copy_from_slice(&header_crc.to_le_bytes());
    let file_crc_offset = bytes.len() - 4;
    let file_crc = crc32fast::hash(&bytes[..file_crc_offset]);
    bytes[file_crc_offset..].copy_from_slice(&file_crc.to_le_bytes());
}

fn rewrite_manifest_as_v3(path: &Path) {
    let manifest_path = only_manifest(path);
    let mut bytes = fs::read(&manifest_path).expect("read v4 manifest fixture");
    assert_eq!(
        u16::from_le_bytes(bytes[8..10].try_into().expect("manifest version")),
        4
    );
    assert_eq!(bytes[64..72], [0; 8]);
    bytes[8..10].copy_from_slice(&3_u16.to_le_bytes());
    rewrite_manifest_checksums(&mut bytes);
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(manifest_path)
        .expect("open manifest fixture for downgrade");
    file.write_all(&bytes).expect("write Manifest v3 fixture");
    file.sync_all().expect("sync Manifest v3 fixture");
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

fn rewrite_single_table_manifest_as_v2(path: &Path, manifest_id: u64) {
    let manifest_path = path.join(format!("MANIFEST-{manifest_id:016}"));
    let source = fs::read(&manifest_path).expect("read v4 manifest fixture");
    assert_eq!(
        u16::from_le_bytes(source[8..10].try_into().expect("manifest version")),
        4
    );
    assert_eq!(
        u64::from_le_bytes(source[32..40].try_into().expect("table count")),
        1
    );
    assert_eq!(
        u32::from_le_bytes(source[112..116].try_into().expect("v4 level")),
        0,
        "only an L0 v4 descriptor can be represented by legacy Manifest v2"
    );
    assert_eq!(source[116..120], [0; 4]);

    let smallest_len = usize::try_from(u32::from_le_bytes(
        source[120..124].try_into().expect("smallest length"),
    ))
    .expect("smallest length fits usize");
    let largest_len = usize::try_from(u32::from_le_bytes(
        source[124..128].try_into().expect("largest length"),
    ))
    .expect("largest length fits usize");
    let keys_end = 128 + smallest_len + largest_len;
    assert!(keys_end + 8 <= source.len());

    let mut descriptor = Vec::new();
    descriptor.extend_from_slice(&source[80..112]);
    descriptor.extend_from_slice(&source[120..124]);
    descriptor.extend_from_slice(&source[124..128]);
    descriptor.extend_from_slice(&source[128..keys_end]);
    let descriptor_crc = crc32fast::hash(&descriptor);
    descriptor.extend_from_slice(&descriptor_crc.to_le_bytes());

    let mut legacy = vec![0_u8; 80];
    legacy[0..8].copy_from_slice(b"DBLSMMAN");
    legacy[8..10].copy_from_slice(&2_u16.to_le_bytes());
    legacy[10..12].copy_from_slice(&80_u16.to_le_bytes());
    legacy[16..40].copy_from_slice(&source[16..40]);
    legacy[40..48].copy_from_slice(
        &u64::try_from(descriptor.len())
            .expect("descriptor length fits u64")
            .to_le_bytes(),
    );
    legacy[48..64].copy_from_slice(&source[48..64]);
    let header_crc = crc32fast::hash(&legacy[..76]);
    legacy[76..80].copy_from_slice(&header_crc.to_le_bytes());
    legacy.extend_from_slice(&descriptor);
    let file_crc = crc32fast::hash(&legacy);
    legacy.extend_from_slice(&file_crc.to_le_bytes());

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&manifest_path)
        .expect("open manifest fixture for downgrade");
    file.write_all(&legacy).expect("write Manifest v2 fixture");
    file.sync_all().expect("sync Manifest v2 fixture");
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
    assert_eq!(stats.tombstone_gc_sequence, 8);
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
fn manifest_v2_descriptor_reopens_as_l0_and_upgrades_through_v4_compaction() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
        engine.put(b"legacy-a", &large_value(0x61)).expect("put a");
        engine
            .put(b"legacy-b", &large_value(0x62))
            .expect("flush first L0 and rotate WAL");
        let stats = engine.stats().expect("v4 source stats");
        assert_eq!(stats.level0_sstables, 1);
        assert_eq!(stats.level1_sstables, 0);
        assert_eq!(stats.active_wal_id, 2);
    }

    assert_eq!(canonical_count(&path, "MANIFEST-", ""), 1);
    rewrite_single_table_manifest_as_v2(&path, 3);

    let mut reopened = LsmEngine::open(&path).expect("open Manifest v2 descriptor as implicit L0");
    let legacy = reopened.stats().expect("legacy v2 stats");
    assert_eq!(legacy.level0_sstables, 1);
    assert_eq!(legacy.level1_sstables, 0);
    assert_eq!(
        reopened.get(b"legacy-a").expect("read legacy a"),
        Some(large_value(0x61))
    );
    assert_eq!(
        reopened.get(b"legacy-b").expect("read legacy b"),
        Some(large_value(0x62))
    );

    for index in 0_u8..6 {
        let key = format!("upgrade-{index}").into_bytes();
        reopened
            .put(&key, &large_value(0x70 + index))
            .expect("build three additional L0 tables");
    }
    let upgraded = reopened.stats().expect("v4 upgraded stats");
    assert_eq!(upgraded.level0_sstables, 0);
    assert_eq!(upgraded.level1_sstables, 1);
    assert_eq!(upgraded.sstables, 1);
    assert_eq!(upgraded.tombstone_gc_sequence, upgraded.durable_sequence);
    reopened.reopen().expect("reopen upgraded v4 L1");
    assert_eq!(reopened.stats().expect("reopened v4 stats"), upgraded);
    assert_eq!(
        reopened.get(b"legacy-a").expect("read migrated a"),
        Some(large_value(0x61))
    );
}

#[test]
fn full_set_compaction_drops_tombstone_and_new_l0_can_reinsert() {
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

    assert_eq!(
        engine
            .current_entry(b"victim")
            .expect("read compacted entry"),
        None,
        "full-set compaction must physically elide the authoritative tombstone"
    );
    assert_eq!(engine.get(b"victim").expect("deleted victim"), None);
    let compacted = engine.stats().expect("L1 stats");
    assert_eq!(compacted.level1_sstables, 1);
    assert_eq!(compacted.sstable_entries, 7);
    assert_eq!(compacted.durable_sequence, 9);
    assert_eq!(compacted.tombstone_gc_sequence, 9);
    engine.reopen().expect("reopen tombstone-elided L1");
    assert_eq!(
        engine.current_entry(b"victim").expect("reopened entry"),
        None
    );
    assert_eq!(engine.get(b"victim").expect("reopened tombstone"), None);

    engine.put(b"victim", b"revived").expect("revive victim");
    engine.put(b"h-fill", &large_value(0x39)).expect("put h");
    engine
        .put(b"i-fill", &large_value(0x3a))
        .expect("flush new L0 over L1");
    let stats = engine.stats().expect("mixed-level stats");
    assert_eq!(stats.level0_sstables, 1);
    assert_eq!(stats.level1_sstables, 1);
    assert_eq!(stats.tombstone_gc_sequence, 9);
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
fn all_tombstone_compaction_publishes_tableless_durable_state() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
    let mut first_key = None;

    for batch in 0_u8..4 {
        for index in 0_u8..16 {
            let mut key = vec![0_u8; MAX_KEY_BYTES];
            key[0] = batch;
            key[1] = index;
            first_key.get_or_insert_with(|| key.clone());
            assert_eq!(engine.delete(&key).expect("delete missing key"), None);
        }
    }

    let stats = engine.stats().expect("table-less compacted stats");
    assert_eq!(stats.durable_sequence, 64);
    assert_eq!(stats.tombstone_gc_sequence, 64);
    assert_eq!(stats.sstables, 0);
    assert_eq!(stats.level0_sstables, 0);
    assert_eq!(stats.level1_sstables, 0);
    assert_eq!(stats.sstable_entries, 0);
    assert_eq!(stats.wal_records, 0);
    assert_eq!(stats.active_wal_first_sequence, 65);
    assert_eq!(canonical_count(&path, "sst-", ".sst"), 0);
    assert_eq!(canonical_count(&path, "MANIFEST-", ""), 1);

    let manifest = fs::read(only_manifest(&path)).expect("read table-less manifest");
    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("manifest version")),
        4
    );
    assert_eq!(
        u64::from_le_bytes(manifest[24..32].try_into().expect("durable")),
        64
    );
    assert_eq!(
        u64::from_le_bytes(manifest[32..40].try_into().expect("tables")),
        0
    );
    assert_eq!(
        u64::from_le_bytes(manifest[64..72].try_into().expect("GC sequence")),
        64
    );

    engine.reopen().expect("reopen table-less compacted state");
    assert_eq!(engine.stats().expect("reopened stats"), stats);
    assert_eq!(
        engine
            .get(first_key.as_deref().expect("first key"))
            .expect("get deleted key"),
        None
    );
    assert_eq!(
        LsmEngine::verify(&path)
            .expect("verify table-less state")
            .memtables,
        stats
    );

    engine
        .put(b"live-a", &large_value(0x91))
        .expect("put after empty compaction");
    engine
        .put(b"live-b", &large_value(0x92))
        .expect("flush after empty compaction");
    let refilled = engine.stats().expect("refilled stats");
    assert_eq!(refilled.sstables, 1);
    assert_eq!(refilled.level0_sstables, 1);
    assert_eq!(refilled.durable_sequence, 66);
    assert_eq!(refilled.tombstone_gc_sequence, 64);
    engine.reopen().expect("reopen refilled state");
    assert_eq!(
        engine.get(b"live-a").expect("get refilled key"),
        Some(large_value(0x91))
    );
}

#[test]
fn manifest_v3_reopens_and_upgrades_to_v4_on_next_install() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
        engine.put(b"legacy-a", &large_value(0xa1)).expect("put a");
        engine
            .put(b"legacy-b", &large_value(0xa2))
            .expect("flush first L0");
    }
    rewrite_manifest_as_v3(&path);

    let mut reopened = LsmEngine::open(&path).expect("open legacy Manifest v3");
    assert_eq!(
        reopened
            .stats()
            .expect("legacy v3 stats")
            .tombstone_gc_sequence,
        0
    );
    reopened
        .put(b"new-a", &large_value(0xa3))
        .expect("put new a");
    reopened
        .put(b"new-b", &large_value(0xa4))
        .expect("publish Manifest v4");
    let manifest = fs::read(only_manifest(&path)).expect("read upgraded manifest");
    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("manifest version")),
        4
    );
    reopened.reopen().expect("reopen upgraded Manifest v4");
    assert_eq!(
        reopened.get(b"legacy-a").expect("get legacy key"),
        Some(large_value(0xa1))
    );
    assert_eq!(
        reopened.get(b"new-b").expect("get new key"),
        Some(large_value(0xa4))
    );
}

#[test]
fn manifest_v4_rejects_unproven_gc_tableless_watermarks_and_reserved_bytes() {
    let live_directory = tempdir().expect("temporary live directory");
    let live_path = live_directory.path().join("engine");
    let mut live = LsmEngine::create_new(&live_path).expect("create live LSM");
    populate_four_flushes(&mut live);
    drop(live);
    let live_manifest = only_manifest(&live_path);
    let original = fs::read(&live_manifest).expect("read live manifest");
    let mut invalid_gc = original.clone();
    invalid_gc[64..72].copy_from_slice(&9_u64.to_le_bytes());
    rewrite_manifest_checksums(&mut invalid_gc);
    fs::write(&live_manifest, invalid_gc).expect("write invalid GC watermark");
    let error = LsmEngine::open(&live_path).expect_err("GC beyond durable sequence must fail");
    assert!(error.to_string().contains("GC sequence"), "{error}");

    let mut invalid_reserved = original;
    invalid_reserved[72] = 1;
    rewrite_manifest_checksums(&mut invalid_reserved);
    fs::write(&live_manifest, invalid_reserved).expect("write nonzero v4 reserved byte");
    let error = LsmEngine::open(&live_path).expect_err("nonzero v4 reserved byte must fail");
    assert!(error.to_string().contains("invalid v4 manifest"), "{error}");

    let empty_directory = tempdir().expect("temporary empty directory");
    let empty_path = empty_directory.path().join("engine");
    let mut empty = LsmEngine::create_new(&empty_path).expect("create empty LSM");
    for batch in 0_u8..4 {
        for index in 0_u8..16 {
            let mut key = vec![0_u8; MAX_KEY_BYTES];
            key[0] = batch;
            key[1] = index;
            empty.delete(&key).expect("build tombstone-only inputs");
        }
    }
    drop(empty);
    let empty_manifest = only_manifest(&empty_path);
    let mut bytes = fs::read(&empty_manifest).expect("read table-less manifest");
    bytes[64..72].copy_from_slice(&0_u64.to_le_bytes());
    rewrite_manifest_checksums(&mut bytes);
    fs::write(&empty_manifest, bytes).expect("write unproven table-less watermark");
    let error = LsmEngine::open(&empty_path).expect_err("table-less watermark must be GC-covered");
    assert!(error.to_string().contains("table-less manifest"), "{error}");
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
