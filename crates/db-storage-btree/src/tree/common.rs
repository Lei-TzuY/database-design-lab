use db_core::{
    ConcurrencyMode, CrashRecovery, DbError, DistributionMode, EngineCapabilities, KvEngine,
    LogicalModel, Persistence, StorageArchitecture,
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
