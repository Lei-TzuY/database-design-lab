from pathlib import Path

tree = Path("crates/db-storage-btree/src/tree.rs")
text = tree.read_text()
text = text.replace("let mut entries = decode_leaf(&page)?;", "let mut entries = self.decode_leaf(&page, None)?;")
text = text.replace("let mut children = decode_internal(&page)?;", "let mut children = self.decode_internal(&page, None)?;")
text = text.replace("for suffix in 0_u8..8 {", "for suffix in 0_u8..96 {")
text = text.replace("let value = vec![suffix; 32];", "let value = vec![suffix; 64];")
text = text.replace("for (key, value) in keys.iter().take(7) {", "for (key, value) in keys.iter().take(95) {")
text = text.replace("reopened.get(&keys[7].0)", "reopened.get(&keys[95].0)")
text = text.replace("Some(keys[7].1.clone())", "Some(keys[95].1.clone())")
tree.write_text(text)

delete = Path("crates/db-storage-btree/src/tree/delete.rs")
text = delete.read_text()
text = text.replace(
    "    cells_fit, choose_split, decode_internal, decode_leaf, encode_internal_cell, encode_leaf_cell,\n",
    "    cells_fit, choose_split, encode_internal_cell, encode_leaf_cell,\n",
    1,
)
delete.write_text(text)
