from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing marker: {label}")
    return text.replace(old, new, 1)


lib = Path("crates/db-storage-btree/src/lib.rs")
text = lib.read_text()

page_kind_marker = '''impl PageKind {
    const fn encoded(self) -> u8 {
        match self {
            Self::Leaf => KIND_LEAF,
            Self::Internal => KIND_INTERNAL,
            Self::Overflow => KIND_OVERFLOW,
        }
    }

    fn decode(encoded: u8, offset: u64) -> Result<Self> {
        match encoded {
            KIND_LEAF => Ok(Self::Leaf),
            KIND_INTERNAL => Ok(Self::Internal),
            KIND_OVERFLOW => Ok(Self::Overflow),
            _ => Err(corruption(offset, format!("unknown page kind {encoded}"))),
        }
    }
}
'''
page_kind_insert = page_kind_marker + '''
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurableWriteKind {
    AppendPage(PageKind),
    RecycledPage(PageKind),
    AllocationSuperblock,
    RootSuperblock,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaultMode {
    BeforeWrite,
    TornWrite,
    AfterSync,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FaultSpec {
    event_index: usize,
    mode: FaultMode,
}
'''
text = replace_once(text, page_kind_marker, page_kind_insert, "durable write kinds")

text = replace_once(
    text,
    '''    recovered_allocation: Option<RecoveredAllocation>,
    poisoned: bool,
}''',
    '''    recovered_allocation: Option<RecoveredAllocation>,
    poisoned: bool,
    #[cfg(test)]
    fault_spec: Option<FaultSpec>,
    #[cfg(test)]
    fault_trace: Vec<DurableWriteKind>,
}''',
    "pager fault fields",
)

initializer = '''            recovered_allocation: None,
            poisoned: false,
        })'''
initializer_new = '''            recovered_allocation: None,
            poisoned: false,
            #[cfg(test)]
            fault_spec: None,
            #[cfg(test)]
            fault_trace: Vec::new(),
        })'''
text = replace_once(text, initializer, initializer_new, "create pager test fields")

initializer = '''            recovered_allocation,
            poisoned: false,
        })'''
initializer_new = '''            recovered_allocation,
            poisoned: false,
            #[cfg(test)]
            fault_spec: None,
            #[cfg(test)]
            fault_trace: Vec::new(),
        })'''
text = replace_once(text, initializer, initializer_new, "open pager test fields")

old_commit_io = '''        let write_result = (|| -> io::Result<()> {
            self.file.seek(SeekFrom::Start(expected_bytes))?;
            self.file.write_all(&page.bytes)?;
            self.file.sync_data()
        })();
        if let Err(error) = write_result {
            self.poisoned = true;
            return Err(BtreeError::Io(error));
        }
        if let Err(error) = write_superblock(&mut self.file, next) {
            self.poisoned = true;
            return Err(error);
        }
'''
new_commit_io = '''        if let Err(error) = self.write_durable_bytes(
            expected_bytes,
            &page.bytes,
            DurableWriteKind::AppendPage(page.kind()),
        ) {
            self.poisoned = true;
            return Err(error);
        }
        if let Err(error) =
            self.write_superblock_durable(next, DurableWriteKind::AllocationSuperblock)
        {
            self.poisoned = true;
            return Err(error);
        }
'''
text = replace_once(text, old_commit_io, new_commit_io, "append durable writes")

insert_before_read = '''    /// Reads and validates one committed data page.
    pub fn read_page(&mut self, page_id: u64) -> Result<Page> {
'''
helpers = r'''    fn commit_recycled_page(&mut self, page: Page) -> Result<u64> {
        self.ensure_usable()?;
        page.validate()?;
        self.validate_committed_page_id(page.page_id)?;
        let page_id = page.page_id;
        let offset = page_offset(page_id)?;
        if let Err(error) = self.write_durable_bytes(
            offset,
            &page.bytes,
            DurableWriteKind::RecycledPage(page.kind()),
        ) {
            self.poisoned = true;
            return Err(error);
        }
        self.cache.insert(page);
        Ok(page_id)
    }

    fn write_superblock_durable(
        &mut self,
        superblock: Superblock,
        kind: DurableWriteKind,
    ) -> Result<()> {
        let offset = u64::from(superblock.slot)
            .checked_mul(PAGE_SIZE_U64)
            .ok_or_else(|| corruption(0, "superblock offset overflowed u64"))?;
        self.write_durable_bytes(offset, &superblock.encode(), kind)
    }

    fn write_durable_bytes(
        &mut self,
        offset: u64,
        bytes: &[u8],
        _kind: DurableWriteKind,
    ) -> Result<()> {
        #[cfg(test)]
        {
            let event_index = self.fault_trace.len();
            self.fault_trace.push(_kind);
            if let Some(spec) = self.fault_spec {
                if spec.event_index == event_index {
                    match spec.mode {
                        FaultMode::BeforeWrite => {
                            return Err(injected_fault(_kind, spec.mode));
                        }
                        FaultMode::TornWrite => {
                            let prefix = (bytes.len() / 2).max(1);
                            self.file.seek(SeekFrom::Start(offset))?;
                            self.file.write_all(&bytes[..prefix])?;
                            self.file.sync_data()?;
                            return Err(injected_fault(_kind, spec.mode));
                        }
                        FaultMode::AfterSync => {
                            self.file.seek(SeekFrom::Start(offset))?;
                            self.file.write_all(bytes)?;
                            self.file.sync_data()?;
                            return Err(injected_fault(_kind, spec.mode));
                        }
                    }
                }
            }
        }

        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(bytes)?;
        self.file.sync_data()?;
        Ok(())
    }

    #[cfg(test)]
    fn begin_fault_trace_for_test(&mut self) {
        self.fault_spec = None;
        self.fault_trace.clear();
    }

    #[cfg(test)]
    fn inject_fault_for_test(&mut self, event_index: usize, mode: FaultMode) {
        self.fault_spec = Some(FaultSpec { event_index, mode });
        self.fault_trace.clear();
    }

    #[cfg(test)]
    fn fault_trace_for_test(&self) -> &[DurableWriteKind] {
        &self.fault_trace
    }

'''
text = replace_once(text, insert_before_read, helpers + insert_before_read, "pager durable helper insertion")

text = replace_once(
    text,
    '''        if let Err(error) = write_superblock(&mut self.file, next) {
            self.poisoned = true;
            return Err(error);
        }
        self.active = next;
''',
    '''        if let Err(error) = self.write_superblock_durable(next, DurableWriteKind::RootSuperblock) {
            self.poisoned = true;
            return Err(error);
        }
        self.active = next;
''',
    "root durable write",
)

old_free = '''fn write_superblock(file: &mut File, superblock: Superblock) -> Result<()> {
    let offset = u64::from(superblock.slot)
        .checked_mul(PAGE_SIZE_U64)
        .ok_or_else(|| corruption(0, "superblock offset overflowed u64"))?;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&superblock.encode())?;
    file.sync_data()?;
    Ok(())
}

'''
text = replace_once(text, old_free, "", "remove unfaultable superblock writer")

lib.write_text(text)

reuse = Path("crates/db-storage-btree/src/tree/reuse.rs")
text = reuse.read_text()
text = replace_once(
    text,
    '''use std::collections::BTreeSet;
use std::io::{self, Seek, SeekFrom, Write};
''',
    '''use std::collections::BTreeSet;
''',
    "reuse imports",
)
text = replace_once(
    text,
    '''use crate::{corruption, page_offset, SUPERBLOCK_COUNT};
''',
    '''use crate::{corruption, SUPERBLOCK_COUNT};
''',
    "reuse crate imports",
)
old_recycled = '''        self.pager.ensure_usable()?;
        page.validate()?;
        self.pager.validate_committed_page_id(page.page_id)?;
        let page_id = page.page_id;
        let offset = page_offset(page_id)?;
        let write_result = (|| -> io::Result<()> {
            self.pager.file.seek(SeekFrom::Start(offset))?;
            self.pager.file.write_all(&page.bytes)?;
            self.pager.file.sync_data()
        })();
        if let Err(error) = write_result {
            self.pager.poisoned = true;
            return Err(error.into());
        }

        self.pager.cache.insert(page);
        Ok(page_id)
'''
text = replace_once(
    text,
    old_recycled,
    '''        self.pager.commit_recycled_page(page)
''',
    "centralize recycled durable write",
)
reuse.write_text(text)

tree = Path("crates/db-storage-btree/src/tree.rs")
text = tree.read_text()
text = replace_once(
    text,
    '''mod common;
mod delete;
mod overflow;
''',
    '''mod common;
mod delete;
#[cfg(test)]
mod fault;
mod overflow;
''',
    "fault test module",
)
tree.write_text(text)

fault = Path("crates/db-storage-btree/src/tree/fault.rs")
fault.write_text(r'''use std::fs;
use std::path::{Path, PathBuf};

use tempfile::{tempdir, TempDir};

use super::{BPlusTree, LeafEntry, StoredKey, StoredValue};
use crate::{BtreeError, DurableWriteKind, FaultMode, PageKind};

const LEFT_KEY: &[u8] = b"a";
const TARGET_KEY: &[u8] = b"m";
const RIGHT_KEY: &[u8] = b"z";
const SMALL_OLD: &[u8] = b"old";
const LARGE_LEN: usize = 12 * 1024;

fn large(byte: u8) -> Vec<u8> {
    vec![byte; LARGE_LEN]
}

fn inline_entry(key: &[u8], value: &[u8]) -> LeafEntry {
    LeafEntry {
        key: StoredKey::Inline(key.to_vec()),
        value: StoredValue::Inline(value.to_vec()),
    }
}

fn build_two_leaf_fixture(path: &Path, target_value: &[u8]) -> BPlusTree {
    let mut tree = BPlusTree::create_new(path, 8).expect("create direct fixture");
    let target_key = StoredKey::Inline(TARGET_KEY.to_vec());
    let target_value = tree
        .store_value(&target_key, target_value)
        .expect("store target fixture value");
    let left = tree
        .commit_leaf(&[
            inline_entry(LEFT_KEY, b"left"),
            LeafEntry {
                key: target_key,
                value: target_value,
            },
        ])
        .expect("commit left fixture leaf");
    let right = tree
        .commit_leaf(&[inline_entry(RIGHT_KEY, b"right")])
        .expect("commit right fixture leaf");
    let root = tree
        .commit_internal(&[left, right])
        .expect("commit direct fixture root");
    tree.pager
        .set_root(Some(root.page_id))
        .expect("publish direct fixture root");
    assert!(tree
        .reusable_page_ids_for_test()
        .expect("derive direct fixture reuse")
        .is_empty());
    tree
}

fn clone_db(source: &Path, directory: &TempDir, name: &str) -> PathBuf {
    let target = directory.path().join(name);
    fs::copy(source, &target).expect("copy database snapshot");
    target
}

fn trace_put(source: &Path, directory: &TempDir, value: &[u8]) -> Vec<DurableWriteKind> {
    let path = clone_db(source, directory, "trace-put.db");
    let mut tree = BPlusTree::open(&path, 8).expect("open trace fixture");
    tree.pager.begin_fault_trace_for_test();
    tree.put(TARGET_KEY, value).expect("trace put");
    tree.pager.fault_trace_for_test().to_vec()
}

fn trace_delete(source: &Path, directory: &TempDir) -> Vec<DurableWriteKind> {
    let path = clone_db(source, directory, "trace-delete.db");
    let mut tree = BPlusTree::open(&path, 8).expect("open delete trace fixture");
    tree.pager.begin_fault_trace_for_test();
    tree.delete(TARGET_KEY).expect("trace delete");
    tree.pager.fault_trace_for_test().to_vec()
}

fn event_index(trace: &[DurableWriteKind], site: DurableWriteKind) -> usize {
    trace
        .iter()
        .position(|candidate| *candidate == site)
        .unwrap_or_else(|| panic!("fault trace did not contain {site:?}: {trace:?}"))
}

fn expected_new(site: DurableWriteKind, mode: FaultMode) -> bool {
    site == DurableWriteKind::RootSuperblock && mode == FaultMode::AfterSync
}

fn assert_put_fault(
    baseline: &Path,
    directory: &TempDir,
    case: usize,
    event_index: usize,
    site: DurableWriteKind,
    mode: FaultMode,
    old_value: &[u8],
    new_value: &[u8],
) {
    let path = clone_db(baseline, directory, &format!("put-{case}.db"));
    let mut tree = BPlusTree::open(&path, 8).expect("open put fault fixture");
    tree.pager.inject_fault_for_test(event_index, mode);
    let error = tree
        .put(TARGET_KEY, new_value)
        .expect_err("injected put must return an I/O error");
    assert!(matches!(error, BtreeError::Io(_)), "{site:?} {mode:?}: {error}");
    assert!(matches!(tree.get(TARGET_KEY), Err(BtreeError::Poisoned)));
    drop(tree);

    let mut reopened = BPlusTree::open(&path, 8).expect("reopen put fault fixture");
    let expected = if expected_new(site, mode) {
        new_value
    } else {
        old_value
    };
    assert_eq!(
        reopened.get(TARGET_KEY).expect("read target after fault"),
        Some(expected.to_vec()),
        "{site:?} {mode:?}"
    );
    assert_eq!(
        reopened.get(LEFT_KEY).expect("read left sentinel"),
        Some(b"left".to_vec())
    );
    assert_eq!(
        reopened.get(RIGHT_KEY).expect("read right sentinel"),
        Some(b"right".to_vec())
    );

    if mode == FaultMode::TornWrite && matches!(site, DurableWriteKind::RecycledPage(_)) {
        let repair = large(0x44);
        reopened
            .put(TARGET_KEY, &repair)
            .expect("later mutation overwrites torn unreachable page");
        drop(reopened);
        let mut repaired = BPlusTree::open(&path, 8).expect("reopen repaired tree");
        assert_eq!(
            repaired.get(TARGET_KEY).expect("read repaired target"),
            Some(repair)
        );
    }
}

#[test]
fn append_fault_matrix_preserves_old_or_complete_new_tree() {
    let directory = tempdir().expect("temporary directory");
    let baseline = directory.path().join("append-baseline.db");
    drop(build_two_leaf_fixture(&baseline, SMALL_OLD));
    let new_value = large(0x31);
    let trace = trace_put(&baseline, &directory, &new_value);
    let sites = [
        DurableWriteKind::AppendPage(PageKind::Overflow),
        DurableWriteKind::AppendPage(PageKind::Leaf),
        DurableWriteKind::AppendPage(PageKind::Internal),
        DurableWriteKind::AllocationSuperblock,
        DurableWriteKind::RootSuperblock,
    ];
    let modes = [
        FaultMode::BeforeWrite,
        FaultMode::TornWrite,
        FaultMode::AfterSync,
    ];

    let mut case = 0;
    for site in sites {
        let index = event_index(&trace, site);
        for mode in modes {
            assert_put_fault(
                &baseline,
                &directory,
                case,
                index,
                site,
                mode,
                SMALL_OLD,
                &new_value,
            );
            case += 1;
        }
    }
}

#[test]
fn recycled_fault_matrix_keeps_authoritative_root_and_repairs_torn_orphans() {
    let directory = tempdir().expect("temporary directory");
    let baseline = directory.path().join("recycled-baseline.db");
    let first = large(0x51);
    let second = large(0x52);
    let third = large(0x53);
    let mut tree = build_two_leaf_fixture(&baseline, &first);
    tree.put(TARGET_KEY, &second)
        .expect("create unreachable history for recycled fixture");
    drop(tree);

    let trace = trace_put(&baseline, &directory, &third);
    assert!(
        !trace.iter().any(|event| matches!(event, DurableWriteKind::AppendPage(_))),
        "recycled fixture unexpectedly appended pages: {trace:?}"
    );
    let sites = [
        DurableWriteKind::RecycledPage(PageKind::Overflow),
        DurableWriteKind::RecycledPage(PageKind::Leaf),
        DurableWriteKind::RecycledPage(PageKind::Internal),
        DurableWriteKind::RootSuperblock,
    ];
    let modes = [
        FaultMode::BeforeWrite,
        FaultMode::TornWrite,
        FaultMode::AfterSync,
    ];

    let mut case = 100;
    for site in sites {
        let index = event_index(&trace, site);
        for mode in modes {
            assert_put_fault(
                &baseline,
                &directory,
                case,
                index,
                site,
                mode,
                &second,
                &third,
            );
            case += 1;
        }
    }
}

#[test]
fn final_delete_root_clear_is_old_or_new_when_metadata_write_is_ambiguous() {
    let directory = tempdir().expect("temporary directory");
    let baseline = directory.path().join("delete-baseline.db");
    let mut tree = BPlusTree::create_new(&baseline, 4).expect("create delete fixture");
    tree.put(TARGET_KEY, SMALL_OLD).expect("insert delete target");
    drop(tree);

    let trace = trace_delete(&baseline, &directory);
    assert_eq!(trace, vec![DurableWriteKind::RootSuperblock]);
    for (case, mode) in [
        FaultMode::BeforeWrite,
        FaultMode::TornWrite,
        FaultMode::AfterSync,
    ]
    .into_iter()
    .enumerate()
    {
        let path = clone_db(&baseline, &directory, &format!("delete-{case}.db"));
        let mut tree = BPlusTree::open(&path, 4).expect("open delete fault fixture");
        tree.pager.inject_fault_for_test(0, mode);
        let error = tree
            .delete(TARGET_KEY)
            .expect_err("injected delete must return an I/O error");
        assert!(matches!(error, BtreeError::Io(_)));
        assert!(matches!(tree.get(TARGET_KEY), Err(BtreeError::Poisoned)));
        drop(tree);

        let mut reopened = BPlusTree::open(&path, 4).expect("reopen delete fault fixture");
        if mode == FaultMode::AfterSync {
            assert_eq!(reopened.root_page_id(), None);
            assert_eq!(reopened.get(TARGET_KEY).expect("read deleted key"), None);
        } else {
            assert!(reopened.root_page_id().is_some());
            assert_eq!(
                reopened.get(TARGET_KEY).expect("read preserved key"),
                Some(SMALL_OLD.to_vec())
            );
        }
    }
}
''')
