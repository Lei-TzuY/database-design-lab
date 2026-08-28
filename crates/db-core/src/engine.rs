use serde::Serialize;

use crate::{ByteString, DbError, Outcome, Result, Workload, WorkloadStep};

/// Maximum key size in the first common KV semantics.
pub const MAX_KEY_BYTES: usize = 4 * 1024;
/// Maximum value size in the first common KV semantics.
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;

/// Logical contract exposed by an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalModel {
    /// Opaque binary key to opaque binary value.
    KeyValue,
}

/// Physical architecture actually implemented by an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageArchitecture {
    /// Deterministic volatile oracle, not a persistent storage candidate.
    InMemoryReference,
    /// Versioned checksummed mutation log plus replay index.
    AppendLog,
    /// Checksummed page file with a copy-on-write B+ tree and mirrored metadata.
    BPlusTree,
    /// Write-ahead log plus ordered mutable and immutable MemTables.
    LsmTree,
}

/// Concurrency contract actually enforced by the current engine boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyMode {
    /// The caller serializes access; no cross-process exclusion is provided.
    CallerSerialized,
}

/// Whether acknowledged state survives process restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Persistence {
    /// State exists only in memory.
    Volatile,
    /// State is represented in a versioned on-disk format.
    Persistent,
}

/// Crash-recovery behavior currently exposed by an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashRecovery {
    /// No persistent crash recovery is applicable.
    None,
    /// Valid records replay and a structurally valid incomplete final append is discarded.
    TruncatedFinalAppend,
    /// Durable COW pages are published by alternating checksummed root metadata copies.
    MirroredCopyOnWritePages,
    /// A versioned checksummed write-ahead log reconstructs MemTables during reopen.
    WriteAheadLogReplay,
}

/// Distribution behavior currently exposed by an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionMode {
    /// One local process owns the engine; no replication protocol exists.
    Standalone,
}

/// Explicit capabilities used to prevent accidental incomparable experiments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EngineCapabilities {
    /// Stable engine identifier.
    pub name: &'static str,
    /// Logical model implemented by the trait contract.
    pub logical_model: LogicalModel,
    /// Physical storage architecture.
    pub storage_architecture: StorageArchitecture,
    /// Concurrency contract.
    pub concurrency: ConcurrencyMode,
    /// Persistence behavior.
    pub persistence: Persistence,
    /// Crash-recovery behavior.
    pub crash_recovery: CrashRecovery,
    /// Distribution behavior.
    pub distribution: DistributionMode,
    /// Whether the public common semantics currently include ordered range scans.
    pub ordered_range_scan: bool,
    /// Maximum accepted key bytes.
    pub max_key_bytes: usize,
    /// Maximum accepted value bytes.
    pub max_value_bytes: usize,
}

/// Minimal engine contract proven by the current reference and persistent implementations.
pub trait KvEngine {
    /// Returns explicit capabilities for experiment validation.
    fn capabilities(&self) -> EngineCapabilities;

    /// Sets a key and returns the previous value, if any.
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Reads a key.
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Deletes a key and returns the previous value, if any.
    fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Returns up to `limit` key/value pairs in ascending bytewise key order from `[start, end)`.
    ///
    /// `end = None` means no upper bound. Engines whose capability advertises
    /// `ordered_range_scan = false` may reject this operation.
    fn range_scan(
        &mut self,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let _ = (start, end, limit);
        Err(DbError::InvalidInput(format!(
            "engine {} does not expose ordered range scans",
            self.capabilities().name
        )))
    }

    /// Closes and reopens engine state, replaying persistent state where applicable.
    fn reopen(&mut self) -> Result<()>;
}

/// Validates a key against the common semantics.
pub fn validate_key(key: &[u8]) -> Result<()> {
    if key.len() > MAX_KEY_BYTES {
        return Err(DbError::InvalidInput(format!(
            "key has {} bytes; maximum is {MAX_KEY_BYTES}",
            key.len()
        )));
    }
    Ok(())
}

/// Validates half-open ordered range bounds against the common key semantics.
pub fn validate_range_scan(start: &[u8], end: Option<&[u8]>) -> Result<()> {
    validate_key(start)?;
    if let Some(end) = end {
        validate_key(end)?;
        if end < start {
            return Err(DbError::InvalidInput(
                "ordered range end must not sort before start".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Validates a key/value pair against the common semantics.
pub fn validate_key_value(key: &[u8], value: &[u8]) -> Result<()> {
    validate_key(key)?;
    if value.len() > MAX_VALUE_BYTES {
        return Err(DbError::InvalidInput(format!(
            "value has {} bytes; maximum is {MAX_VALUE_BYTES}",
            value.len()
        )));
    }
    Ok(())
}

/// Executes one step using the common observable semantics.
pub fn execute_step<E: KvEngine>(engine: &mut E, step: &WorkloadStep) -> Result<Outcome> {
    match step {
        WorkloadStep::Put { key, value } => {
            engine
                .put(key.as_slice(), value.as_slice())
                .map(|previous| Outcome::Put {
                    previous: previous.map(ByteString::from),
                })
        }
        WorkloadStep::Get { key } => engine.get(key.as_slice()).map(|value| Outcome::Get {
            value: value.map(ByteString::from),
        }),
        WorkloadStep::Delete { key } => {
            engine
                .delete(key.as_slice())
                .map(|previous| Outcome::Delete {
                    previous: previous.map(ByteString::from),
                })
        }
        WorkloadStep::Reopen => {
            engine.reopen()?;
            Ok(Outcome::Reopened)
        }
    }
}

/// Executes a validated workload and returns every observable result.
pub fn execute_workload<E: KvEngine>(engine: &mut E, workload: &Workload) -> Result<Vec<Outcome>> {
    workload.validate()?;
    workload
        .steps
        .iter()
        .map(|step| execute_step(engine, step))
        .collect()
}
