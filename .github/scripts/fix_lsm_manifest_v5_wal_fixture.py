from pathlib import Path

path = Path("crates/db-storage-lsm/src/wal_rotation_tests.rs")
text = path.read_text()
old = '''    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("version")),
        4
    );'''
new = '''    assert_eq!(
        u16::from_le_bytes(manifest[8..10].try_into().expect("version")),
        5
    );'''
if old not in text:
    raise SystemExit("missing WAL rotation manifest-version fixture")
path.write_text(text.replace(old, new, 1))
