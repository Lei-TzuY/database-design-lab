from pathlib import Path

# Historical manifest snapshots are now reclaimable only after the same authoritative manifest has
# been published through both CURRENT mirrors. Keep the orphan-id assertion on the newly allocated
# SSTable, but stop requiring the superseded manifest snapshot to remain physically present.
p = Path("crates/db-storage-lsm/src/sstable_tests.rs")
text = p.read_text()
old = '''    assert!(numbered_file(&path, "sst-", ".sst", 100).exists());
    assert!(numbered_file(&path, "MANIFEST-", "", 100).exists());
    reopened.reopen().expect("reopen after skipping orphan ids");
    assert!(reopened.stats().expect("final stats").sstables >= 2);
'''
new = '''    assert!(numbered_file(&path, "sst-", ".sst", 100).exists());
    assert!(
        !numbered_file(&path, "sst-", ".sst", 99).exists(),
        "obsolete canonical orphan may be reclaimed after double-mirror publication"
    );
    reopened.reopen().expect("reopen after skipping orphan ids");
    assert!(reopened.stats().expect("final stats").sstables >= 2);
'''
if old not in text:
    raise SystemExit("missing orphan regression block")
p.write_text(text.replace(old, new, 1))

# Newly written manifests are v3 because descriptors now carry an authoritative level. The WAL
# authority fields retain their v2 offsets in the unchanged 80-byte header.
p = Path("crates/db-storage-lsm/src/wal_rotation_tests.rs")
text = p.read_text()
old = '''    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("version")),
        2
    );
'''
new = '''    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("version")),
        3
    );
'''
if old not in text:
    raise SystemExit("missing manifest-version regression block")
p.write_text(text.replace(old, new, 1))
