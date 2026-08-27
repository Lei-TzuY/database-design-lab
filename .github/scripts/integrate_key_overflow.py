from pathlib import Path


def replace_once(text, old, new, label):
    if old not in text:
        raise SystemExit(f"missing marker: {label}")
    return text.replace(old, new, 1)


def replace_between(text, start, end, new, label):
    a = text.find(start)
    if a < 0:
        raise SystemExit(f"missing start: {label}")
    b = text.find(end, a)
    if b < 0:
        raise SystemExit(f"missing end: {label}")
    return text[:a] + new + text[b:]


tree_path = Path("crates/db-storage-btree/src/tree.rs")
text = tree_path.read_text()
text = text.replace(
    "/// Maximum key size accepted by the first executable B+ tree layer.\n///\n/// The common KV contract permits larger keys, but this initial page-local representation keeps\n/// separators bounded so a two-child internal root is always representable on one 4 KiB page.\npub const MAX_TREE_KEY_BYTES: usize = 1024;",
    "/// Maximum key size accepted by the tree; matches the common KV key contract.\npub const MAX_TREE_KEY_BYTES: usize = 4 * 1024;",
    1,
)
text = replace_once(
    text,
    "const LEAF_CELL_HEADER_LEN: usize = 6;\n",
    "const LEAF_CELL_HEADER_LEN: usize = 6;\nconst KEY_OVERFLOW_FLAG: u16 = 1 << 15;\nconst KEY_LENGTH_MASK: u16 = KEY_OVERFLOW_FLAG - 1;\nconst KEY_OVERFLOW_REF_LEN: usize = 8;\n",
    "key constants",
)
text = replace_once(
    text,
    "const PAGE_BODY_CAPACITY: usize = CHECKSUM_OFFSET - DATA_HEADER_LEN;\n",
    "const PAGE_BODY_CAPACITY: usize = CHECKSUM_OFFSET - DATA_HEADER_LEN;\nconst MAX_INLINE_KEY_BYTES: usize =\n    PAGE_BODY_CAPACITY - SLOT_LEN - LEAF_CELL_HEADER_LEN - OVERFLOW_VALUE_REF_LEN;\n",
    "inline key capacity",
)

# Ordinary traversal must resolve key descriptors; reachability traversal additionally collects them.
text = text.replace("let entries = decode_leaf(&page)?;", "let entries = self.decode_leaf(&page, None)?;")
text = text.replace("let children = decode_internal(&page)?;", "let children = self.decode_internal(&page, None)?;")
# The two validation-site calls need the global reachable set rather than None.
validation_leaf = "                let entries = self.decode_leaf(&page, None)?;\n                for entry in &entries {\n                    self.validate_value_reachability(&entry.value, seen)?;\n                }"
text = replace_once(
    text,
    validation_leaf,
    "                let entries = self.decode_leaf(&page, Some(seen))?;\n                for entry in &entries {\n                    self.validate_value_reachability(&entry.value, seen)?;\n                }",
    "validation leaf decode",
)
validation_internal = "                let children = self.decode_internal(&page, None)?;\n                if children.len() < 2 {"
text = replace_once(
    text,
    validation_internal,
    "                let children = self.decode_internal(&page, Some(seen))?;\n                if children.len() < 2 {",
    "validation internal decode",
)

new_put = '''    /// Inserts or replaces one binary key/value pair and returns the previous value.
    ///
    /// Keys through 4 KiB and values through 1 MiB are accepted. Keys that cannot remain inline
    /// while still leaving room for an overflow-value descriptor are stored in checksummed overflow
    /// pages. All key/value overflow pages are durable before the replacement leaf and final root are
    /// published.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key_value(key, value)?;
        let previous = self.get(key)?;
        self.refresh_reusable_pages()?;
        let stored_key = self.store_key(key)?;
        let stored_value = self.store_value(&stored_key, value)?;

        let replacements = match self.pager.root_page_id() {
            Some(root) => self.rewrite_insert(root, &stored_key, &stored_value, 0)?,
            None => {
                let entry = LeafEntry {
                    key: stored_key.clone(),
                    value: stored_value.clone(),
                };
                vec![self.commit_leaf(&[entry])?]
            }
        };

        let new_root = match replacements.as_slice() {
            [only] => only.page_id,
            [_, _] => self.commit_internal(&replacements)?.page_id,
            _ => {
                return Err(corruption(
                    0,
                    "insert rewrite returned an invalid number of root replacements",
                ));
            }
        };
        self.pager.set_root(Some(new_root))?;
        Ok(previous)
    }

'''
text = replace_between(text, "    /// Inserts or replaces one binary key/value pair", "    fn rewrite_insert(", new_put, "put")

text = text.replace("        key: &[u8],\n        value: &StoredValue,", "        key: &StoredKey,\n        value: &StoredValue,", 1)
text = text.replace(
    "match entries.binary_search_by(|entry| entry.key.as_slice().cmp(key)) {\n                    Ok(index) => entries[index].value = value.clone(),\n                    Err(index) => entries.insert(\n                        index,\n                        LeafEntry {\n                            key: key.to_vec(),\n                            value: value.clone(),\n                        },\n                    ),\n                }",
    "match entries.binary_search_by(|entry| entry.key.as_slice().cmp(key.as_slice())) {\n                    Ok(index) => {\n                        entries[index].key = key.clone();\n                        entries[index].value = value.clone();\n                    }\n                    Err(index) => entries.insert(\n                        index,\n                        LeafEntry {\n                            key: key.clone(),\n                            value: value.clone(),\n                        },\n                    ),\n                }",
    1,
)
text = text.replace("let child_index = route_child(&children, key)?;", "let child_index = route_child(&children, key.as_slice())?;", 1)

text = text.replace("            min_key: first.key.clone(),", "            min_key: first.key.as_slice().to_vec(),", 1)
text = text.replace("                    min_key: first.key.clone(),", "                    min_key: first.key.as_slice().to_vec(),", 1)
text = text.replace("                    max_key: last.key.clone(),", "                    max_key: last.key.as_slice().to_vec(),", 1)

new_internal_commit = '''    fn commit_internal_with_cells(
        &mut self,
        children: &[ChildRef],
        preview_cells: &[Vec<u8>],
    ) -> Result<ChildRef> {
        let first = children.first().ok_or_else(|| {
            BtreeError::InvalidInput("cannot persist an empty internal page".to_owned())
        })?;
        if preview_cells.len() != children.len() || !cells_fit(preview_cells) {
            return Err(corruption(
                0,
                "internal cell preview disagrees with committed page shape",
            ));
        }

        let mut cells = Vec::with_capacity(children.len());
        for child in children {
            let stored_key = self.store_key(&child.min_key)?;
            cells.push(encode_internal_cell_stored(&stored_key, child.page_id)?);
        }
        if cells
            .iter()
            .zip(preview_cells)
            .any(|(actual, preview)| actual.len() != preview.len())
            || !cells_fit(&cells)
        {
            return Err(corruption(
                0,
                "materialized internal separator cells changed their previewed size",
            ));
        }

        let (mut page, recycled) = self.prepare_tree_page(PageKind::Internal)?;
        for cell in &cells {
            page.insert_cell(cell)?;
        }
        let page_id = self.commit_tree_page(page, recycled)?;
        Ok(ChildRef {
            min_key: first.min_key.clone(),
            page_id,
        })
    }

'''
text = replace_between(text, "    fn commit_internal_with_cells(", "    fn validate_reachable_tree", new_internal_commit, "commit internal with cells")

# Replace key/value cell representation and decoders as one coherent block.
start = "#[derive(Debug, Clone, PartialEq, Eq)]\nstruct LeafEntry"
end = "fn validate_leaf_order(entries: &[LeafEntry], page_id: u64) -> Result<()> {"
new_codec = r'''#[derive(Debug, Clone, PartialEq, Eq)]
struct LeafEntry {
    key: StoredKey,
    value: StoredValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredKey {
    Inline(Vec<u8>),
    Overflow { bytes: Vec<u8>, first_page_id: u64 },
}

impl StoredKey {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline(bytes) | Self::Overflow { bytes, .. } => bytes,
        }
    }

    fn encoded_payload_len(&self) -> usize {
        match self {
            Self::Inline(bytes) => bytes.len(),
            Self::Overflow { .. } => KEY_OVERFLOW_REF_LEN,
        }
    }

    fn encoded_field(&self) -> Result<u16> {
        let len = u16::try_from(self.as_slice().len()).map_err(|_| {
            BtreeError::InvalidInput("key length does not fit the on-page u16 field".to_owned())
        })?;
        if len > KEY_LENGTH_MASK {
            return Err(BtreeError::InvalidInput(
                "key length collides with the overflow marker bit".to_owned(),
            ));
        }
        Ok(match self {
            Self::Inline(_) => len,
            Self::Overflow { .. } => KEY_OVERFLOW_FLAG | len,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredValue {
    Inline(Vec<u8>),
    Overflow { len: u32, first_page_id: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildRef {
    min_key: Vec<u8>,
    page_id: u64,
}

#[derive(Debug)]
struct SubtreeBounds {
    min_key: Vec<u8>,
    max_key: Vec<u8>,
    height: usize,
}

fn validate_key(key: &[u8]) -> Result<()> {
    if key.len() > MAX_TREE_KEY_BYTES {
        return Err(BtreeError::InvalidInput(format!(
            "B+ tree key has {} bytes; maximum is {MAX_TREE_KEY_BYTES}",
            key.len()
        )));
    }
    Ok(())
}

fn validate_key_value(key: &[u8], value: &[u8]) -> Result<()> {
    validate_key(key)?;
    if value.len() > MAX_TREE_VALUE_BYTES {
        return Err(BtreeError::InvalidInput(format!(
            "B+ tree value has {} bytes; maximum is {MAX_TREE_VALUE_BYTES}",
            value.len()
        )));
    }
    let value_len = u32::try_from(value.len()).map_err(|_| {
        BtreeError::InvalidInput("value length does not fit the on-page u32 field".to_owned())
    })?;
    if value_len > VALUE_LENGTH_MASK {
        return Err(BtreeError::InvalidInput(
            "value length collides with the overflow marker bit".to_owned(),
        ));
    }
    Ok(())
}

fn encode_leaf_cell(entry: &LeafEntry) -> Result<Vec<u8>> {
    validate_key(entry.key.as_slice())?;
    let key_field = entry.key.encoded_field()?;
    let key_payload_len = entry.key.encoded_payload_len();
    let (value_field, value_payload_len) = match &entry.value {
        StoredValue::Inline(value) => {
            if value.len() > MAX_TREE_VALUE_BYTES {
                return Err(BtreeError::InvalidInput(format!(
                    "B+ tree value has {} bytes; maximum is {MAX_TREE_VALUE_BYTES}",
                    value.len()
                )));
            }
            let len = u32::try_from(value.len()).map_err(|_| {
                BtreeError::InvalidInput("inline value length does not fit u32".to_owned())
            })?;
            (len, value.len())
        }
        StoredValue::Overflow { len, first_page_id } => {
            if *len == 0 || usize::try_from(*len).unwrap_or(usize::MAX) > MAX_TREE_VALUE_BYTES {
                return Err(BtreeError::InvalidInput(
                    "overflow value length is outside supported bounds".to_owned(),
                ));
            }
            if *first_page_id < SUPERBLOCK_COUNT {
                return Err(BtreeError::InvalidInput(format!(
                    "overflow value page {first_page_id} overlaps mirrored superblocks"
                )));
            }
            (VALUE_OVERFLOW_FLAG | *len, OVERFLOW_VALUE_REF_LEN)
        }
    };
    let mut cell = Vec::with_capacity(
        LEAF_CELL_HEADER_LEN + key_payload_len + value_payload_len,
    );
    cell.extend_from_slice(&key_field.to_le_bytes());
    cell.extend_from_slice(&value_field.to_le_bytes());
    match &entry.key {
        StoredKey::Inline(key) => cell.extend_from_slice(key),
        StoredKey::Overflow { first_page_id, .. } => {
            cell.extend_from_slice(&first_page_id.to_le_bytes());
        }
    }
    match &entry.value {
        StoredValue::Inline(value) => cell.extend_from_slice(value),
        StoredValue::Overflow { first_page_id, .. } => {
            cell.extend_from_slice(&first_page_id.to_le_bytes());
        }
    }
    Ok(cell)
}

fn encode_internal_cell(child: &ChildRef) -> Result<Vec<u8>> {
    validate_key(&child.min_key)?;
    if child.page_id < SUPERBLOCK_COUNT {
        return Err(BtreeError::InvalidInput(format!(
            "child page {} overlaps mirrored superblocks",
            child.page_id
        )));
    }
    let len = u16::try_from(child.min_key.len()).map_err(|_| {
        BtreeError::InvalidInput("separator length does not fit the on-page u16 field".to_owned())
    })?;
    let overflow = child.min_key.len() > MAX_INLINE_KEY_BYTES;
    let field = if overflow { KEY_OVERFLOW_FLAG | len } else { len };
    let payload_len = if overflow { KEY_OVERFLOW_REF_LEN } else { child.min_key.len() };
    let mut cell = Vec::with_capacity(INTERNAL_CELL_HEADER_LEN + payload_len);
    cell.extend_from_slice(&field.to_le_bytes());
    cell.extend_from_slice(&child.page_id.to_le_bytes());
    if overflow {
        cell.extend_from_slice(&0_u64.to_le_bytes());
    } else {
        cell.extend_from_slice(&child.min_key);
    }
    Ok(cell)
}

fn encode_internal_cell_stored(key: &StoredKey, page_id: u64) -> Result<Vec<u8>> {
    validate_key(key.as_slice())?;
    if page_id < SUPERBLOCK_COUNT {
        return Err(BtreeError::InvalidInput(format!(
            "child page {page_id} overlaps mirrored superblocks"
        )));
    }
    let field = key.encoded_field()?;
    let mut cell = Vec::with_capacity(INTERNAL_CELL_HEADER_LEN + key.encoded_payload_len());
    cell.extend_from_slice(&field.to_le_bytes());
    cell.extend_from_slice(&page_id.to_le_bytes());
    match key {
        StoredKey::Inline(bytes) => cell.extend_from_slice(bytes),
        StoredKey::Overflow { first_page_id, .. } => {
            cell.extend_from_slice(&first_page_id.to_le_bytes());
        }
    }
    Ok(cell)
}

impl BPlusTree {
    fn decode_leaf(
        &mut self,
        page: &Page,
        mut seen: Option<&mut BTreeSet<u64>>,
    ) -> Result<Vec<LeafEntry>> {
        if page.kind() != PageKind::Leaf {
            return Err(corruption(0, "attempted leaf decoding on a non-leaf page"));
        }
        let mut entries = Vec::with_capacity(usize::from(page.cell_count()));
        for index in 0..page.cell_count() {
            let cell = page.cell(index)?;
            if cell.len() < LEAF_CELL_HEADER_LEN {
                return Err(corruption(
                    0,
                    format!("leaf page {} slot {index} is shorter than its cell header", page.page_id()),
                ));
            }
            let key_field = u16::from_le_bytes([cell[0], cell[1]]);
            let key_overflow = key_field & KEY_OVERFLOW_FLAG != 0;
            let key_len_u16 = key_field & KEY_LENGTH_MASK;
            let key_len = usize::from(key_len_u16);
            if key_len > MAX_TREE_KEY_BYTES || (key_overflow && key_len == 0) {
                return Err(corruption(
                    0,
                    format!("leaf page {} contains invalid key length {key_len}", page.page_id()),
                ));
            }
            let key_payload_len = if key_overflow { KEY_OVERFLOW_REF_LEN } else { key_len };

            let value_field = u32::from_le_bytes([cell[2], cell[3], cell[4], cell[5]]);
            let value_overflow = value_field & VALUE_OVERFLOW_FLAG != 0;
            let value_len_u32 = value_field & VALUE_LENGTH_MASK;
            let value_len = usize::try_from(value_len_u32)
                .map_err(|_| corruption(0, "leaf value length does not fit usize"))?;
            if value_len > MAX_TREE_VALUE_BYTES || (value_overflow && value_len == 0) {
                return Err(corruption(
                    0,
                    format!("leaf page {} contains invalid value length {value_len}", page.page_id()),
                ));
            }
            let value_payload_len = if value_overflow { OVERFLOW_VALUE_REF_LEN } else { value_len };
            let expected = LEAF_CELL_HEADER_LEN
                .checked_add(key_payload_len)
                .and_then(|length| length.checked_add(value_payload_len))
                .ok_or_else(|| corruption(0, "leaf cell decoded length overflowed usize"))?;
            if expected != cell.len() {
                return Err(corruption(
                    0,
                    format!(
                        "leaf page {} slot {index} encodes {expected} bytes but slot contains {}",
                        page.page_id(), cell.len()
                    ),
                ));
            }

            let key_start = LEAF_CELL_HEADER_LEN;
            let key_end = key_start + key_payload_len;
            let key = if key_overflow {
                let bytes = &cell[key_start..key_end];
                let first_page_id = u64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                self.load_key_blob(first_page_id, key_len_u16, seen.as_deref_mut())?
            } else {
                StoredKey::Inline(cell[key_start..key_end].to_vec())
            };

            let value = if value_overflow {
                let bytes = &cell[key_end..key_end + OVERFLOW_VALUE_REF_LEN];
                let first_page_id = u64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                if first_page_id < SUPERBLOCK_COUNT {
                    return Err(corruption(
                        0,
                        format!("leaf page {} references invalid overflow value page {first_page_id}", page.page_id()),
                    ));
                }
                StoredValue::Overflow { len: value_len_u32, first_page_id }
            } else {
                StoredValue::Inline(cell[key_end..].to_vec())
            };
            entries.push(LeafEntry { key, value });
        }
        validate_leaf_order(&entries, page.page_id())?;
        Ok(entries)
    }

    fn decode_internal(
        &mut self,
        page: &Page,
        mut seen: Option<&mut BTreeSet<u64>>,
    ) -> Result<Vec<ChildRef>> {
        if page.kind() != PageKind::Internal {
            return Err(corruption(0, "attempted internal decoding on a non-internal page"));
        }
        let mut children = Vec::with_capacity(usize::from(page.cell_count()));
        for index in 0..page.cell_count() {
            let cell = page.cell(index)?;
            if cell.len() < INTERNAL_CELL_HEADER_LEN {
                return Err(corruption(
                    0,
                    format!("internal page {} slot {index} is shorter than its cell header", page.page_id()),
                ));
            }
            let key_field = u16::from_le_bytes([cell[0], cell[1]]);
            let key_overflow = key_field & KEY_OVERFLOW_FLAG != 0;
            let key_len_u16 = key_field & KEY_LENGTH_MASK;
            let key_len = usize::from(key_len_u16);
            if key_len > MAX_TREE_KEY_BYTES || (key_overflow && key_len == 0) {
                return Err(corruption(
                    0,
                    format!("internal page {} contains invalid separator length {key_len}", page.page_id()),
                ));
            }
            let key_payload_len = if key_overflow { KEY_OVERFLOW_REF_LEN } else { key_len };
            let expected = INTERNAL_CELL_HEADER_LEN
                .checked_add(key_payload_len)
                .ok_or_else(|| corruption(0, "internal cell decoded length overflowed usize"))?;
            if expected != cell.len() {
                return Err(corruption(
                    0,
                    format!(
                        "internal page {} slot {index} encodes {expected} bytes but slot contains {}",
                        page.page_id(), cell.len()
                    ),
                ));
            }
            let page_id = u64::from_le_bytes([
                cell[2], cell[3], cell[4], cell[5], cell[6], cell[7], cell[8], cell[9],
            ]);
            if page_id < SUPERBLOCK_COUNT {
                return Err(corruption(
                    0,
                    format!("internal page {} references invalid child {page_id}", page.page_id()),
                ));
            }
            let key = if key_overflow {
                let bytes = &cell[INTERNAL_CELL_HEADER_LEN..];
                let first_page_id = u64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                self.load_key_blob(first_page_id, key_len_u16, seen.as_deref_mut())?
                    .as_slice()
                    .to_vec()
            } else {
                cell[INTERNAL_CELL_HEADER_LEN..].to_vec()
            };
            children.push(ChildRef { min_key: key, page_id });
        }
        validate_child_refs(&children)?;
        Ok(children)
    }
}

'''
text = replace_between(text, start, end, new_codec, "key codecs")

text = text.replace("if pair[0].key >= pair[1].key {", "if pair[0].key.as_slice() >= pair[1].key.as_slice() {")

# Tests constructing raw entries now need an explicit inline-key descriptor.
text = text.replace(
    "use super::{encode_leaf_cell, BPlusTree, LeafEntry, MAX_TREE_KEY_BYTES, MAX_TREE_VALUE_BYTES};",
    "use super::{\n        encode_leaf_cell, BPlusTree, LeafEntry, StoredKey, MAX_TREE_KEY_BYTES,\n        MAX_TREE_VALUE_BYTES,\n    };",
    1,
)
text = text.replace("            key: b\"key\".to_vec(),", "            key: StoredKey::Inline(b\"key\".to_vec()),", 1)

# Add the contract-driving long-key regression before the bounds test.
marker = "    #[test]\n    fn oversized_key_or_value_is_rejected_before_writing() {"
long_test = r'''    #[test]
    fn four_kib_keys_with_prefix_adjacency_round_trip_split_and_reopen() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("long-keys.db");
        let mut tree = BPlusTree::create_new(&path, 8).expect("create tree");
        let mut keys = Vec::new();

        let prefix = vec![0x7a; MAX_TREE_KEY_BYTES - 1];
        for suffix in 0_u8..8 {
            let mut key = prefix.clone();
            key.push(suffix);
            let value = vec![suffix; 32];
            tree.put(&key, &value).expect("insert maximum key");
            keys.push((key, value));
        }
        let immediate_prefix = prefix.clone();
        tree.put(&immediate_prefix, b"prefix")
            .expect("insert prefix-adjacent key");

        assert!(tree.height().expect("height after long-key inserts") >= 2);
        assert_eq!(
            tree.get(&immediate_prefix).expect("prefix lookup"),
            Some(b"prefix".to_vec())
        );
        for (key, value) in &keys {
            assert_eq!(tree.get(key).expect("long-key lookup"), Some(value.clone()));
        }
        drop(tree);

        let mut reopened = BPlusTree::open(&path, 4).expect("reopen long-key tree");
        assert_eq!(
            reopened.get(&immediate_prefix).expect("reopened prefix"),
            Some(b"prefix".to_vec())
        );
        for (key, value) in &keys {
            assert_eq!(
                reopened.get(key).expect("reopened long-key lookup"),
                Some(value.clone())
            );
        }
        for (key, value) in keys.iter().take(7) {
            assert_eq!(
                reopened.delete(key).expect("delete long key"),
                Some(value.clone())
            );
        }
        assert_eq!(
            reopened.get(&keys[7].0).expect("surviving long key"),
            Some(keys[7].1.clone())
        );
    }

'''
text = replace_once(text, marker, long_test + marker, "long key test")
tree_path.write_text(text)

# Overflow module becomes the shared immutable blob layer for both keys and values.
overflow_path = Path("crates/db-storage-btree/src/tree/overflow.rs")
over = overflow_path.read_text()
over = over.replace(
    "    BPlusTree, PageKind, Result, StoredValue, LEAF_CELL_HEADER_LEN, MAX_TREE_VALUE_BYTES,\n    PAGE_BODY_CAPACITY, SLOT_LEN,",
    "    validate_key, BPlusTree, PageKind, Result, StoredKey, StoredValue, LEAF_CELL_HEADER_LEN,\n    MAX_INLINE_KEY_BYTES, MAX_TREE_KEY_BYTES, MAX_TREE_VALUE_BYTES, PAGE_BODY_CAPACITY, SLOT_LEN,",
    1,
)
over = replace_once(
    over,
    "impl BPlusTree {\n    pub(super) fn store_value(&mut self, key: &[u8], value: &[u8]) -> Result<StoredValue> {",
    '''impl BPlusTree {
    pub(super) fn store_key(&mut self, key: &[u8]) -> Result<StoredKey> {
        validate_key(key)?;
        if key.len() <= MAX_INLINE_KEY_BYTES {
            return Ok(StoredKey::Inline(key.to_vec()));
        }
        let first_page_id = self.commit_overflow_blob(key)?;
        Ok(StoredKey::Overflow {
            bytes: key.to_vec(),
            first_page_id,
        })
    }

    pub(super) fn load_key_blob(
        &mut self,
        first_page_id: u64,
        encoded_len: u16,
        seen: Option<&mut BTreeSet<u64>>,
    ) -> Result<StoredKey> {
        let bytes = self.read_overflow_blob(
            first_page_id,
            u32::from(encoded_len),
            seen,
            MAX_TREE_KEY_BYTES,
            "key",
        )?;
        Ok(StoredKey::Overflow { bytes, first_page_id })
    }

    pub(super) fn store_value(&mut self, key: &StoredKey, value: &[u8]) -> Result<StoredValue> {''',
    "overflow store key",
)
over = over.replace(
    ".and_then(|length| length.checked_add(key.len()))",
    ".and_then(|length| length.checked_add(key.encoded_payload_len()))",
    1,
)
over = over.replace(
    "self.read_overflow_blob(*first_page_id, *len, None)",
    "self.read_overflow_blob(\n                    *first_page_id,\n                    *len,\n                    None,\n                    MAX_TREE_VALUE_BYTES,\n                    \"value\",\n                )",
    1,
)
over = over.replace(
    "self.read_overflow_blob(*first_page_id, *len, Some(seen))?;",
    "self.read_overflow_blob(\n                *first_page_id,\n                *len,\n                Some(seen),\n                MAX_TREE_VALUE_BYTES,\n                \"value\",\n            )?;",
    1,
)
over = over.replace(
    "        mut global_seen: Option<&mut BTreeSet<u64>>,\n    ) -> Result<Vec<u8>> {",
    "        mut global_seen: Option<&mut BTreeSet<u64>>,\n        max_len: usize,\n        blob_kind: &str,\n    ) -> Result<Vec<u8>> {",
    1,
)
over = over.replace("expected_len > MAX_TREE_VALUE_BYTES", "expected_len > max_len", 1)
over = over.replace(
    'format!("overflow value encodes invalid length {expected_len}"),',
    'format!("overflow {blob_kind} encodes invalid length {expected_len}"),',
    1,
)
overflow_path.write_text(over)

# Delete traversal now calls the pager-aware decoders.
delete_path = Path("crates/db-storage-btree/src/tree/delete.rs")
delete = delete_path.read_text()
delete = delete.replace("decode_internal(&page)?", "self.decode_internal(&page, None)?")
delete = delete.replace("decode_leaf(&page)?", "self.decode_leaf(&page, None)?")
delete = delete.replace("decode_leaf(left)?", "self.decode_leaf(left, None)?")
delete = delete.replace("decode_leaf(right)?", "self.decode_leaf(right, None)?")
delete = delete.replace("decode_internal(left)?", "self.decode_internal(left, None)?")
delete = delete.replace("decode_internal(right)?", "self.decode_internal(right, None)?")
delete = delete.replace("if pair[0].key >= pair[1].key {", "if pair[0].key.as_slice() >= pair[1].key.as_slice() {")
delete_path.write_text(delete)

# The low-level reuse shadow fixture uses the new key descriptor explicitly.
reuse_path = Path("crates/db-storage-btree/src/tree/reuse.rs")
reuse = reuse_path.read_text()
reuse = reuse.replace("use crate::tree::StoredValue;", "use crate::tree::{StoredKey, StoredValue};", 1)
reuse = reuse.replace(
    '            key: b"key".to_vec(),',
    '            key: StoredKey::Inline(b"key".to_vec()),',
    1,
)
reuse_path.write_text(reuse)
