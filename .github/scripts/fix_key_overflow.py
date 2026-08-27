from pathlib import Path

tree = Path("crates/db-storage-btree/src/tree.rs")
text = tree.read_text()
text = text.replace("let mut entries = decode_leaf(&page)?;", "let mut entries = self.decode_leaf(&page, None)?;")
text = text.replace("let mut children = decode_internal(&page)?;", "let mut children = self.decode_internal(&page, None)?;")
tree.write_text(text)

delete = Path("crates/db-storage-btree/src/tree/delete.rs")
text = delete.read_text()
text = text.replace(
    "    cells_fit, choose_split, decode_internal, decode_leaf, encode_internal_cell, encode_leaf_cell,\n",
    "    cells_fit, choose_split, encode_internal_cell, encode_leaf_cell,\n",
    1,
)
delete.write_text(text)
