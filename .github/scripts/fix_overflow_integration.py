from pathlib import Path

tree = Path("crates/db-storage-btree/src/tree.rs")
text = tree.read_text()
text = text.replace(
    "                PageKind::Internal => {\n                    let children = decode_internal(&page)?;\n                    page_id = children[route_child(&children, key)?].page_id;\n                }\n            }",
    "                PageKind::Internal => {\n                    let children = decode_internal(&page)?;\n                    page_id = children[route_child(&children, key)?].page_id;\n                }\n                PageKind::Overflow => {\n                    return Err(corruption(\n                        0,\n                        format!(\"lookup reached overflow page {page_id} as a tree node\"),\n                    ));\n                }\n            }",
    1,
)
text = text.replace(
    "            PageKind::Internal => {\n                let mut children = decode_internal(&page)?;\n                let child_index = route_child(&children, key)?;\n                let child_id = children[child_index].page_id;\n                let replacements = self.rewrite_insert(child_id, key, value, depth + 1)?;\n                children.splice(child_index..=child_index, replacements);\n                self.commit_internal_level(&children)\n            }\n        }",
    "            PageKind::Internal => {\n                let mut children = decode_internal(&page)?;\n                let child_index = route_child(&children, key)?;\n                let child_id = children[child_index].page_id;\n                let replacements = self.rewrite_insert(child_id, key, value, depth + 1)?;\n                children.splice(child_index..=child_index, replacements);\n                self.commit_internal_level(&children)\n            }\n            PageKind::Overflow => Err(corruption(\n                0,\n                format!(\"insert traversal reached overflow page {page_id} as a tree node\"),\n            )),\n        }",
    1,
)
text = text.replace(
    "            value: b\"new-but-unpublished\".to_vec(),",
    "            value: super::StoredValue::Inline(b\"new-but-unpublished\".to_vec()),",
    1,
)
text = text.replace(
    "use super::{encode_leaf_cell, BPlusTree, LeafEntry, MAX_TREE_KEY_BYTES};",
    "use super::{\n        encode_leaf_cell, BPlusTree, LeafEntry, MAX_TREE_KEY_BYTES, MAX_TREE_VALUE_BYTES,\n    };",
    1,
)
text = text.replace(
    "fn oversized_key_or_single_entry_is_rejected_before_writing()",
    "fn oversized_key_or_value_is_rejected_before_writing()",
    1,
)
text = text.replace(
    "            .put(b\"key\", &vec![0_u8; 5000])\n            .expect_err(\"oversized inline entry must fail\");",
    "            .put(b\"key\", &vec![0_u8; MAX_TREE_VALUE_BYTES + 1])\n            .expect_err(\"oversized value must fail\");",
    1,
)
tree.write_text(text)
