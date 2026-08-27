from pathlib import Path


def replace_once(text, old, new, label):
    if old not in text:
        raise SystemExit(f"missing marker: {label}")
    return text.replace(old, new, 1)

# db-core: name the physical architecture and crash-recovery protocol now exercised by B+ tree.
core = Path("crates/db-core/src/engine.rs")
text = core.read_text()
text = replace_once(
    text,
    "    /// Versioned checksummed mutation log plus replay index.\n    AppendLog,\n",
    "    /// Versioned checksummed mutation log plus replay index.\n    AppendLog,\n    /// Checksummed page file with a copy-on-write B+ tree and mirrored metadata.\n    BPlusTree,\n",
    "storage architecture variant",
)
text = replace_once(
    text,
    "    /// Valid records replay and a structurally valid incomplete final append is discarded.\n    TruncatedFinalAppend,\n",
    "    /// Valid records replay and a structurally valid incomplete final append is discarded.\n    TruncatedFinalAppend,\n    /// Durable COW pages are published by alternating checksummed root metadata copies.\n    MirroredCopyOnWritePages,\n",
    "crash recovery variant",
)
core.write_text(text)

# B+ tree depends on the common contract and uses the memory engine only as a differential-test oracle.
manifest = Path("crates/db-storage-btree/Cargo.toml")
text = manifest.read_text()
text = replace_once(
    text,
    "[dependencies]\ncrc32fast.workspace = true\nthiserror.workspace = true\n\n[dev-dependencies]\ntempfile.workspace = true\n",
    "[dependencies]\ncrc32fast.workspace = true\ndb-core = { path = \"../db-core\" }\nthiserror.workspace = true\n\n[dev-dependencies]\ndb-storage-memory = { path = \"../db-storage-memory\" }\ntempfile.workspace = true\n",
    "btree dependencies",
)
manifest.write_text(text)

# Keep the lockfile deterministic: both new dependencies are existing workspace packages.
lock = Path("Cargo.lock")
text = lock.read_text()
text = replace_once(
    text,
    'dependencies = [\n "crc32fast",\n "tempfile",\n "thiserror",\n]\n\n[[package]]\nname = "db-storage-log"',
    'dependencies = [\n "crc32fast",\n "db-core",\n "db-storage-memory",\n "tempfile",\n "thiserror",\n]\n\n[[package]]\nname = "db-storage-log"',
    "btree lock dependencies",
)
lock.write_text(text)

# Retain the constructor's cache policy so common REOPEN can reconstruct an equivalent live engine.
tree = Path("crates/db-storage-btree/src/tree.rs")
text = tree.read_text()
text = replace_once(
    text,
    "pub struct BPlusTree {\n    pager: Pager,\n    reusable_pages: VecDeque<u64>,\n}",
    "pub struct BPlusTree {\n    pager: Pager,\n    reusable_pages: VecDeque<u64>,\n    cache_capacity: usize,\n}",
    "cache capacity field",
)
text = replace_once(
    text,
    "        Ok(Self {\n            pager: Pager::create_new(path, cache_capacity)?,\n            reusable_pages: VecDeque::new(),\n        })",
    "        Ok(Self {\n            pager: Pager::create_new(path, cache_capacity)?,\n            reusable_pages: VecDeque::new(),\n            cache_capacity,\n        })",
    "create cache capacity",
)
text = replace_once(
    text,
    "        let mut tree = Self {\n            pager: Pager::open(path, cache_capacity)?,\n            reusable_pages: VecDeque::new(),\n        };",
    "        let mut tree = Self {\n            pager: Pager::open(path, cache_capacity)?,\n            reusable_pages: VecDeque::new(),\n            cache_capacity,\n        };",
    "open cache capacity",
)
text = replace_once(text, "mod delete;\n", "mod common;\nmod delete;\n", "common module")
tree.write_text(text)

common = Path("crates/db-storage-btree/src/tree/common.rs")
common.write_text(r'''use db_core::{
    ConcurrencyMode, CrashRecovery, DbError, DistributionMode, EngineCapabilities, KvEngine,
    LogicalModel, Persistence, StorageArchitecture, MAX_KEY_BYTES, MAX_VALUE_BYTES,
};

use super::{BPlusTree, MAX_TREE_KEY_BYTES, MAX_TREE_VALUE_BYTES};
use crate::BtreeError;

fn common_error(error: BtreeError) -> DbError {
    match error {
        BtreeError::InvalidInput(reason) => DbError::InvalidInput(reason),
        BtreeError::Io(error) => DbError::Io(error),
        BtreeError::Corruption { offset, reason } => DbError::Corruption { offset, reason },
        BtreeError::UnsupportedVersion { found, supported } => DbError::UnsupportedVersion {
            format: "B+ tree page file",
            found,
            supported,
        },
        BtreeError::Poisoned => DbError::Poisoned,
    }
}

impl KvEngine for BPlusTree {
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            name: "bplus-tree-v1",
            logical_model: LogicalModel::KeyValue,
            storage_architecture: StorageArchitecture::BPlusTree,
            concurrency: ConcurrencyMode::CallerSerialized,
            persistence: Persistence::Persistent,
            crash_recovery: CrashRecovery::MirroredCopyOnWritePages,
            distribution: DistributionMode::Standalone,
            ordered_range_scan: false,
            max_key_bytes: MAX_TREE_KEY_BYTES,
            max_value_bytes: MAX_TREE_VALUE_BYTES,
        }
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> db_core::Result<Option<Vec<u8>>> {
        BPlusTree::put(self, key, value).map_err(common_error)
    }

    fn get(&mut self, key: &[u8]) -> db_core::Result<Option<Vec<u8>>> {
        BPlusTree::get(self, key).map_err(common_error)
    }

    fn delete(&mut self, key: &[u8]) -> db_core::Result<Option<Vec<u8>>> {
        BPlusTree::delete(self, key).map_err(common_error)
    }

    fn reopen(&mut self) -> db_core::Result<()> {
        let path = self.path().to_path_buf();
        match BPlusTree::open(&path, self.cache_capacity) {
            Ok(reopened) => {
                *self = reopened;
                Ok(())
            }
            Err(error) => {
                self.pager.poisoned = true;
                Err(common_error(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use db_core::{
        compare_workload, ByteString, KvEngine, Workload, WorkloadStep, MAX_KEY_BYTES,
        MAX_VALUE_BYTES, WORKLOAD_FORMAT_VERSION,
    };
    use db_storage_memory::MemoryEngine;
    use tempfile::tempdir;

    use super::{BPlusTree, MAX_TREE_KEY_BYTES, MAX_TREE_VALUE_BYTES};

    #[test]
    fn capabilities_match_the_common_point_contract() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("capabilities.db");
        let tree = BPlusTree::create_new(&path, 4).expect("create tree");
        let capabilities = tree.capabilities();

        assert_eq!(MAX_TREE_KEY_BYTES, MAX_KEY_BYTES);
        assert_eq!(MAX_TREE_VALUE_BYTES, MAX_VALUE_BYTES);
        assert_eq!(capabilities.max_key_bytes, MAX_KEY_BYTES);
        assert_eq!(capabilities.max_value_bytes, MAX_VALUE_BYTES);
        assert!(!capabilities.ordered_range_scan);
    }

    #[test]
    fn common_differential_harness_covers_limits_delete_and_reopen() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("differential.db");
        let mut reference = MemoryEngine::new();
        let mut candidate = BPlusTree::create_new(&path, 16).expect("create tree");

        let mut maximum_key = vec![0xa5; MAX_KEY_BYTES];
        maximum_key[MAX_KEY_BYTES - 1] = 0xfe;
        let maximum_value = (0..MAX_VALUE_BYTES)
            .map(|index| ((index * 29 + 7) & 0xff) as u8)
            .collect::<Vec<_>>();
        let binary_key = vec![0x00, 0xff, 0x10, 0x80];

        let workload = Workload {
            format_version: WORKLOAD_FORMAT_VERSION,
            seed: None,
            steps: vec![
                WorkloadStep::Put {
                    key: ByteString::from(Vec::new()),
                    value: ByteString::from(Vec::new()),
                },
                WorkloadStep::Put {
                    key: ByteString::from(binary_key.clone()),
                    value: ByteString::from(b"binary".to_vec()),
                },
                WorkloadStep::Put {
                    key: ByteString::from(maximum_key.clone()),
                    value: ByteString::from(maximum_value.clone()),
                },
                WorkloadStep::Get {
                    key: ByteString::from(maximum_key.clone()),
                },
                WorkloadStep::Reopen,
                WorkloadStep::Get {
                    key: ByteString::from(Vec::new()),
                },
                WorkloadStep::Put {
                    key: ByteString::from(maximum_key.clone()),
                    value: ByteString::from(b"replacement".to_vec()),
                },
                WorkloadStep::Delete {
                    key: ByteString::from(binary_key.clone()),
                },
                WorkloadStep::Delete {
                    key: ByteString::from(binary_key),
                },
                WorkloadStep::Reopen,
                WorkloadStep::Get {
                    key: ByteString::from(maximum_key.clone()),
                },
                WorkloadStep::Delete {
                    key: ByteString::from(maximum_key),
                },
                WorkloadStep::Reopen,
            ],
        };

        let report = compare_workload(&mut reference, &mut candidate, &workload)
            .expect("B+ tree matches common memory oracle");
        assert_eq!(report.steps_checked, workload.steps.len());
    }
}
''')
