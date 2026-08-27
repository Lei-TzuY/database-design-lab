from pathlib import Path

path = Path("crates/db-storage-btree/src/tree/scan.rs")
text = path.read_text()
old = "tree.put(&key(index), &vec![(index & 0xff) as u8; 128])"
new = "tree.put(&key(index), &[(index & 0xff) as u8; 128])"
if old not in text:
    raise SystemExit("missing scan lint marker")
path.write_text(text.replace(old, new, 1))
