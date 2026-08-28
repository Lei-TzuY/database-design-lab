from pathlib import Path

path = Path("crates/db-storage-lsm/src/compaction_tests.rs")
text = path.read_text()
marker = "\n\n#[test]\nfn manifest_v5_rejects_unproven_gc_tableless_and_allocation_watermarks()"
if marker not in text:
    raise SystemExit("missing Manifest v5 semantic test marker")
if "tableless_manifest_v4_migration_uses_conservative_allocation_floor" in text:
    raise SystemExit("tableless v4 migration test already present")

test = r'''

#[test]
fn tableless_manifest_v4_migration_uses_conservative_allocation_floor() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
        for batch in 0_u8..4 {
            for index in 0_u8..16 {
                let mut key = vec![0_u8; MAX_KEY_BYTES];
                key[0] = batch;
                key[1] = index;
                assert_eq!(engine.delete(&key).expect("build tombstone-only inputs"), None);
            }
        }
        assert_eq!(engine.stats().expect("v5 table-less stats").sstables, 0);
    }

    rewrite_manifest_as_v4(&path);
    let mut reopened = LsmEngine::open(&path).expect("open table-less Manifest v4");
    let legacy = reopened.stats().expect("legacy table-less stats");
    assert_eq!(legacy.durable_sequence, 64);
    assert_eq!(legacy.tombstone_gc_sequence, 64);
    assert_eq!(legacy.sstables, 0);

    reopened
        .put(b"live-a", &large_value(0xd1))
        .expect("put first post-v4 value");
    reopened
        .put(b"live-b", &large_value(0xd2))
        .expect("flush first post-v4 SSTable");
    assert!(
        path.join("sst-0000000000000065.sst").exists(),
        "v4 table-less history has no exact allocation watermark, so v5 migration must conservatively reserve ids through the durable sequence"
    );
    let manifest = fs::read(only_manifest(&path)).expect("read upgraded Manifest v5");
    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("manifest version")),
        5
    );
    assert_eq!(
        u64::from_le_bytes(
            manifest[72..80]
                .try_into()
                .expect("SSTable id high watermark"),
        ),
        65
    );
    reopened.reopen().expect("reopen migrated table 65");
    assert_eq!(
        reopened.get(b"live-a").expect("read migrated value"),
        Some(large_value(0xd1))
    );
}
'''
path.write_text(text.replace(marker, test + marker, 1))
