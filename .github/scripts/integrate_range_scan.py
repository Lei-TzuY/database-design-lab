from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing marker: {label}")
    return text.replace(old, new, 1)

# Common trait: add validated half-open ordered range scans with a default unsupported path.
engine = Path("crates/db-core/src/engine.rs")
text = engine.read_text()
text = replace_once(
    text,
    "    /// Closes and reopens engine state, replaying persistent state where applicable.\n    fn reopen(&mut self) -> Result<()>;\n",
    "    /// Returns up to `limit` key/value pairs in ascending bytewise key order from `[start, end)`.\n"
    "    ///\n"
    "    /// `end = None` means no upper bound. Engines whose capability advertises\n"
    "    /// `ordered_range_scan = false` may reject this operation.\n"
    "    fn range_scan(\n"
    "        &mut self,\n"
    "        start: &[u8],\n"
    "        end: Option<&[u8]>,\n"
    "        limit: usize,\n"
    "    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {\n"
    "        let _ = (start, end, limit);\n"
    "        Err(DbError::InvalidInput(format!(\n"
    "            \"engine {} does not expose ordered range scans\",\n"
    "            self.capabilities().name\n"
    "        )))\n"
    "    }\n\n"
    "    /// Closes and reopens engine state, replaying persistent state where applicable.\n"
    "    fn reopen(&mut self) -> Result<()>;\n",
    "KvEngine range_scan",
)
text = replace_once(
    text,
    "/// Validates a key/value pair against the common semantics.\npub fn validate_key_value(key: &[u8], value: &[u8]) -> Result<()> {\n",
    "/// Validates half-open ordered range bounds against the common key semantics.\n"
    "pub fn validate_range_scan(start: &[u8], end: Option<&[u8]>) -> Result<()> {\n"
    "    validate_key(start)?;\n"
    "    if let Some(end) = end {\n"
    "        validate_key(end)?;\n"
    "        if end < start {\n"
    "            return Err(DbError::InvalidInput(\n"
    "                \"ordered range end must not sort before start\".to_owned(),\n"
    "            ));\n"
    "        }\n"
    "    }\n"
    "    Ok(())\n"
    "}\n\n"
    "/// Validates a key/value pair against the common semantics.\npub fn validate_key_value(key: &[u8], value: &[u8]) -> Result<()> {\n",
    "range validation helper",
)
engine.write_text(text)

core_lib = Path("crates/db-core/src/lib.rs")
text = core_lib.read_text()
text = replace_once(
    text,
    "    execute_step, execute_workload, validate_key, validate_key_value, ConcurrencyMode,\n",
    "    execute_step, execute_workload, validate_key, validate_key_value, validate_range_scan,\n    ConcurrencyMode,\n",
    "export range validation",
)
core_lib.write_text(text)

# Memory oracle: expose the same ordered semantics via BTreeMap.
memory = Path("crates/db-storage-memory/src/lib.rs")
text = memory.read_text()
text = replace_once(
    text,
    "use std::collections::BTreeMap;\n",
    "use std::collections::BTreeMap;\nuse std::ops::Bound;\n",
    "memory Bound import",
)
text = replace_once(text, "            ordered_range_scan: false,\n", "            ordered_range_scan: true,\n", "memory capability")
text = replace_once(
    text,
    "    fn reopen(&mut self) -> Result<()> {\n        Ok(())\n    }\n",
    "    fn range_scan(\n"
    "        &mut self,\n"
    "        start: &[u8],\n"
    "        end: Option<&[u8]>,\n"
    "        limit: usize,\n"
    "    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {\n"
    "        db_core::validate_range_scan(start, end)?;\n"
    "        if limit == 0 || end.is_some_and(|end| end == start) {\n"
    "            return Ok(Vec::new());\n"
    "        }\n"
    "        let lower = Bound::Included(start.to_vec());\n"
    "        let upper = end\n"
    "            .map(|end| Bound::Excluded(end.to_vec()))\n"
    "            .unwrap_or(Bound::Unbounded);\n"
    "        Ok(self\n"
    "            .values\n"
    "            .range((lower, upper))\n"
    "            .take(limit)\n"
    "            .map(|(key, value)| (key.clone(), value.clone()))\n"
    "            .collect())\n"
    "    }\n\n"
    "    fn reopen(&mut self) -> Result<()> {\n        Ok(())\n    }\n",
    "memory range implementation",
)
text = replace_once(
    text,
    "        assert_eq!(engine.get(b\"key\").expect(\"get\"), Some(Vec::new()));\n",
    "        assert_eq!(engine.get(b\"key\").expect(\"get\"), Some(Vec::new()));\n"
    "        assert_eq!(\n"
    "            engine\n"
    "                .range_scan(b\"\", None, 8)\n"
    "                .expect(\"ordered range scan\"),\n"
    "            vec![(b\"key\".to_vec(), Vec::new())]\n"
    "        );\n",
    "memory range test",
)
memory.write_text(text)

# B+ tree: keep scan logic isolated from mutation code.
tree = Path("crates/db-storage-btree/src/tree.rs")
text = tree.read_text()
text = replace_once(text, "mod reuse;\n", "mod reuse;\nmod scan;\n", "scan module")
tree.write_text(text)

scan = Path("crates/db-storage-btree/src/tree/scan.rs")
scan.write_text(r'''use super::{validate_key, BPlusTree, MAX_TREE_HEIGHT};
use crate::{corruption, PageKind, Result};

impl BPlusTree {
    /// Returns up to `limit` live key/value pairs in ascending bytewise key order from `[start, end)`.
    ///
    /// `end = None` means no upper bound. The point-tree format intentionally keeps leaf sibling
    /// pointers at zero; scans therefore walk internal children in key order. Exact child minimum
    /// separators let traversal skip subtrees that end before `start`, while `end` and `limit` stop
    /// the walk without introducing any additional persistent edge that copy-on-write mutation would
    /// need to maintain.
    pub fn range_scan(
        &mut self,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        validate_key(start)?;
        if let Some(end) = end {
            validate_key(end)?;
            if end < start {
                return Err(crate::BtreeError::InvalidInput(
                    "ordered range end must not sort before start".to_owned(),
                ));
            }
        }
        if limit == 0 || end.is_some_and(|end| end == start) {
            return Ok(Vec::new());
        }
        let Some(root) = self.pager.root_page_id() else {
            return Ok(Vec::new());
        };

        let mut rows = Vec::new();
        self.scan_subtree(root, start, end, limit, 0, &mut rows)?;
        Ok(rows)
    }

    fn scan_subtree(
        &mut self,
        page_id: u64,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
        depth: usize,
        rows: &mut Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<bool> {
        if depth >= MAX_TREE_HEIGHT {
            return Err(corruption(
                0,
                format!("B+ tree range scan exceeded maximum height {MAX_TREE_HEIGHT}"),
            ));
        }
        let page = self.pager.read_page(page_id)?;
        match page.kind() {
            PageKind::Leaf => {
                let entries = self.decode_leaf(&page, None)?;
                for entry in entries {
                    let key = entry.key.as_slice();
                    if key < start {
                        continue;
                    }
                    if end.is_some_and(|end| key >= end) {
                        return Ok(true);
                    }
                    let value = self.load_value(&entry.value)?;
                    rows.push((key.to_vec(), value));
                    if rows.len() >= limit {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            PageKind::Internal => {
                let children = self.decode_internal(&page, None)?;
                for (index, child) in children.iter().enumerate() {
                    if children
                        .get(index + 1)
                        .is_some_and(|next| next.min_key.as_slice() <= start)
                    {
                        continue;
                    }
                    if end.is_some_and(|end| child.min_key.as_slice() >= end) {
                        return Ok(true);
                    }
                    if self.scan_subtree(child.page_id, start, end, limit, depth + 1, rows)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            PageKind::Overflow => Err(corruption(
                0,
                format!("range traversal reached overflow page {page_id} as a tree node"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::BPlusTree;

    fn key(index: u16) -> Vec<u8> {
        index.to_be_bytes().to_vec()
    }

    #[test]
    fn half_open_ranges_are_sorted_limited_and_read_only() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("scan.db");
        let mut tree = BPlusTree::create_new(&path, 8).expect("create tree");
        for index in 0..96_u16 {
            tree.put(&key(index), &vec![(index & 0xff) as u8; 128])
                .expect("insert row");
        }
        assert!(tree.height().expect("height") >= 2);
        let generation = tree.generation();
        let page_count = tree.data_page_count();

        let rows = tree
            .range_scan(&key(17), Some(&key(44)), 9)
            .expect("bounded range");
        assert_eq!(rows.len(), 9);
        assert_eq!(rows.first().expect("first").0, key(17));
        assert_eq!(rows.last().expect("last").0, key(25));
        assert!(rows.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(tree.generation(), generation);
        assert_eq!(tree.data_page_count(), page_count);

        assert!(tree
            .range_scan(&key(44), Some(&key(44)), 100)
            .expect("empty equal-bound range")
            .is_empty());
        assert!(tree
            .range_scan(&key(0), None, 0)
            .expect("zero-limit range")
            .is_empty());
        assert!(tree.range_scan(&key(50), Some(&key(49)), 1).is_err());
    }

    #[test]
    fn range_scan_survives_reopen_delete_and_overflow_values() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("scan-reopen.db");
        let mut tree = BPlusTree::create_new(&path, 4).expect("create tree");
        let large = vec![0x5a; 32 * 1024];
        for index in 0..40_u16 {
            let value = if index == 21 {
                large.clone()
            } else {
                vec![(index & 0xff) as u8; 192]
            };
            tree.put(&key(index), &value).expect("insert row");
        }
        tree.delete(&key(20)).expect("delete predecessor");
        tree.delete(&key(22)).expect("delete successor");
        drop(tree);

        let mut reopened = BPlusTree::open(&path, 4).expect("reopen tree");
        let rows = reopened
            .range_scan(&key(19), Some(&key(24)), 16)
            .expect("scan reopened tree");
        let keys = rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>();
        assert_eq!(keys, vec![key(19), key(21), key(23)]);
        assert_eq!(rows[1].1, large);
    }
}
''')

# Common adapter and cross-engine range differential regression.
common = Path("crates/db-storage-btree/src/tree/common.rs")
text = common.read_text()
text = replace_once(text, "            ordered_range_scan: false,\n", "            ordered_range_scan: true,\n", "btree capability")
text = replace_once(
    text,
    "    fn reopen(&mut self) -> db_core::Result<()> {\n",
    "    fn range_scan(\n"
    "        &mut self,\n"
    "        start: &[u8],\n"
    "        end: Option<&[u8]>,\n"
    "        limit: usize,\n"
    "    ) -> db_core::Result<Vec<(Vec<u8>, Vec<u8>)>> {\n"
    "        BPlusTree::range_scan(self, start, end, limit).map_err(common_error)\n"
    "    }\n\n"
    "    fn reopen(&mut self) -> db_core::Result<()> {\n",
    "btree common range",
)
text = replace_once(
    text,
    "        assert!(!capabilities.ordered_range_scan);\n",
    "        assert!(capabilities.ordered_range_scan);\n",
    "btree capability test",
)
insert_marker = "    #[test]\n    fn common_differential_harness_covers_limits_delete_and_reopen() {\n"
range_test = r'''    #[test]
    fn common_ordered_ranges_match_memory_after_splits_reopen_and_delete() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("range-differential.db");
        let mut reference = MemoryEngine::new();
        let mut candidate = BPlusTree::create_new(&path, 8).expect("create tree");

        for index in 0..96_u16 {
            let key = index.to_be_bytes().to_vec();
            let value = if index == 48 {
                vec![0x7c; 64 * 1024]
            } else {
                vec![(index & 0xff) as u8; 160]
            };
            reference.put(&key, &value).expect("reference insert");
            candidate.put(&key, &value).expect("candidate insert");
        }
        candidate.reopen().expect("candidate reopen");
        reference.reopen().expect("reference reopen");
        for index in [0_u16, 17, 47, 49, 95] {
            let key = index.to_be_bytes();
            assert_eq!(
                reference.delete(&key).expect("reference delete"),
                candidate.delete(&key).expect("candidate delete")
            );
        }

        let cases = [
            (0_u16, Some(96_u16), 200_usize),
            (16, Some(52), 11),
            (48, Some(49), 8),
            (80, None, 7),
            (30, Some(30), 10),
        ];
        for (start, end, limit) in cases {
            let start = start.to_be_bytes();
            let end_bytes = end.map(u16::to_be_bytes);
            let end = end_bytes.as_ref().map(<[u8; 2]>::as_slice);
            assert_eq!(
                reference
                    .range_scan(&start, end, limit)
                    .expect("reference range"),
                candidate
                    .range_scan(&start, end, limit)
                    .expect("candidate range")
            );
        }
    }

'''
text = replace_once(text, insert_marker, range_test + insert_marker, "common range differential test")
common.write_text(text)
