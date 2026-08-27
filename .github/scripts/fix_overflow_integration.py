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
    "            value: StoredValue::Inline(b\"new-but-unpublished\".to_vec()),",
    1,
)
# The shadow-page regression lives in this module, so StoredValue is already in scope as a sibling item.
tree.write_text(text)
