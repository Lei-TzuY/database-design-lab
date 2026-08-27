from pathlib import Path

path = Path("crates/db-storage-btree/src/tree/common.rs")
text = path.read_text()
text = text.replace(
    "    LogicalModel, Persistence, StorageArchitecture, MAX_KEY_BYTES, MAX_VALUE_BYTES,\n",
    "    LogicalModel, Persistence, StorageArchitecture,\n",
    1,
)
path.write_text(text)
