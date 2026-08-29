from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CORE_ENGINE = ROOT / "crates/db-core/src/engine.rs"
CORE_LIB = ROOT / "crates/db-core/src/lib.rs"
LSM = ROOT / "crates/db-storage-lsm/src/lib.rs"
BTREE_LIB = ROOT / "crates/db-storage-btree/src/lib.rs"
TREE = ROOT / "crates/db-storage-btree/src/tree.rs"
DELETE = ROOT / "crates/db-storage-btree/src/tree/delete.rs"
SCAN = ROOT / "crates/db-storage-btree/src/tree/scan.rs"
COMMON = ROOT / "crates/db-storage-btree/src/tree/common.rs"
TEST = ROOT / "crates/db-storage-btree/src/tree/instrumentation_tests.rs"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one marker, found {count}")
    return text.replace(old, new, 1)


# ---- db-core: common experiment contract and amplification report ----
text = CORE_ENGINE.read_text()
marker = '''pub struct EngineCapabilities {
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
'''
insert = '''pub struct EngineCapabilities {
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

/// Exact integer numerator/denominator pair used by reproducible amplification evidence.
///
/// Zero denominators are preserved instead of being converted into floating-point infinities or NaN.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct AmplificationRatio {
    /// Raw physical/structural work numerator.
    pub numerator: u64,
    /// Raw logical baseline denominator.
    pub denominator: u64,
}

/// Architecture-specific structural unit used by a read-amplification numerator.
///
/// These units deliberately prevent a page access from being silently compared as though it were the
/// same event as consulting an SSTable or decoding one SSTable version. Device-level read bytes and
/// cache misses require a separate controlled benchmark layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadWorkUnit {
    /// One logical access to a validated fixed-size B+ tree page, including cache hits.
    BtreePageAccess,
    /// One LSM SSTable considered by a point lookup before a hit/miss decision.
    LsmSstableConsult,
    /// One physical SSTable record version decoded while resolving an ordered range.
    LsmSstableVersionDecoded,
}

/// One structural read-amplification ratio plus the unit carried by its numerator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StructuralReadAmplification {
    /// Exact structural-work numerator over the logical-operation/result denominator.
    pub ratio: AmplificationRatio,
    /// Architecture-specific unit represented by `ratio.numerator`.
    pub unit: ReadWorkUnit,
}

/// Common reporting shape for hand-computable storage-engine amplification evidence.
///
/// Point/range read numerators remain explicitly architecture-specific through `ReadWorkUnit`.
/// Data-write and primary-structure ratios use bytes, but callers must still keep each engine's
/// documented accounting boundary fixed when comparing experiments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AmplificationReport {
    /// Structural work per successful explicit point GET.
    pub point_read: StructuralReadAmplification,
    /// Structural work per logical record returned by successful range scans.
    pub range_read: StructuralReadAmplification,
    /// Data-path bytes written per acknowledged logical mutation byte.
    pub data_write_bytes_per_logical_byte: AmplificationRatio,
    /// Current primary data-structure bytes retained per live logical key+value byte.
    pub primary_structure_bytes_per_live_byte: AmplificationRatio,
}

/// Common reset/report surface implemented by storage engines admitted to amplification experiments.
pub trait AmplificationInstrumented {
    /// Clears process-local measurement counters without changing database state.
    fn reset_amplification(&mut self);

    /// Returns an exact report for the current process-local measurement window and durable state.
    fn amplification_report(&mut self) -> Result<AmplificationReport>;
}

/// Rejects two engine configurations when common experiment semantics are not comparable.
///
/// Architecture and crash-recovery *mechanism* are intentionally allowed to differ: those are the
/// independent variables. Logical model, caller/concurrency contract, persistence class, distribution
/// mode, ordered-range capability, and size limits must match. `require_ordered_range` additionally
/// refuses a pair that does not expose the common half-open range API.
pub fn validate_experiment_compatibility(
    left: EngineCapabilities,
    right: EngineCapabilities,
    require_ordered_range: bool,
) -> Result<()> {
    let mut mismatches = Vec::new();
    if left.logical_model != right.logical_model {
        mismatches.push("logical_model");
    }
    if left.concurrency != right.concurrency {
        mismatches.push("concurrency");
    }
    if left.persistence != right.persistence {
        mismatches.push("persistence");
    }
    if left.distribution != right.distribution {
        mismatches.push("distribution");
    }
    if left.ordered_range_scan != right.ordered_range_scan {
        mismatches.push("ordered_range_scan");
    }
    if left.max_key_bytes != right.max_key_bytes {
        mismatches.push("max_key_bytes");
    }
    if left.max_value_bytes != right.max_value_bytes {
        mismatches.push("max_value_bytes");
    }
    if require_ordered_range && (!left.ordered_range_scan || !right.ordered_range_scan) {
        mismatches.push("required_ordered_range_scan");
    }
    if mismatches.is_empty() {
        return Ok(());
    }
    Err(DbError::InvalidInput(format!(
        "engine capabilities are not experiment-compatible: {} vs {} differ in {}",
        left.name,
        right.name,
        mismatches.join(", ")
    )))
}

/// Minimal engine contract proven by the current reference and persistent implementations.
'''
text = replace_once(text, marker, insert, "core amplification schema")
CORE_ENGINE.write_text(text)

text = CORE_LIB.read_text()
text = replace_once(
    text,
    '''    execute_step, execute_workload, validate_key, validate_key_value, validate_range_scan,
    ConcurrencyMode, CrashRecovery, DistributionMode, EngineCapabilities, KvEngine, LogicalModel,
    Persistence, StorageArchitecture, MAX_KEY_BYTES, MAX_VALUE_BYTES,
''',
    '''    execute_step, execute_workload, validate_experiment_compatibility, validate_key,
    validate_key_value, validate_range_scan, AmplificationInstrumented, AmplificationRatio,
    AmplificationReport, ConcurrencyMode, CrashRecovery, DistributionMode, EngineCapabilities,
    KvEngine, LogicalModel, Persistence, ReadWorkUnit, StorageArchitecture,
    StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
''',
    "core exports",
)
CORE_LIB.write_text(text)

# ---- LSM: migrate public report to common schema without changing raw counters ----
text = LSM.read_text()
text = replace_once(
    text,
    '''use db_core::{
    validate_key, validate_key_value, validate_range_scan, ConcurrencyMode, CrashRecovery, DbError,
    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, Persistence, Result,
    StorageArchitecture, MAX_KEY_BYTES, MAX_VALUE_BYTES,
};
''',
    '''use db_core::{
    validate_key, validate_key_value, validate_range_scan, AmplificationInstrumented,
    AmplificationReport, AmplificationRatio, ConcurrencyMode, CrashRecovery, DbError,
    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, Persistence, ReadWorkUnit, Result,
    StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
};
''',
    "lsm common imports",
)
old = '''/// Exact integer numerator/denominator pair for an amplification metric.
///
/// A zero denominator is preserved rather than converted to NaN/infinity so experiment code can
/// decide how to render an empty measurement window without losing the raw evidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct AmplificationRatio {
    /// Raw work/space numerator.
    pub numerator: u64,
    /// Raw logical baseline denominator.
    pub denominator: u64,
}

/// Reproducible amplification report derived from current state plus process-local counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LsmAmplificationReport {
    /// SSTables consulted per explicit point GET.
    pub point_read_tables_per_get: AmplificationRatio,
    /// Physical SSTable versions decoded per logical range record returned.
    pub range_versions_per_result: AmplificationRatio,
    /// WAL-record + flush-SSTable + compaction-output bytes per acknowledged logical mutation byte.
    pub data_write_bytes_per_logical_byte: AmplificationRatio,
    /// Authoritative SSTable bytes per durable live key+value byte represented by those SSTables.
    pub sorted_table_bytes_per_durable_live_byte: AmplificationRatio,
}

'''
new = '''/// Backward-compatible name for the common amplification report shape.
pub type LsmAmplificationReport = AmplificationReport;

'''
text = replace_once(text, old, new, "remove lsm-local report structs")
text = replace_once(
    text,
    '''        Ok(LsmAmplificationReport {
            point_read_tables_per_get: AmplificationRatio {
                numerator: self.instrumentation.point_sstable_consults,
                denominator: self.instrumentation.point_reads,
            },
            range_versions_per_result: AmplificationRatio {
                numerator: self.instrumentation.range_sstable_records_decoded,
                denominator: self.instrumentation.range_result_records,
            },
            data_write_bytes_per_logical_byte: AmplificationRatio {
                numerator: data_write_bytes,
                denominator: self.instrumentation.logical_mutation_bytes,
            },
            sorted_table_bytes_per_durable_live_byte: AmplificationRatio {
                numerator: authoritative_sstable_bytes,
                denominator: durable_live_bytes,
            },
        })
''',
    '''        Ok(LsmAmplificationReport {
            point_read: StructuralReadAmplification {
                ratio: AmplificationRatio {
                    numerator: self.instrumentation.point_sstable_consults,
                    denominator: self.instrumentation.point_reads,
                },
                unit: ReadWorkUnit::LsmSstableConsult,
            },
            range_read: StructuralReadAmplification {
                ratio: AmplificationRatio {
                    numerator: self.instrumentation.range_sstable_records_decoded,
                    denominator: self.instrumentation.range_result_records,
                },
                unit: ReadWorkUnit::LsmSstableVersionDecoded,
            },
            data_write_bytes_per_logical_byte: AmplificationRatio {
                numerator: data_write_bytes,
                denominator: self.instrumentation.logical_mutation_bytes,
            },
            primary_structure_bytes_per_live_byte: AmplificationRatio {
                numerator: authoritative_sstable_bytes,
                denominator: durable_live_bytes,
            },
        })
''',
    "lsm common report construction",
)
# Add common trait implementation before Layout.
text = replace_once(
    text,
    '''struct Layout {
''',
    '''impl AmplificationInstrumented for LsmEngine {
    fn reset_amplification(&mut self) {
        self.reset_instrumentation();
    }

    fn amplification_report(&mut self) -> Result<AmplificationReport> {
        LsmEngine::amplification_report(self)
    }
}

struct Layout {
''',
    "lsm amplification trait",
)
LSM.write_text(text)

# ---- Pager: process-local logical page-access and data-page-write evidence ----
text = BTREE_LIB.read_text()
text = replace_once(
    text,
    '''    cache: PageCache,
    recovered_allocation: Option<RecoveredAllocation>,
    poisoned: bool,
''',
    '''    cache: PageCache,
    recovered_allocation: Option<RecoveredAllocation>,
    read_page_calls: u64,
    data_page_bytes_written: u64,
    poisoned: bool,
''',
    "pager counter fields",
)
# two initializers share same marker
old_init = '''            cache: PageCache::new(cache_capacity),
            recovered_allocation: None,
            poisoned: false,
'''
new_init = '''            cache: PageCache::new(cache_capacity),
            recovered_allocation: None,
            read_page_calls: 0,
            data_page_bytes_written: 0,
            poisoned: false,
'''
if text.count(old_init) != 1:
    raise SystemExit(f"pager create initializer: expected 1, found {text.count(old_init)}")
text = text.replace(old_init, new_init, 1)
old_open = '''            cache: PageCache::new(cache_capacity),
            recovered_allocation,
            poisoned: false,
'''
new_open = '''            cache: PageCache::new(cache_capacity),
            recovered_allocation,
            read_page_calls: 0,
            data_page_bytes_written: 0,
            poisoned: false,
'''
text = replace_once(text, old_open, new_open, "pager open initializer")
text = replace_once(
    text,
    '''    fn write_durable_bytes(
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
''',
    '''    fn write_durable_bytes(
        &mut self,
        offset: u64,
        bytes: &[u8],
        kind: DurableWriteKind,
    ) -> Result<()> {
        #[cfg(test)]
        {
            let event_index = self.fault_trace.len();
            self.fault_trace.push(kind);
            if let Some(spec) = self.fault_spec {
                if spec.event_index == event_index {
                    match spec.mode {
                        FaultMode::BeforeWrite => {
                            return Err(injected_fault(kind, spec.mode));
                        }
                        FaultMode::TornWrite => {
''',
    "pager write kind rename start",
)
text = text.replace("return Err(injected_fault(_kind, spec.mode));", "return Err(injected_fault(kind, spec.mode));")
text = replace_once(
    text,
    '''        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(bytes)?;
        self.file.sync_data()?;
        Ok(())
    }
''',
    '''        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(bytes)?;
        self.file.sync_data()?;
        if matches!(kind, DurableWriteKind::AppendPage(_) | DurableWriteKind::RecycledPage(_)) {
            self.data_page_bytes_written = self
                .data_page_bytes_written
                .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        }
        Ok(())
    }
''',
    "pager data-page write accounting",
)
text = replace_once(
    text,
    '''    /// Reads and validates one committed data page.
    pub fn read_page(&mut self, page_id: u64) -> Result<Page> {
        self.ensure_usable()?;
        self.validate_committed_page_id(page_id)?;
        if let Some(page) = self.cache.get(page_id) {
''',
    '''    /// Process-local logical page-access count used by higher-level instrumentation snapshots.
    pub(crate) const fn read_page_calls(&self) -> u64 {
        self.read_page_calls
    }

    /// Process-local successfully synchronized data-page bytes written since this pager was opened.
    pub(crate) const fn data_page_bytes_written(&self) -> u64 {
        self.data_page_bytes_written
    }

    /// Reads and validates one committed data page.
    pub fn read_page(&mut self, page_id: u64) -> Result<Page> {
        self.ensure_usable()?;
        self.validate_committed_page_id(page_id)?;
        self.read_page_calls = self.read_page_calls.saturating_add(1);
        if let Some(page) = self.cache.get(page_id) {
''',
    "pager read access accounting",
)
text = replace_once(
    text,
    '''pub use tree::{BPlusTree, MAX_TREE_KEY_BYTES, MAX_TREE_VALUE_BYTES};
''',
    '''pub use tree::{BPlusTree, BtreeInstrumentation, MAX_TREE_KEY_BYTES, MAX_TREE_VALUE_BYTES};
''',
    "export btree instrumentation",
)
BTREE_LIB.write_text(text)

# ---- B+ tree raw counters, wrappers, report ----
text = TREE.read_text()
text = replace_once(
    text,
    '''mod common;
mod delete;
#[cfg(test)]
mod fault;
mod overflow;
mod reuse;
mod scan;
''',
    '''mod common;
mod delete;
#[cfg(test)]
mod fault;
#[cfg(test)]
mod instrumentation_tests;
mod overflow;
mod reuse;
mod scan;
''',
    "tree instrumentation test module",
)
text = replace_once(
    text,
    '''use super::{
    corruption, BtreeError, Page, PageKind, Pager, Result, CHECKSUM_OFFSET, DATA_HEADER_LEN,
    SLOT_LEN, SUPERBLOCK_COUNT,
};
''',
    '''use db_core::{
    AmplificationRatio, AmplificationReport, ReadWorkUnit, StructuralReadAmplification,
};

use super::{
    corruption, BtreeError, Page, PageKind, Pager, Result, CHECKSUM_OFFSET, DATA_HEADER_LEN, PAGE_SIZE,
    SLOT_LEN, SUPERBLOCK_COUNT,
};
''',
    "tree common amplification imports",
)
text = replace_once(
    text,
    '''/// Persistent copy-on-write B+ tree supporting binary point lookup, insertion/update, and deletion.
''',
    '''/// Process-local, resettable counters for reproducible B+ tree amplification experiments.
///
/// Read counters measure logical validated-page accesses, including cache hits. Data-write bytes count
/// synchronized leaf/internal/overflow page images and exclude mirrored superblock metadata. The
/// counters describe the implemented data path; they are not device I/O telemetry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BtreeInstrumentation {
    /// Successful explicit `GET` operations.
    pub point_reads: u64,
    /// Validated B+ tree/overflow page accesses serving explicit GETs.
    pub point_page_accesses: u64,
    /// Successful explicit range scans, including empty-result scans.
    pub range_scans: u64,
    /// Logical key/value records returned by successful range scans.
    pub range_result_records: u64,
    /// Validated B+ tree/overflow page accesses serving explicit range scans.
    pub range_page_accesses: u64,
    /// Successful acknowledged PUT/DELETE calls, including a missing-key DELETE.
    pub logical_mutations: u64,
    /// Key plus PUT-value bytes accepted by mutations; DELETE contributes key bytes only.
    pub logical_mutation_bytes: u64,
    /// Synchronized leaf/internal/overflow page bytes produced by successful mutations.
    pub data_page_bytes_written: u64,
}

/// Persistent copy-on-write B+ tree supporting binary point lookup, insertion/update, and deletion.
''',
    "btree instrumentation struct",
)
text = replace_once(
    text,
    '''pub struct BPlusTree {
    pager: Pager,
    reusable_pages: VecDeque<u64>,
    cache_capacity: usize,
}
''',
    '''pub struct BPlusTree {
    pager: Pager,
    reusable_pages: VecDeque<u64>,
    cache_capacity: usize,
    instrumentation: BtreeInstrumentation,
}
''',
    "btree instrumentation field",
)
text = replace_once(
    text,
    '''            reusable_pages: VecDeque::new(),
            cache_capacity,
        })
''',
    '''            reusable_pages: VecDeque::new(),
            cache_capacity,
            instrumentation: BtreeInstrumentation::default(),
        })
''',
    "btree create instrumentation init",
)
text = replace_once(
    text,
    '''            reusable_pages: VecDeque::new(),
            cache_capacity,
        };
''',
    '''            reusable_pages: VecDeque::new(),
            cache_capacity,
            instrumentation: BtreeInstrumentation::default(),
        };
''',
    "btree open instrumentation init",
)
# Insert public instrumentation/report methods before height.
text = replace_once(
    text,
    '''    /// Returns tree height (`0` for empty, `1` for a leaf root).
''',
    '''    /// Returns a copy of process-local B+ tree instrumentation counters.
    #[must_use]
    pub const fn instrumentation(&self) -> BtreeInstrumentation {
        self.instrumentation
    }

    /// Resets process-local amplification counters without modifying database state.
    pub fn reset_instrumentation(&mut self) {
        self.instrumentation = BtreeInstrumentation::default();
    }

    /// Builds the common exact amplification report for the current window and retained page file.
    ///
    /// The primary-structure numerator is every committed data page retained by the page file,
    /// including unreachable COW history that has not yet been recycled; the two mirrored
    /// superblocks are excluded. The live-byte denominator is reconstructed from the authoritative
    /// tree without incrementing the public read counters.
    pub fn amplification_report(&mut self) -> Result<AmplificationReport> {
        let rows = self.range_scan_uninstrumented(b"", None, usize::MAX)?;
        let live_bytes = rows.into_iter().try_fold(0_u64, |total, (key, value)| {
            let bytes = key
                .len()
                .checked_add(value.len())
                .ok_or_else(|| corruption(0, "B+ tree live logical byte count overflowed usize"))?;
            let bytes = u64::try_from(bytes)
                .map_err(|_| corruption(0, "B+ tree live logical byte count does not fit u64"))?;
            total
                .checked_add(bytes)
                .ok_or_else(|| corruption(0, "B+ tree live logical byte count overflowed u64"))
        })?;
        let primary_bytes = self
            .pager
            .data_page_count()
            .saturating_mul(u64::try_from(PAGE_SIZE).expect("page size fits u64"));
        Ok(AmplificationReport {
            point_read: StructuralReadAmplification {
                ratio: AmplificationRatio {
                    numerator: self.instrumentation.point_page_accesses,
                    denominator: self.instrumentation.point_reads,
                },
                unit: ReadWorkUnit::BtreePageAccess,
            },
            range_read: StructuralReadAmplification {
                ratio: AmplificationRatio {
                    numerator: self.instrumentation.range_page_accesses,
                    denominator: self.instrumentation.range_result_records,
                },
                unit: ReadWorkUnit::BtreePageAccess,
            },
            data_write_bytes_per_logical_byte: AmplificationRatio {
                numerator: self.instrumentation.data_page_bytes_written,
                denominator: self.instrumentation.logical_mutation_bytes,
            },
            primary_structure_bytes_per_live_byte: AmplificationRatio {
                numerator: primary_bytes,
                denominator: live_bytes,
            },
        })
    }

    pub(super) fn record_mutation(&mut self, logical_bytes: usize, data_write_before: u64) {
        self.instrumentation.logical_mutations =
            self.instrumentation.logical_mutations.saturating_add(1);
        self.instrumentation.logical_mutation_bytes = self
            .instrumentation
            .logical_mutation_bytes
            .saturating_add(u64::try_from(logical_bytes).unwrap_or(u64::MAX));
        self.instrumentation.data_page_bytes_written = self
            .instrumentation
            .data_page_bytes_written
            .saturating_add(
                self.pager
                    .data_page_bytes_written()
                    .saturating_sub(data_write_before),
            );
    }

    /// Returns tree height (`0` for empty, `1` for a leaf root).
''',
    "btree instrumentation methods",
)
# Refactor GET wrapper.
old_get = '''    /// Looks up one opaque binary key.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        let Some(mut page_id) = self.pager.root_page_id() else {
            return Ok(None);
        };
'''
new_get = '''    /// Looks up one opaque binary key and records structural page-access evidence.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let before = self.pager.read_page_calls();
        let result = self.get_uninstrumented(key);
        if result.is_ok() {
            self.instrumentation.point_reads = self.instrumentation.point_reads.saturating_add(1);
            self.instrumentation.point_page_accesses = self
                .instrumentation
                .point_page_accesses
                .saturating_add(self.pager.read_page_calls().saturating_sub(before));
        }
        result
    }

    pub(super) fn get_uninstrumented(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        let Some(mut page_id) = self.pager.root_page_id() else {
            return Ok(None);
        };
'''
text = replace_once(text, old_get, new_get, "btree get wrapper")
# PUT: use uninstrumented previous and record successful mutation.
text = replace_once(
    text,
    '''        validate_key_value(key, value)?;
        let previous = self.get(key)?;
        self.refresh_reusable_pages()?;
''',
    '''        validate_key_value(key, value)?;
        let data_write_before = self.pager.data_page_bytes_written();
        let previous = self.get_uninstrumented(key)?;
        self.refresh_reusable_pages()?;
''',
    "btree put snapshot",
)
text = replace_once(
    text,
    '''        self.pager.set_root(Some(new_root))?;
        Ok(previous)
    }
''',
    '''        self.pager.set_root(Some(new_root))?;
        self.record_mutation(key.len().saturating_add(value.len()), data_write_before);
        Ok(previous)
    }
''',
    "btree put mutation accounting",
)
TREE.write_text(text)

# ---- DELETE: suppress internal GET read accounting, count even missing delete ----
text = DELETE.read_text()
text = replace_once(
    text,
    '''        validate_key(key)?;
        let previous = self.get(key)?;
        let Some(previous_value) = previous else {
            return Ok(None);
        };
''',
    '''        validate_key(key)?;
        let data_write_before = self.pager.data_page_bytes_written();
        let previous = self.get_uninstrumented(key)?;
        let Some(previous_value) = previous else {
            self.record_mutation(key.len(), data_write_before);
            return Ok(None);
        };
''',
    "btree delete snapshot",
)
text = replace_once(
    text,
    '''        self.pager.set_root(new_root)?;
        Ok(Some(previous_value))
    }
''',
    '''        self.pager.set_root(new_root)?;
        self.record_mutation(key.len(), data_write_before);
        Ok(Some(previous_value))
    }
''',
    "btree delete mutation accounting",
)
DELETE.write_text(text)

# ---- RANGE: wrapper + uninstrumented body for report reconstruction ----
text = SCAN.read_text()
old = '''    pub fn range_scan(
        &mut self,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        validate_key(start)?;
'''
new = '''    pub fn range_scan(
        &mut self,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let before = self.pager.read_page_calls();
        let result = self.range_scan_uninstrumented(start, end, limit);
        if let Ok(rows) = &result {
            self.instrumentation.range_scans = self.instrumentation.range_scans.saturating_add(1);
            self.instrumentation.range_page_accesses = self
                .instrumentation
                .range_page_accesses
                .saturating_add(self.pager.read_page_calls().saturating_sub(before));
            self.instrumentation.range_result_records = self
                .instrumentation
                .range_result_records
                .saturating_add(u64::try_from(rows.len()).unwrap_or(u64::MAX));
        }
        result
    }

    pub(super) fn range_scan_uninstrumented(
        &mut self,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        validate_key(start)?;
'''
text = replace_once(text, old, new, "btree range wrapper")
SCAN.write_text(text)

# ---- Common trait implementation + preserve counters across logical reopen ----
text = COMMON.read_text()
text = replace_once(
    text,
    '''use db_core::{
    ConcurrencyMode, CrashRecovery, DbError, DistributionMode, EngineCapabilities, KvEngine,
    LogicalModel, Persistence, StorageArchitecture,
};
''',
    '''use db_core::{
    AmplificationInstrumented, AmplificationReport, ConcurrencyMode, CrashRecovery, DbError,
    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, Persistence, StorageArchitecture,
};
''',
    "btree common imports",
)
text = replace_once(
    text,
    '''    fn reopen(&mut self) -> db_core::Result<()> {
        let path = self.path().to_path_buf();
        match BPlusTree::open(&path, self.cache_capacity) {
            Ok(reopened) => {
                *self = reopened;
                Ok(())
            }
''',
    '''    fn reopen(&mut self) -> db_core::Result<()> {
        let path = self.path().to_path_buf();
        let instrumentation = self.instrumentation;
        match BPlusTree::open(&path, self.cache_capacity) {
            Ok(mut reopened) => {
                reopened.instrumentation = instrumentation;
                *self = reopened;
                Ok(())
            }
''',
    "btree reopen preserves instrumentation",
)
text = replace_once(
    text,
    '''#[cfg(test)]
mod tests {
''',
    '''impl AmplificationInstrumented for BPlusTree {
    fn reset_amplification(&mut self) {
        self.reset_instrumentation();
    }

    fn amplification_report(&mut self) -> db_core::Result<AmplificationReport> {
        BPlusTree::amplification_report(self).map_err(common_error)
    }
}

#[cfg(test)]
mod tests {
''',
    "btree amplification trait impl",
)
COMMON.write_text(text)

# ---- Tests: exact one-leaf accounting, overflow reads, common capability preflight ----
TEST.write_text(r'''use db_core::{
    validate_experiment_compatibility, AmplificationInstrumented, ConcurrencyMode, CrashRecovery,
    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, Persistence, ReadWorkUnit,
    StorageArchitecture,
};
use tempfile::tempdir;

use super::BPlusTree;
use crate::PAGE_SIZE;

#[test]
fn one_leaf_trace_has_hand_computable_read_write_and_space_ratios() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("one-leaf-amplification.db");
    let mut tree = BPlusTree::create_new(&path, 2).expect("create tree");
    tree.put(b"a", b"1").expect("seed a");
    tree.put(b"b", b"2").expect("seed b");
    assert_eq!(tree.data_page_count(), 2, "second COW leaf appends before reuse exists");

    tree.reset_instrumentation();
    assert_eq!(tree.put(b"a", b"xx").expect("overwrite a"), Some(b"1".to_vec()));
    assert_eq!(tree.delete(b"z").expect("missing delete"), None);
    assert_eq!(tree.get(b"a").expect("point read"), Some(b"xx".to_vec()));
    let rows = tree.range_scan(b"", None, 8).expect("full range");
    assert_eq!(rows, vec![(b"a".to_vec(), b"xx".to_vec()), (b"b".to_vec(), b"2".to_vec())]);

    let counters = tree.instrumentation();
    assert_eq!(counters.logical_mutations, 2);
    assert_eq!(counters.logical_mutation_bytes, 4, "PUT a/xx = 3 bytes; DELETE z = 1");
    assert_eq!(counters.data_page_bytes_written, PAGE_SIZE as u64, "one recycled leaf image");
    assert_eq!(counters.point_reads, 1);
    assert_eq!(counters.point_page_accesses, 1);
    assert_eq!(counters.range_scans, 1);
    assert_eq!(counters.range_page_accesses, 1);
    assert_eq!(counters.range_result_records, 2);

    let report = tree.amplification_report().expect("report");
    assert_eq!(report.point_read.unit, ReadWorkUnit::BtreePageAccess);
    assert_eq!(report.point_read.ratio.numerator, 1);
    assert_eq!(report.point_read.ratio.denominator, 1);
    assert_eq!(report.range_read.unit, ReadWorkUnit::BtreePageAccess);
    assert_eq!(report.range_read.ratio.numerator, 1);
    assert_eq!(report.range_read.ratio.denominator, 2);
    assert_eq!(report.data_write_bytes_per_logical_byte.numerator, PAGE_SIZE as u64);
    assert_eq!(report.data_write_bytes_per_logical_byte.denominator, 4);
    assert_eq!(report.primary_structure_bytes_per_live_byte.numerator, 2 * PAGE_SIZE as u64);
    assert_eq!(report.primary_structure_bytes_per_live_byte.denominator, 5, "a+xx and b+2");
    assert_eq!(tree.instrumentation(), counters, "report reconstruction must not pollute counters");

    KvEngine::reopen(&mut tree).expect("logical reopen");
    assert_eq!(tree.instrumentation(), counters, "same-handle reopen preserves measurement window");
}

#[test]
fn overflow_value_pages_are_counted_as_structural_read_work() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("overflow-read-amplification.db");
    let mut tree = BPlusTree::create_new(&path, 2).expect("create tree");
    let value = vec![0x5a; 8_192];
    tree.put(b"k", &value).expect("put overflow value");
    tree.reset_instrumentation();

    assert_eq!(tree.get(b"k").expect("get overflow"), Some(value.clone()));
    let after_get = tree.instrumentation();
    assert_eq!(after_get.point_reads, 1);
    assert_eq!(after_get.point_page_accesses, 4, "leaf plus three 4,048-byte overflow chunks");

    let rows = tree.range_scan(b"k", None, 1).expect("scan overflow");
    assert_eq!(rows, vec![(b"k".to_vec(), value)]);
    let after_scan = tree.instrumentation();
    assert_eq!(after_scan.range_scans, 1);
    assert_eq!(after_scan.range_page_accesses, 4);
    assert_eq!(after_scan.range_result_records, 1);
}

#[test]
fn phase4_preflight_allows_architecture_and_recovery_to_differ_but_not_semantics() {
    let btree = EngineCapabilities {
        name: "btree",
        logical_model: LogicalModel::KeyValue,
        storage_architecture: StorageArchitecture::BPlusTree,
        concurrency: ConcurrencyMode::CallerSerialized,
        persistence: Persistence::Persistent,
        crash_recovery: CrashRecovery::MirroredCopyOnWritePages,
        distribution: DistributionMode::Standalone,
        ordered_range_scan: true,
        max_key_bytes: 4 * 1024,
        max_value_bytes: 1024 * 1024,
    };
    let mut lsm = btree;
    lsm.name = "lsm";
    lsm.storage_architecture = StorageArchitecture::LsmTree;
    lsm.crash_recovery = CrashRecovery::WriteAheadLogReplay;
    validate_experiment_compatibility(btree, lsm, true).expect("architectures should compare");

    lsm.max_value_bytes -= 1;
    let error = validate_experiment_compatibility(btree, lsm, true)
        .expect_err("different common value bound must fail preflight");
    assert!(error.to_string().contains("max_value_bytes"));

    let mut no_range = btree;
    no_range.ordered_range_scan = false;
    assert!(validate_experiment_compatibility(btree, no_range, true).is_err());
}

#[test]
fn common_amplification_trait_uses_the_same_report_shape() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("trait-report.db");
    let mut tree = BPlusTree::create_new(&path, 2).expect("create tree");
    tree.put(b"key", b"value").expect("put");
    AmplificationInstrumented::reset_amplification(&mut tree);
    tree.get(b"key").expect("get");
    let report = AmplificationInstrumented::amplification_report(&mut tree)
        .expect("common amplification report");
    assert_eq!(report.point_read.unit, ReadWorkUnit::BtreePageAccess);
    assert_eq!(report.point_read.ratio.denominator, 1);
}
''')

print("phase4 common amplification implementation applied")
