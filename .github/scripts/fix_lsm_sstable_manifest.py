from pathlib import Path

path = Path("crates/db-storage-lsm/src/lib.rs")
text = path.read_text()
marker = "#[cfg(test)]\nmod tests;\n"
replacement = "#[cfg(test)]\nmod sstable_tests;\n#[cfg(test)]\nmod tests;\n"
if replacement not in text:
    if marker not in text:
        raise SystemExit("missing LSM test-module marker")
    text = text.replace(marker, replacement, 1)
path.write_text(text)
