//! Executable LSM foundation with a checksummed WAL and ordered MemTables.
//!
//! Every mutation is appended to the engine's own versioned write-ahead log and synchronized before
//! it enters the mutable MemTable or returns. A size threshold freezes the mutable table into an
//! immutable ordered table; reads search mutable then immutable tables newest-first, and reopen
//! reconstructs the same table boundaries by deterministic WAL replay. This crate does not yet have
//! SSTables, a manifest, Bloom filters, levels, compaction, or WAL truncation, so it is a correctness
//! foundation rather than a B+ tree performance-comparison candidate.

mod memtable;
mod wal;

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::ops::Bound;
use std::path::{Path, PathBuf};

use db_core::{
    validate_key, validate_key_value, validate_range_scan, ConcurrencyMode, CrashRecovery, DbError,
    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, Persistence, Result,
    StorageArchitecture, MAX_KEY_BYTES, MAX_VALUE_BYTES,
};
use serde::Serialize;

use memtable::MemTableSet;
use wal::{MutationKind, Wal, WAL_FILE_NAME};
pub use wal::{RecoveredWalTail, WalVerification};

/// Deterministic threshold used to freeze the current mutable MemTable.
pub const MUTABLE_MEMTABLE_BYTES_LIMIT: usize = 64 * 1024;

/// Current in-memory/WAL structure counts, not performance instrumentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LsmStats {
    /// Complete mutation records retained in the WAL.
    pub wal_records: u64,
    /// Latest entries (including tombstones) in the active mutable table.
    pub mutable_entries: usize,
    /// Frozen in-memory tables awaiting a future SSTable implementation.
    pub immutable_memtables: usize,
    /// Latest entries (including tombstones) across immutable tables.
    pub immutable_entries: usize,
}

/// Read-only verification result for the implemented LSM directory state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationReport {
    /// Active WAL validation summary.
    pub wal: WalVerification,
    /// MemTable structure reconstructed without changing persistent bytes.
    pub memtables: LsmStats,
}

/// Standalone WAL/MemTable engine implementing the common binary KV contract.
pub struct LsmEngine {
    path: PathBuf,
    wal: Option<Wal>,
    memtables: MemTableSet,
    poisoned: bool,
}

impl std::fmt::Debug for LsmEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LsmEngine")
            .field("path", &self.path)
            .field("stats", &self.stats().ok())
            .field(
                "recovered_tail",
                &self.wal.as_ref().and_then(Wal::recovered_tail),
            )
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

impl LsmEngine {
    /// Opens an existing engine directory or creates a new one when the path is absent.
    ///
    /// An existing directory is never initialized implicitly: missing, extra, or non-regular format
    /// files fail closed. A structurally valid incomplete final WAL record is truncated and synced.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        match fs::create_dir(&path) {
            Ok(()) => Self::initialize_new(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Self::open_existing(path),
            Err(error) => Err(error.into()),
        }
    }

    /// Creates a fresh engine directory and rejects any pre-existing path.
    pub fn create_new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        fs::create_dir(&path)?;
        Self::initialize_new(path)
    }

    fn initialize_new(path: PathBuf) -> Result<Self> {
        let wal = Wal::create_new(&path.join(WAL_FILE_NAME))?;
        Ok(Self {
            path,
            wal: Some(wal),
            memtables: MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?,
            poisoned: false,
        })
    }

    fn open_existing(path: PathBuf) -> Result<Self> {
        let wal_path = validate_layout(&path)?;
        let mut memtables = MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?;
        let wal = Wal::open(&wal_path, |mutation| {
            memtables.apply(mutation.sequence, mutation.key, mutation.value)
        })?;
        Ok(Self {
            path,
            wal: Some(wal),
            memtables,
            poisoned: false,
        })
    }

    /// Validates the current implemented layout and replays its WAL without modifying it.
    pub fn verify(path: impl AsRef<Path>) -> Result<VerificationReport> {
        let wal_path = validate_layout(path.as_ref())?;
        let mut memtables = MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?;
        let wal = Wal::verify(&wal_path, |mutation| {
            memtables.apply(mutation.sequence, mutation.key, mutation.value)
        })?;
        Ok(VerificationReport {
            memtables: LsmStats {
                wal_records: wal.record_count,
                mutable_entries: memtables.mutable_entries(),
                immutable_memtables: memtables.immutable_count(),
                immutable_entries: memtables.immutable_entries(),
            },
            wal,
        })
    }

    /// Engine directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Structurally valid incomplete WAL tail repaired by the most recent open, if any.
    #[must_use]
    pub fn recovered_tail(&self) -> Option<&RecoveredWalTail> {
        self.wal.as_ref().and_then(Wal::recovered_tail)
    }

    /// Returns structural counts without presenting them as amplification metrics.
    pub fn stats(&self) -> Result<LsmStats> {
        self.ensure_usable()?;
        let wal = self.wal.as_ref().ok_or(DbError::Poisoned)?;
        Ok(LsmStats {
            wal_records: wal.record_count(),
            mutable_entries: self.memtables.mutable_entries(),
            immutable_memtables: self.memtables.immutable_count(),
            immutable_entries: self.memtables.immutable_entries(),
        })
    }

    fn current_value(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.memtables
            .get(key)
            .and_then(|entry| entry.value.clone())
    }

    fn persist_and_apply(
        &mut self,
        kind: MutationKind,
        key: &[u8],
        value: Option<&[u8]>,
    ) -> Result<()> {
        self.ensure_usable()?;
        let append = self.wal.as_mut().ok_or(DbError::Poisoned)?.append(
            kind,
            key,
            value.unwrap_or_default(),
        );
        let sequence = match append {
            Ok(sequence) => sequence,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        if let Err(error) = self
            .memtables
            .apply(sequence, key.to_vec(), value.map(<[u8]>::to_vec))
        {
            self.poisoned = true;
            return Err(error);
        }
        Ok(())
    }

    fn ensure_usable(&self) -> Result<()> {
        if self.poisoned || self.wal.is_none() {
            Err(DbError::Poisoned)
        } else {
            Ok(())
        }
    }
}

impl KvEngine for LsmEngine {
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            name: "lsm-wal-memtable-v1",
            logical_model: LogicalModel::KeyValue,
            storage_architecture: StorageArchitecture::LsmTree,
            concurrency: ConcurrencyMode::CallerSerialized,
            persistence: Persistence::Persistent,
            crash_recovery: CrashRecovery::WriteAheadLogReplay,
            distribution: DistributionMode::Standalone,
            ordered_range_scan: true,
            max_key_bytes: MAX_KEY_BYTES,
            max_value_bytes: MAX_VALUE_BYTES,
        }
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key_value(key, value)?;
        self.ensure_usable()?;
        let previous = self.current_value(key);
        self.persist_and_apply(MutationKind::Put, key, Some(value))?;
        Ok(previous)
    }

    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        self.ensure_usable()?;
        Ok(self.current_value(key))
    }

    fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        self.ensure_usable()?;
        let previous = self.current_value(key);
        self.persist_and_apply(MutationKind::Delete, key, None)?;
        Ok(previous)
    }

    fn range_scan(
        &mut self,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        validate_range_scan(start, end)?;
        self.ensure_usable()?;
        if limit == 0 || end.is_some_and(|end| end == start) {
            return Ok(Vec::new());
        }
        let lower = Bound::Included(start.to_vec());
        let upper = end
            .map(|end| Bound::Excluded(end.to_vec()))
            .unwrap_or(Bound::Unbounded);
        Ok(self
            .memtables
            .visible_state()
            .range((lower, upper))
            .filter_map(|(key, entry)| {
                entry
                    .value
                    .as_ref()
                    .map(|value| (key.clone(), value.clone()))
            })
            .take(limit)
            .collect())
    }

    fn reopen(&mut self) -> Result<()> {
        self.wal.take();
        match Self::open_existing(self.path.clone()) {
            Ok(reopened) => {
                *self = reopened;
                Ok(())
            }
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }
}

fn validate_layout(path: &Path) -> Result<PathBuf> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(corruption("LSM engine path is not a directory"));
    }

    let expected = OsStr::new(WAL_FILE_NAME);
    let mut found_wal = false;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_name() != expected {
            return Err(corruption(format!(
                "unknown file in LSM v1 directory: {}",
                entry.file_name().to_string_lossy()
            )));
        }
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            return Err(corruption("LSM WAL path is not a regular file"));
        }
        if found_wal {
            return Err(corruption("LSM directory contains duplicate WAL entries"));
        }
        found_wal = true;
    }
    if !found_wal {
        return Err(corruption(format!(
            "LSM directory is missing required {WAL_FILE_NAME}"
        )));
    }
    Ok(path.join(WAL_FILE_NAME))
}

fn corruption(reason: impl Into<String>) -> DbError {
    DbError::Corruption {
        offset: 0,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests;
