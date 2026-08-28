from pathlib import Path

path = Path("crates/db-storage-lsm/src/compaction_tests.rs")
text = path.read_text()
old = "    let mut invalid_reserved = original;"
new = "    let mut invalid_reserved = original.clone();"
if old not in text:
    raise SystemExit("missing Manifest v5 reserved-byte fixture marker")
path.write_text(text.replace(old, new, 1))
