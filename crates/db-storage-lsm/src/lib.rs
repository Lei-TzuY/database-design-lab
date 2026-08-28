//! Executable LSM engine with a checksummed WAL, ordered MemTables, and immutable SSTables.
//!
//! Mutations are synchronized to the WAL before entering the mutable MemTable. When a table freezes,
//! it is synchronously written as an indexed/checksummed SSTable, a new immutable manifest snapshot is
//! synchronized, and only then is the new version set published through one slot of mirrored `CURRENT`.
//! The WAL deliberately retains complete history in this phase; `durable_sequence` lets reopen skip the
//! prefix already represented by published SSTables. Bloom filters, WAL reclamation, levels, and
//! compaction remain later Phase 3 work.

mod manifest;
mod memtable;
mod sstable;
mod wal;

use std::collections::BTreeMap;
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

use manifest::{VersionSet, CURRENT_FILE_NAME};
use memtable::{MemTableSet, VersionedEntry};
use sstable::{file_name as sstable_file_name, SsTable};
use wal::{MutationKind, Wal, WAL_FILE_NAME};
pub use wal::{RecoveredWalTail, WalVerification};

/// Deterministic threshold used to freeze the current mutable MemTable.
pub const MUTABLE_MEMTABLE_BYTES_LIMIT: usize = 64 * 1024;

/// Current WAL/MemTable/SSTable structure counts, not performance instrumentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LsmStats {
    /// Complete mutation records retained in the WAL, including already-flushed history.
    pub wal_records: u64,
    /// Latest entries (including tombstones) in the active mutable table.
    pub mutable_entries: usize,
    /// Frozen in-memory tables not yet published as SSTables.
    pub immutable_memtables: usize,
    /// Latest entries (including tombstones) across unflushed immutable tables.
    pub immutable_entries: usize,
    /// Immutable sorted tables referenced by the authoritative manifest.
    pub sstables: usize,
    /// Total indexed entries (including tombstones) across authoritative SSTables.
    pub sstable_entries: u64,
    /// Highest WAL sequence represented by the authoritative SSTable version set.
    pub durable_sequence: u64,
}

/// Read-only verification result for the implemented LSM directory state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationReport {
    /// Active WAL validation summary.
    pub wal: WalVerification,
    /// Reconstructed memory/disk structure counts.
    pub memtables: LsmStats,
}

/// Standalone LSM engine implementing the common binary KV contract.
pub struct LsmEngine {
    path: PathBuf,
    wal: Option<Wal>,
    memtables: MemTableSet,
    tables: Vec<SsTable>,
    version: VersionSet,
    next_table_id: u64,
    next_manifest_id: u64,
    poisoned: bool,
}

impl std::fmt::Debug for LsmEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LsmEngine")
            .field("path", &self.path)
            .field("stats", &self.stats().ok())
            .field("manifest_id", &self.version.manifest_id)
            .field("current_generation", &self.version.current_generation)
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
    /// Existing directories are never initialized implicitly. Unknown directory entries fail closed;
    /// canonical orphan SSTables/manifests left before `CURRENT` publication are tolerated and ignored.
    /// A structurally valid incomplete final WAL record is truncated and synchronized.
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
        let version = manifest::create_initial(&path)?;
        Ok(Self {
            path,
            wal: Some(wal),
            memtables: MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?,
            tables: Vec::new(),
            version,
            next_table_id: 1,
            next_manifest_id: 2,
            poisoned: false,
        })
    }

    fn open_existing(path: PathBuf) -> Result<Self> {
        let layout = validate_layout(&path)?;
        let version = if layout.has_version_set {
            manifest::load(&path)?
        } else {
            VersionSet {
                current_generation: 0,
                manifest_id: 0,
                durable_sequence: 0,
                tables: Vec::new(),
            }
        };
        let mut tables = Vec::with_capacity(version.tables.len());
        for descriptor in &version.tables {
            tables.push(SsTable::open(
                &path.join(sstable_file_name(descriptor.table_id)),
                descriptor.clone(),
            )?);
        }

        let mut memtables = MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?;
        let durable_sequence = version.durable_sequence;
        let wal = Wal::open(&layout.wal_path, |mutation| {
            if mutation.sequence > durable_sequence {
                memtables.apply(mutation.sequence, mutation.key, mutation.value)?;
            }
            Ok(())
        })?;
        if durable_sequence > wal.record_count() {
            return Err(corruption(format!(
                "manifest durable sequence {durable_sequence} exceeds WAL record count {}",
                wal.record_count()
            )));
        }

        Ok(Self {
            path,
            wal: Some(wal),
            memtables,
            tables,
            next_table_id: checked_next_id(layout.max_table_id, "SSTable")?,
            next_manifest_id: checked_next_id(layout.max_manifest_id, "manifest")?,
            version,
            poisoned: false,
        })
    }

    /// Validates the authoritative CURRENT/manifest/SSTable set and WAL without modifying bytes.
    pub fn verify(path: impl AsRef<Path>) -> Result<VerificationReport> {
        let path = path.as_ref();
        let layout = validate_layout(path)?;
        let version = if layout.has_version_set {
            manifest::load(path)?
        } else {
            VersionSet {
                current_generation: 0,
                manifest_id: 0,
                durable_sequence: 0,
                tables: Vec::new(),
            }
        };
        let mut sstable_entries = 0_u64;
        for descriptor in &version.tables {
            SsTable::open(
                &path.join(sstable_file_name(descriptor.table_id)),
                descriptor.clone(),
            )?;
            sstable_entries = sstable_entries
                .checked_add(descriptor.entry_count)
                .ok_or_else(|| corruption("SSTable verification entry count overflowed u64"))?;
        }

        let mut memtables = MemTableSet::new(MUTABLE_MEMTABLE_BYTES_LIMIT)?;
        let durable_sequence = version.durable_sequence;
        let wal = Wal::verify(&layout.wal_path, |mutation| {
            if mutation.sequence > durable_sequence {
                memtables.apply(mutation.sequence, mutation.key, mutation.value)?;
            }
            Ok(())
        })?;
        if durable_sequence >= wal.next_sequence {
            return Err(corruption(format!(
                "manifest durable sequence {durable_sequence} is not below WAL next sequence {}",
                wal.next_sequence
            )));
        }
        Ok(VerificationReport {
            memtables: LsmStats {
                wal_records: wal.record_count,
                mutable_entries: memtables.mutable_entries(),
                immutable_memtables: memtables.immutable_count(),
                immutable_entries: memtables.immutable_entries(),
                sstables: version.tables.len(),
                sstable_entries,
                durable_sequence,
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
        let sstable_entries = self.version.tables.iter().try_fold(0_u64, |total, table| {
            total
                .checked_add(table.entry_count)
                .ok_or_else(|| corruption("SSTable entry count overflowed u64"))
        })?;
        Ok(LsmStats {
            wal_records: wal.record_count(),
            mutable_entries: self.memtables.mutable_entries(),
            immutable_memtables: self.memtables.immutable_count(),
            immutable_entries: self.memtables.immutable_entries(),
            sstables: self.tables.len(),
            sstable_entries,
            durable_sequence: self.version.durable_sequence,
        })
    }

    fn current_entry(&self, key: &[u8]) -> Result<Option<VersionedEntry>> {
        if let Some(entry) = self.memtables.get(key) {
            return Ok(Some(entry.clone()));
        }
        for table in self.tables.iter().rev() {
            if let Some(entry) = table.get(key)? {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    fn current_value(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.current_entry(key)?.and_then(|entry| entry.value))
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
        if let Err(error) = self.flush_frozen_memtables() {
            self.poisoned = true;
            return Err(error);
        }
        Ok(())
    }

    fn flush_frozen_memtables(&mut self) -> Result<()> {
        while let Some((entries, durable_sequence)) = self.memtables.oldest_immutable_snapshot()? {
            if self.version.manifest_id == 0 {
                self.version = manifest::create_initial(&self.path)?;
                self.next_manifest_id = 2;
            }
            let table_id = self.next_table_id;
            let manifest_id = self.next_manifest_id;
            let next_table_id = table_id
                .checked_add(1)
                .ok_or_else(|| corruption("SSTable id exhausted"))?;
            let next_manifest_id = manifest_id
                .checked_add(1)
                .ok_or_else(|| corruption("manifest id exhausted"))?;

            let table = SsTable::create_new(&self.path, table_id, durable_sequence, &entries)?;
            let mut descriptors = self.version.tables.clone();
            descriptors.push(table.descriptor().clone());
            let next_version = manifest::install(
                &self.path,
                &self.version,
                manifest_id,
                durable_sequence,
                descriptors,
            )?;

            self.tables.push(table);
            self.version = next_version;
            self.next_table_id = next_table_id;
            self.next_manifest_id = next_manifest_id;
            self.memtables.retire_oldest_immutable()?;
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
            name: "lsm-sstable-manifest-v1",
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
        let previous = self.current_value(key)?;
        self.persist_and_apply(MutationKind::Put, key, Some(value))?;
        Ok(previous)
    }

    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        self.ensure_usable()?;
        self.current_value(key)
    }

    fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        self.ensure_usable()?;
        let previous = self.current_value(key)?;
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

        let mut visible = BTreeMap::new();
        for table in &self.tables {
            table.overlay_range(start, end, &mut visible)?;
        }
        let lower = Bound::Included(start.to_vec());
        let upper = end
            .map(|end| Bound::Excluded(end.to_vec()))
            .unwrap_or(Bound::Unbounded);
        for (key, entry) in self.memtables.visible_state().range((lower, upper)) {
            let replace = visible
                .get(key.as_slice())
                .is_none_or(|current: &VersionedEntry| entry.sequence > current.sequence);
            if replace {
                visible.insert(key.clone(), entry.clone());
            }
        }
        Ok(visible
            .into_iter()
            .filter_map(|(key, entry)| entry.value.map(|value| (key, value)))
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

struct Layout {
    wal_path: PathBuf,
    max_table_id: u64,
    max_manifest_id: u64,
    has_version_set: bool,
}

fn validate_layout(path: &Path) -> Result<Layout> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(corruption("LSM engine path is not a directory"));
    }

    let wal_name = OsStr::new(WAL_FILE_NAME);
    let current_name = OsStr::new(CURRENT_FILE_NAME);
    let mut found_wal = false;
    let mut found_current = false;
    let mut max_table_id = 0_u64;
    let mut max_manifest_id = 0_u64;

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            return Err(corruption(format!(
                "LSM v1 directory entry is not a regular file: {}",
                entry.file_name().to_string_lossy()
            )));
        }
        let name = entry.file_name();
        if name == wal_name {
            if found_wal {
                return Err(corruption("LSM directory contains duplicate WAL entries"));
            }
            found_wal = true;
            continue;
        }
        if name == current_name {
            if found_current {
                return Err(corruption(
                    "LSM directory contains duplicate CURRENT entries",
                ));
            }
            found_current = true;
            continue;
        }
        let text = name.to_string_lossy();
        if let Some(id) = parse_numbered_name(&text, "MANIFEST-", "") {
            max_manifest_id = max_manifest_id.max(id);
            continue;
        }
        if let Some(id) = parse_numbered_name(&text, "sst-", ".sst") {
            max_table_id = max_table_id.max(id);
            continue;
        }
        return Err(corruption(format!(
            "unknown file in LSM v1 directory: {text}"
        )));
    }

    if !found_wal {
        return Err(corruption(format!(
            "LSM directory is missing required {WAL_FILE_NAME}"
        )));
    }
    let has_version_set = match (found_current, max_manifest_id != 0) {
        (true, true) => true,
        (false, false) if max_table_id == 0 => false,
        (false, false) => {
            return Err(corruption(
                "WAL-only legacy layout cannot contain SSTable files",
            ));
        }
        (false, true) => {
            return Err(corruption(
                "LSM directory has manifest snapshots but is missing CURRENT",
            ));
        }
        (true, false) => {
            return Err(corruption(
                "LSM directory has CURRENT but no manifest snapshot",
            ));
        }
    };

    Ok(Layout {
        wal_path: path.join(WAL_FILE_NAME),
        max_table_id,
        max_manifest_id,
        has_version_set,
    })
}

fn parse_numbered_name(name: &str, prefix: &str, suffix: &str) -> Option<u64> {
    let digits = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if digits.len() != 16 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = digits.parse::<u64>().ok()?;
    (value != 0).then_some(value)
}

fn checked_next_id(max_id: u64, label: &str) -> Result<u64> {
    max_id
        .checked_add(1)
        .ok_or_else(|| corruption(format!("{label} id space exhausted")))
}

fn corruption(reason: impl Into<String>) -> DbError {
    DbError::Corruption {
        offset: 0,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests;
