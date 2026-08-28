from pathlib import Path

for name in [
    "crates/db-storage-lsm/src/bloom.rs",
    "docs/lsm-sstable-manifest-format.md",
    "docs/roadmap.md",
]:
    path = Path(name)
    text = path.read_text()
    text = text.replace("424", "422").replace("0.848%", "0.844%")
    path.write_text(text)
