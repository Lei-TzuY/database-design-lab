use std::collections::BTreeSet;
use std::path::Path;

mod delete;

use super::{
    corruption, BtreeError, Page, PageKind, Pager, Result, CHECKSUM_OFFSET, DATA_HEADER_LEN,
    SLOT_LEN, SUPERBLOCK_COUNT,
};

/// Maximum key size accepted by the first executable B+ tree layer.
///
/// The common KV contract permits larger keys, but this initial page-local representation keeps
/// separators bounded so a two-child internal root is always representable on one 4 KiB page.
pub const MAX_TREE_KEY_BYTES: usize = 1024;

const LEAF_CELL_HEADER_LEN: usize = 6;
const INTERNAL_CELL_HEADER_LEN: usize = 10;
const MAX_TREE_HEIGHT: usize = 64;
const PAGE_BODY_CAPACITY: usize = CHECKSUM_OFFSET - DATA_HEADER_LEN;

/// Persistent copy-on-write B+ tree supporting binary point lookup and insertion/update.
///
/// Mutations never overwrite a reachable data page. A `put` rewrites the changed leaf and every
/// ancestor as newly appended immutable pages, synchronizes those pages through [`Pager`], then
/// publishes exactly one new root pointer through the mirrored superblock. A crash before that final
/// root publication leaves the previous tree authoritative; already committed shadow pages are
/// unreachable space that later reclamation work may recover.
#[derive(Debug)]
pub struct BPlusTree {
    pager: Pager,
}

impl BPlusTree {
    /// Creates a new empty tree without overwriting an existing file.
    pub fn create_new(path: impl AsRef<Path>, cache_capacity: usize) -> Result<Self> {
        Ok(Self {
            pager: Pager::create_new(path, cache_capacity)?,
        })
    }

    /// Opens an existing tree and validates every page reachable from the committed root.
    pub fn open(path: impl AsRef<Path>, cache_capacity: usize) -> Result<Self> {
        let mut tree = Self {
            pager: Pager::open(path, cache_capacity)?,
        };
        tree.validate_reachable_tree()?;
        Ok(tree)
    }

    /// Returns the backing path.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.pager.path()
    }

    /// Returns the currently published root page id.
    #[must_use]
    pub fn root_page_id(&self) -> Option<u64> {
        self.pager.root_page_id()
    }

    /// Returns the number of committed data pages, including unreachable copy-on-write history.
    #[must_use]
    pub fn data_page_count(&self) -> u64 {
        self.pager.data_page_count()
    }

    /// Returns the committed metadata generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.pager.generation()
    }

    /// Returns tree height (`0` for empty, `1` for a leaf root).
    pub fn height(&mut self) -> Result<usize> {
        let Some(root) = self.pager.root_page_id() else {
            return Ok(0);
        };
        let mut seen = BTreeSet::new();
        Ok(self.validate_subtree(root, 0, &mut seen)?.height)
    }

    /// Looks up one opaque binary key.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        let Some(mut page_id) = self.pager.root_page_id() else {
            return Ok(None);
        };

        for _ in 0..MAX_TREE_HEIGHT {
            let page = self.pager.read_page(page_id)?;
            match page.kind() {
                PageKind::Leaf => {
                    let entries = decode_leaf(&page)?;
                    return match entries.binary_search_by(|entry| entry.key.as_slice().cmp(key)) {
                        Ok(index) => Ok(Some(entries[index].value.clone())),
                        Err(_) => Ok(None),
                    };
                }
                PageKind::Internal => {
                    let children = decode_internal(&page)?;
                    page_id = children[route_child(&children, key)?].page_id;
                }
            }
        }

        Err(corruption(
            0,
            format!("B+ tree traversal exceeded maximum height {MAX_TREE_HEIGHT}"),
        ))
    }

    /// Inserts or replaces one binary key/value pair and returns the previous value.
    ///
    /// One encoded entry must fit on an otherwise empty leaf. Deletion and page reclamation are not
    /// part of this phase, so repeated updates intentionally leave unreachable historical pages.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key_value(key, value)?;
        let previous = self.get(key)?;

        let replacements = match self.pager.root_page_id() {
            Some(root) => self.rewrite_insert(root, key, value, 0)?,
            None => {
                let entry = LeafEntry {
                    key: key.to_vec(),
                    value: value.to_vec(),
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

    fn rewrite_insert(
        &mut self,
        page_id: u64,
        key: &[u8],
        value: &[u8],
        depth: usize,
    ) -> Result<Vec<ChildRef>> {
        if depth >= MAX_TREE_HEIGHT {
            return Err(corruption(
                0,
                format!("B+ tree insert exceeded maximum height {MAX_TREE_HEIGHT}"),
            ));
        }

        let page = self.pager.read_page(page_id)?;
        match page.kind() {
            PageKind::Leaf => {
                let mut entries = decode_leaf(&page)?;
                match entries.binary_search_by(|entry| entry.key.as_slice().cmp(key)) {
                    Ok(index) => entries[index].value = value.to_vec(),
                    Err(index) => entries.insert(
                        index,
                        LeafEntry {
                            key: key.to_vec(),
                            value: value.to_vec(),
                        },
                    ),
                }
                self.commit_leaf_level(&entries)
            }
            PageKind::Internal => {
                let mut children = decode_internal(&page)?;
                let child_index = route_child(&children, key)?;
                let child_id = children[child_index].page_id;
                let replacements = self.rewrite_insert(child_id, key, value, depth + 1)?;
                children.splice(child_index..=child_index, replacements);
                self.commit_internal_level(&children)
            }
        }
    }

    fn commit_leaf_level(&mut self, entries: &[LeafEntry]) -> Result<Vec<ChildRef>> {
        if entries.is_empty() {
            return Err(corruption(
                0,
                "attempted to persist an empty reachable leaf",
            ));
        }
        let cells = entries
            .iter()
            .map(encode_leaf_cell)
            .collect::<Result<Vec<_>>>()?;
        if cells_fit(&cells) {
            return Ok(vec![self.commit_leaf_with_cells(entries, &cells)?]);
        }

        let split = choose_split(&cells).ok_or_else(|| {
            BtreeError::InvalidInput(
                "leaf entries cannot be divided into two valid 4 KiB pages".to_owned(),
            )
        })?;
        let left = self.commit_leaf(&entries[..split])?;
        let right = self.commit_leaf(&entries[split..])?;
        Ok(vec![left, right])
    }

    fn commit_internal_level(&mut self, children: &[ChildRef]) -> Result<Vec<ChildRef>> {
        validate_child_refs(children)?;
        let cells = children
            .iter()
            .map(encode_internal_cell)
            .collect::<Result<Vec<_>>>()?;
        if cells_fit(&cells) {
            return Ok(vec![self.commit_internal_with_cells(children, &cells)?]);
        }

        let split = choose_split(&cells).ok_or_else(|| {
            BtreeError::InvalidInput(
                "internal separators cannot be divided into two valid 4 KiB pages".to_owned(),
            )
        })?;
        let left = self.commit_internal(&children[..split])?;
        let right = self.commit_internal(&children[split..])?;
        Ok(vec![left, right])
    }

    fn commit_leaf(&mut self, entries: &[LeafEntry]) -> Result<ChildRef> {
        let cells = entries
            .iter()
            .map(encode_leaf_cell)
            .collect::<Result<Vec<_>>>()?;
        if !cells_fit(&cells) {
            return Err(BtreeError::InvalidInput(
                "leaf entries exceed one 4 KiB page".to_owned(),
            ));
        }
        self.commit_leaf_with_cells(entries, &cells)
    }

    fn commit_leaf_with_cells(
        &mut self,
        entries: &[LeafEntry],
        cells: &[Vec<u8>],
    ) -> Result<ChildRef> {
        let first = entries.first().ok_or_else(|| {
            BtreeError::InvalidInput("cannot persist an empty leaf page".to_owned())
        })?;
        let mut page = self.pager.prepare_new_page(PageKind::Leaf)?;
        for cell in cells {
            page.insert_cell(cell)?;
        }
        let page_id = self.pager.commit_new_page(page)?;
        Ok(ChildRef {
            min_key: first.key.clone(),
            page_id,
        })
    }

    fn commit_internal(&mut self, children: &[ChildRef]) -> Result<ChildRef> {
        validate_child_refs(children)?;
        let cells = children
            .iter()
            .map(encode_internal_cell)
            .collect::<Result<Vec<_>>>()?;
        if !cells_fit(&cells) {
            return Err(BtreeError::InvalidInput(
                "internal separators exceed one 4 KiB page".to_owned(),
            ));
        }
        self.commit_internal_with_cells(children, &cells)
    }

    fn commit_internal_with_cells(
        &mut self,
        children: &[ChildRef],
        cells: &[Vec<u8>],
    ) -> Result<ChildRef> {
        let first = children.first().ok_or_else(|| {
            BtreeError::InvalidInput("cannot persist an empty internal page".to_owned())
        })?;
        let mut page = self.pager.prepare_new_page(PageKind::Internal)?;
        for cell in cells {
            page.insert_cell(cell)?;
        }
        let page_id = self.pager.commit_new_page(page)?;
        Ok(ChildRef {
            min_key: first.min_key.clone(),
            page_id,
        })
    }

    fn validate_reachable_tree(&mut self) -> Result<()> {
        let Some(root) = self.pager.root_page_id() else {
            return Ok(());
        };
        let mut seen = BTreeSet::new();
        self.validate_subtree(root, 0, &mut seen)?;
        Ok(())
    }

    fn validate_subtree(
        &mut self,
        page_id: u64,
        depth: usize,
        seen: &mut BTreeSet<u64>,
    ) -> Result<SubtreeBounds> {
        if depth >= MAX_TREE_HEIGHT {
            return Err(corruption(
                0,
                format!("reachable tree exceeds maximum height {MAX_TREE_HEIGHT}"),
            ));
        }
        if !seen.insert(page_id) {
            return Err(corruption(
                0,
                format!(
                    "reachable B+ tree contains a cycle or duplicate page reference at {page_id}"
                ),
            ));
        }

        let page = self.pager.read_page(page_id)?;
        match page.kind() {
            PageKind::Leaf => {
                let entries = decode_leaf(&page)?;
                let first = entries.first().ok_or_else(|| {
                    corruption(0, format!("reachable leaf page {page_id} is empty"))
                })?;
                let last = entries.last().expect("non-empty leaf has last entry");
                Ok(SubtreeBounds {
                    min_key: first.key.clone(),
                    max_key: last.key.clone(),
                    height: 1,
                })
            }
            PageKind::Internal => {
                let children = decode_internal(&page)?;
                if children.len() < 2 {
                    return Err(corruption(
                        0,
                        format!("reachable internal page {page_id} has fewer than two children"),
                    ));
                }

                let mut child_bounds = Vec::with_capacity(children.len());
                for child in &children {
                    let bounds = self.validate_subtree(child.page_id, depth + 1, seen)?;
                    if bounds.min_key != child.min_key {
                        return Err(corruption(
                            0,
                            format!(
                                "internal page {page_id} separator does not equal child {} minimum key",
                                child.page_id
                            ),
                        ));
                    }
                    child_bounds.push(bounds);
                }

                let height = child_bounds[0].height;
                for bounds in &child_bounds[1..] {
                    if bounds.height != height {
                        return Err(corruption(
                            0,
                            format!("internal page {page_id} has children at different heights"),
                        ));
                    }
                }
                for pair in child_bounds.windows(2) {
                    if pair[0].max_key >= pair[1].min_key {
                        return Err(corruption(
                            0,
                            format!("internal page {page_id} has overlapping child key ranges"),
                        ));
                    }
                }

                Ok(SubtreeBounds {
                    min_key: child_bounds[0].min_key.clone(),
                    max_key: child_bounds
                        .last()
                        .expect("internal page has children")
                        .max_key
                        .clone(),
                    height: height + 1,
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LeafEntry {
    key: Vec<u8>,
    value: Vec<u8>,
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
            "B+ tree key has {} bytes; current page-local maximum is {MAX_TREE_KEY_BYTES}",
            key.len()
        )));
    }
    Ok(())
}

fn validate_key_value(key: &[u8], value: &[u8]) -> Result<()> {
    validate_key(key)?;
    let cell_len = LEAF_CELL_HEADER_LEN
        .checked_add(key.len())
        .and_then(|length| length.checked_add(value.len()))
        .ok_or_else(|| BtreeError::InvalidInput("leaf entry length overflowed usize".to_owned()))?;
    let occupied = SLOT_LEN
        .checked_add(cell_len)
        .ok_or_else(|| BtreeError::InvalidInput("leaf entry extent overflowed usize".to_owned()))?;
    if occupied > PAGE_BODY_CAPACITY {
        return Err(BtreeError::InvalidInput(format!(
            "encoded key/value occupies {occupied} bytes; one entry must fit within the {PAGE_BODY_CAPACITY}-byte slotted-page body"
        )));
    }
    let _ = u32::try_from(value.len()).map_err(|_| {
        BtreeError::InvalidInput("value length does not fit the on-page u32 field".to_owned())
    })?;
    Ok(())
}

fn encode_leaf_cell(entry: &LeafEntry) -> Result<Vec<u8>> {
    validate_key_value(&entry.key, &entry.value)?;
    let key_len = u16::try_from(entry.key.len()).map_err(|_| {
        BtreeError::InvalidInput("key length does not fit the on-page u16 field".to_owned())
    })?;
    let value_len = u32::try_from(entry.value.len()).map_err(|_| {
        BtreeError::InvalidInput("value length does not fit the on-page u32 field".to_owned())
    })?;
    let mut cell = Vec::with_capacity(LEAF_CELL_HEADER_LEN + entry.key.len() + entry.value.len());
    cell.extend_from_slice(&key_len.to_le_bytes());
    cell.extend_from_slice(&value_len.to_le_bytes());
    cell.extend_from_slice(&entry.key);
    cell.extend_from_slice(&entry.value);
    Ok(cell)
}

fn decode_leaf(page: &Page) -> Result<Vec<LeafEntry>> {
    if page.kind() != PageKind::Leaf {
        return Err(corruption(0, "attempted leaf decoding on an internal page"));
    }
    let mut entries = Vec::with_capacity(usize::from(page.cell_count()));
    for index in 0..page.cell_count() {
        let cell = page.cell(index)?;
        if cell.len() < LEAF_CELL_HEADER_LEN {
            return Err(corruption(
                0,
                format!(
                    "leaf page {} slot {index} is shorter than its cell header",
                    page.page_id()
                ),
            ));
        }
        let key_len = usize::from(u16::from_le_bytes([cell[0], cell[1]]));
        let value_len = usize::try_from(u32::from_le_bytes([cell[2], cell[3], cell[4], cell[5]]))
            .map_err(|_| corruption(0, "leaf value length does not fit usize"))?;
        let expected = LEAF_CELL_HEADER_LEN
            .checked_add(key_len)
            .and_then(|length| length.checked_add(value_len))
            .ok_or_else(|| corruption(0, "leaf cell decoded length overflowed usize"))?;
        if expected != cell.len() {
            return Err(corruption(
                0,
                format!(
                    "leaf page {} slot {index} encodes {expected} bytes but slot contains {}",
                    page.page_id(),
                    cell.len()
                ),
            ));
        }
        if key_len > MAX_TREE_KEY_BYTES {
            return Err(corruption(
                0,
                format!(
                    "leaf page {} contains oversized key of {key_len} bytes",
                    page.page_id()
                ),
            ));
        }
        let key_end = LEAF_CELL_HEADER_LEN + key_len;
        entries.push(LeafEntry {
            key: cell[LEAF_CELL_HEADER_LEN..key_end].to_vec(),
            value: cell[key_end..].to_vec(),
        });
    }
    validate_leaf_order(&entries, page.page_id())?;
    Ok(entries)
}

fn encode_internal_cell(child: &ChildRef) -> Result<Vec<u8>> {
    validate_key(&child.min_key)?;
    if child.page_id < SUPERBLOCK_COUNT {
        return Err(BtreeError::InvalidInput(format!(
            "child page {} overlaps mirrored superblocks",
            child.page_id
        )));
    }
    let key_len = u16::try_from(child.min_key.len()).map_err(|_| {
        BtreeError::InvalidInput("separator length does not fit the on-page u16 field".to_owned())
    })?;
    let mut cell = Vec::with_capacity(INTERNAL_CELL_HEADER_LEN + child.min_key.len());
    cell.extend_from_slice(&key_len.to_le_bytes());
    cell.extend_from_slice(&child.page_id.to_le_bytes());
    cell.extend_from_slice(&child.min_key);
    Ok(cell)
}

fn decode_internal(page: &Page) -> Result<Vec<ChildRef>> {
    if page.kind() != PageKind::Internal {
        return Err(corruption(0, "attempted internal decoding on a leaf page"));
    }
    let mut children = Vec::with_capacity(usize::from(page.cell_count()));
    for index in 0..page.cell_count() {
        let cell = page.cell(index)?;
        if cell.len() < INTERNAL_CELL_HEADER_LEN {
            return Err(corruption(
                0,
                format!(
                    "internal page {} slot {index} is shorter than its cell header",
                    page.page_id()
                ),
            ));
        }
        let key_len = usize::from(u16::from_le_bytes([cell[0], cell[1]]));
        let expected = INTERNAL_CELL_HEADER_LEN
            .checked_add(key_len)
            .ok_or_else(|| corruption(0, "internal cell decoded length overflowed usize"))?;
        if expected != cell.len() {
            return Err(corruption(
                0,
                format!(
                    "internal page {} slot {index} encodes {expected} bytes but slot contains {}",
                    page.page_id(),
                    cell.len()
                ),
            ));
        }
        if key_len > MAX_TREE_KEY_BYTES {
            return Err(corruption(
                0,
                format!(
                    "internal page {} contains oversized separator of {key_len} bytes",
                    page.page_id()
                ),
            ));
        }
        let page_id = u64::from_le_bytes([
            cell[2], cell[3], cell[4], cell[5], cell[6], cell[7], cell[8], cell[9],
        ]);
        if page_id < SUPERBLOCK_COUNT {
            return Err(corruption(
                0,
                format!(
                    "internal page {} references invalid child {page_id}",
                    page.page_id()
                ),
            ));
        }
        children.push(ChildRef {
            min_key: cell[INTERNAL_CELL_HEADER_LEN..].to_vec(),
            page_id,
        });
    }
    validate_child_refs(&children)?;
    Ok(children)
}

fn validate_leaf_order(entries: &[LeafEntry], page_id: u64) -> Result<()> {
    for pair in entries.windows(2) {
        if pair[0].key >= pair[1].key {
            return Err(corruption(
                0,
                format!("leaf page {page_id} keys are not strictly increasing"),
            ));
        }
    }
    Ok(())
}

fn validate_child_refs(children: &[ChildRef]) -> Result<()> {
    if children.is_empty() {
        return Err(BtreeError::InvalidInput(
            "internal page requires at least one child".to_owned(),
        ));
    }
    for child in children {
        validate_key(&child.min_key)?;
        if child.page_id < SUPERBLOCK_COUNT {
            return Err(BtreeError::InvalidInput(format!(
                "internal child {} overlaps mirrored superblocks",
                child.page_id
            )));
        }
    }
    for pair in children.windows(2) {
        if pair[0].min_key >= pair[1].min_key {
            return Err(corruption(
                0,
                "internal separator keys are not strictly increasing",
            ));
        }
    }
    Ok(())
}

fn route_child(children: &[ChildRef], key: &[u8]) -> Result<usize> {
    validate_child_refs(children)?;
    match children.binary_search_by(|child| child.min_key.as_slice().cmp(key)) {
        Ok(index) => Ok(index),
        Err(0) => Ok(0),
        Err(index) => Ok(index - 1),
    }
}

fn cells_fit(cells: &[Vec<u8>]) -> bool {
    cells
        .iter()
        .try_fold(0_usize, |used, cell| {
            used.checked_add(SLOT_LEN)?.checked_add(cell.len())
        })
        .is_some_and(|used| used <= PAGE_BODY_CAPACITY)
}

fn choose_split(cells: &[Vec<u8>]) -> Option<usize> {
    if cells.len() < 2 {
        return None;
    }
    let sizes = cells
        .iter()
        .map(|cell| SLOT_LEN.checked_add(cell.len()))
        .collect::<Option<Vec<_>>>()?;
    let total = sizes
        .iter()
        .try_fold(0_usize, |sum, size| sum.checked_add(*size))?;
    let mut left = 0_usize;
    let mut best: Option<(usize, usize)> = None;
    for split in 1..cells.len() {
        left = left.checked_add(sizes[split - 1])?;
        let right = total.checked_sub(left)?;
        if left <= PAGE_BODY_CAPACITY && right <= PAGE_BODY_CAPACITY {
            let imbalance = left.abs_diff(right);
            if best.is_none_or(|(_, current)| imbalance < current) {
                best = Some((split, imbalance));
            }
        }
    }
    best.map(|(split, _)| split)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{encode_leaf_cell, BPlusTree, LeafEntry, MAX_TREE_KEY_BYTES};
    use crate::PageKind;

    #[test]
    fn binary_lookup_insert_update_and_reopen() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("tree.db");
        let mut tree = BPlusTree::create_new(&path, 8).expect("create tree");

        assert_eq!(tree.get(b"").expect("get empty key"), None);
        assert_eq!(tree.put(b"", b"zero").expect("insert empty key"), None);
        assert_eq!(
            tree.put(&[0xff, 0x00, 0x7f], b"binary")
                .expect("insert binary key"),
            None
        );
        assert_eq!(
            tree.get(b"").expect("read empty key"),
            Some(b"zero".to_vec())
        );
        assert_eq!(
            tree.put(b"", b"updated").expect("update empty key"),
            Some(b"zero".to_vec())
        );
        assert_eq!(tree.height().expect("height"), 1);
        drop(tree);

        let mut reopened = BPlusTree::open(&path, 4).expect("reopen tree");
        assert_eq!(
            reopened.get(b"").expect("read updated value"),
            Some(b"updated".to_vec())
        );
        assert_eq!(
            reopened
                .get(&[0xff, 0x00, 0x7f])
                .expect("read binary value"),
            Some(b"binary".to_vec())
        );
        assert_eq!(reopened.get(b"missing").expect("read missing"), None);
    }

    #[test]
    fn root_and_non_root_splits_preserve_every_key() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("splits.db");
        let mut tree = BPlusTree::create_new(&path, 3).expect("create tree");
        let mut keys = Vec::new();

        for number in 0_u16..20 {
            let mut key = vec![b'k'; 510];
            key.extend_from_slice(&number.to_be_bytes());
            let value = vec![(number % 251) as u8; 900];
            tree.put(&key, &value).expect("insert split workload");
            keys.push((key, value));
        }

        assert!(tree.height().expect("height after splits") >= 3);
        for (key, value) in &keys {
            assert_eq!(
                tree.get(key).expect("lookup after splits"),
                Some(value.clone())
            );
        }
        drop(tree);

        let mut reopened = BPlusTree::open(&path, 2).expect("reopen split tree");
        assert!(reopened.height().expect("reopened height") >= 3);
        for (key, value) in &keys {
            assert_eq!(
                reopened.get(key).expect("lookup after reopen"),
                Some(value.clone())
            );
        }
    }

    #[test]
    fn unpublished_shadow_page_does_not_change_committed_root() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("shadow.db");
        let mut tree = BPlusTree::create_new(&path, 2).expect("create tree");
        tree.put(b"key", b"old").expect("insert committed value");
        let committed_root = tree.root_page_id();

        let entry = LeafEntry {
            key: b"key".to_vec(),
            value: b"new-but-unpublished".to_vec(),
        };
        let cell = encode_leaf_cell(&entry).expect("encode shadow entry");
        let mut shadow = tree
            .pager
            .prepare_new_page(PageKind::Leaf)
            .expect("prepare shadow page");
        shadow.insert_cell(&cell).expect("pack shadow page");
        tree.pager
            .commit_new_page(shadow)
            .expect("commit unreachable shadow page");
        assert_eq!(tree.root_page_id(), committed_root);
        drop(tree);

        let mut reopened = BPlusTree::open(&path, 2).expect("reopen tree");
        assert_eq!(reopened.root_page_id(), committed_root);
        assert_eq!(
            reopened.get(b"key").expect("read authoritative value"),
            Some(b"old".to_vec())
        );
    }

    #[test]
    fn oversized_key_or_single_entry_is_rejected_before_writing() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("bounds.db");
        let mut tree = BPlusTree::create_new(&path, 1).expect("create tree");
        let initial_pages = tree.data_page_count();

        let error = tree
            .put(&vec![0_u8; MAX_TREE_KEY_BYTES + 1], b"value")
            .expect_err("oversized key must fail");
        assert!(matches!(error, crate::BtreeError::InvalidInput(_)));
        assert_eq!(tree.data_page_count(), initial_pages);

        let error = tree
            .put(b"key", &vec![0_u8; 5000])
            .expect_err("oversized inline entry must fail");
        assert!(matches!(error, crate::BtreeError::InvalidInput(_)));
        assert_eq!(tree.data_page_count(), initial_pages);
    }
}
