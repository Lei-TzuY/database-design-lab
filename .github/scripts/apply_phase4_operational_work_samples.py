from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENGINE = ROOT / "crates/db-core/src/engine.rs"
CORE_LIB = ROOT / "crates/db-core/src/lib.rs"
EXPERIMENT = ROOT / "crates/db-core/src/experiment.rs"
BTREE_TREE = ROOT / "crates/db-storage-btree/src/tree.rs"
BTREE_COMMON = ROOT / "crates/db-storage-btree/src/tree/common.rs"
BTREE_TESTS = ROOT / "crates/db-storage-btree/src/tree/instrumentation_tests.rs"
LSM = ROOT / "crates/db-storage-lsm/src/lib.rs"
LSM_WAL = ROOT / "crates/db-storage-lsm/src/wal.rs"
LSM_TESTS = ROOT / "crates/db-storage-lsm/src/instrumentation_tests.rs"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one marker, found {count}")
    return text.replace(old, new, 1)


# db-core operational evidence schema.
text = ENGINE.read_text()
old = '''/// Raw process-local duration samples for synchronous recovery and compaction stalls.
///
/// Samples are integer nanoseconds measured with `std::time::Instant`. They are evidence to archive,
/// not a performance claim: host, filesystem, cache state, build profile, and scheduler must be pinned
/// before durations are compared across engines or revisions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OperationalTimingReport {
    /// Successful same-handle `REOPEN` durations in nanoseconds.
    pub reopen_ns: Vec<u64>,
    /// Successful synchronous compaction-path durations in nanoseconds. Empty for engines without compaction.
    pub compaction_stall_ns: Vec<u64>,
}

/// Reset/report surface for raw operational timing samples collected during an experiment window.
pub trait OperationalTimingInstrumented {
    /// Clears process-local duration samples without changing database state.
    fn reset_operational_timing(&mut self);

    /// Returns a clone of the raw timing samples accumulated in the current window.
    fn operational_timing_report(&self) -> OperationalTimingReport;
}
'''
new = '''/// Architecture-specific unit paired with one synchronous operational timing sample.
///
/// Like `ReadWorkUnit`, these are deterministic engine-level work units rather than device I/O events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalWorkUnit {
    /// One logical validated B+ tree data-page access during reopen validation/reuse discovery.
    BtreePageAccess,
    /// One LSM persisted record version examined while reopening WAL/SSTable state.
    LsmRecordVersion,
    /// One authoritative SSTable record version consumed by a full-set compaction.
    LsmSstableRecordVersion,
}

/// Deterministic data-path work associated with one operational timing sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OperationalWork {
    /// Architecture-specific logical work unit.
    pub unit: OperationalWorkUnit,
    /// Number of logical units examined by the operation.
    pub units_examined: u64,
    /// Data-path bytes represented by those units under the engine's documented accounting boundary.
    pub bytes_examined: u64,
}

/// One successful synchronous operation sample associated with an experiment step when available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OperationalTimingSample {
    /// Zero-based measured experiment step that triggered this sample, or `None` outside a measured runner.
    pub measured_step_index: Option<u64>,
    /// Wall-clock duration measured with `std::time::Instant`.
    pub duration_ns: u64,
    /// Deterministic data-path work completed by the timed operation.
    pub work: OperationalWork,
}

/// Raw process-local successful recovery and compaction-stall samples.
///
/// Duration plus deterministic work is evidence to archive, not a performance claim: failed/excluded attempts,
/// execution-order counterbalancing, cache/filesystem protocol, host pinning, and scheduler/device controls remain
/// required before durations are compared across engines or revisions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OperationalTimingReport {
    /// Successful same-handle `REOPEN` samples.
    pub reopen_samples: Vec<OperationalTimingSample>,
    /// Successful synchronous compaction-path samples. Empty for engines without compaction.
    pub compaction_stall_samples: Vec<OperationalTimingSample>,
}

/// Reset/context/report surface for operational samples collected during an experiment window.
pub trait OperationalTimingInstrumented {
    /// Clears process-local operational samples without changing database state.
    fn reset_operational_timing(&mut self);

    /// Associates subsequently emitted operational samples with one measured experiment step.
    ///
    /// The experiment runner sets this immediately before a measured action and clears it immediately after.
    fn set_operational_step_index(&mut self, step_index: Option<u64>);

    /// Returns a clone of the raw operational samples accumulated in the current window.
    fn operational_timing_report(&self) -> OperationalTimingReport;
}
'''
text = replace_once(text, old, new, "common operational evidence schema")
ENGINE.write_text(text)

text = CORE_LIB.read_text()
text = replace_once(
    text,
    '''    KvEngine, LogicalModel, OperationalTimingInstrumented, OperationalTimingReport, Persistence,
    ReadWorkUnit, StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
''',
    '''    KvEngine, LogicalModel, OperationalTimingInstrumented, OperationalTimingReport,
    OperationalTimingSample, OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit,
    StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
''',
    "core operational exports",
)
CORE_LIB.write_text(text)

# Runner associates emitted samples with exact measured-step indices and clears context even on errors.
text = EXPERIMENT.read_text()
text = replace_once(
    text,
    '''    AmplificationInstrumented, AmplificationReport, ByteString, DbError, EngineCapabilities,
    KvEngine, OperationalTimingInstrumented, OperationalTimingReport, Result, MAX_VALUE_BYTES,
''',
    '''    AmplificationInstrumented, AmplificationReport, ByteString, DbError, EngineCapabilities,
    KvEngine, OperationalTimingInstrumented, OperationalTimingReport, Result, MAX_VALUE_BYTES,
''',
    "experiment import stability",
)
# run_experiment_trace measured loop
old = '''    let mut outcome_bytes = 0_u64;
    let mut outcomes = Vec::with_capacity(trace.measured_steps.len());
    for step in &trace.measured_steps {
        let outcome = execute_experiment_step(engine, step)?;
        outcome_bytes =
            checked_add_outcome_payload(outcome_bytes, &outcome, "experiment measured outcomes")?;
        outcomes.push(outcome);
    }
'''
new = '''    let mut outcome_bytes = 0_u64;
    let mut outcomes = Vec::with_capacity(trace.measured_steps.len());
    for (index, step) in trace.measured_steps.iter().enumerate() {
        let outcome = execute_measured_experiment_step(engine, step, index)?;
        outcome_bytes =
            checked_add_outcome_payload(outcome_bytes, &outcome, "experiment measured outcomes")?;
        outcomes.push(outcome);
    }
'''
text = replace_once(text, old, new, "single runner measured context")
# comparison measured loop
old = '''    for (index, step) in trace.measured_steps.iter().enumerate() {
        let left_outcome = execute_experiment_step(left, step)?;
        let right_outcome = execute_experiment_step(right, step)?;
        if left_outcome != right_outcome {
'''
new = '''    for (index, step) in trace.measured_steps.iter().enumerate() {
        let left_outcome = execute_measured_experiment_step(left, step, index)?;
        let right_outcome = execute_measured_experiment_step(right, step, index)?;
        if left_outcome != right_outcome {
'''
text = replace_once(text, old, new, "comparison measured context")
# helper before logical_mismatch
text = replace_once(
    text,
    '''fn logical_mismatch(
''',
    '''fn execute_measured_experiment_step<E>(
    engine: &mut E,
    step: &ExperimentStep,
    index: usize,
) -> Result<ExperimentOutcome>
where
    E: KvEngine + OperationalTimingInstrumented,
{
    let index = u64::try_from(index).map_err(|_| {
        DbError::InvalidInput("measured experiment step index does not fit u64".to_owned())
    })?;
    engine.set_operational_step_index(Some(index));
    let result = execute_experiment_step(engine, step);
    engine.set_operational_step_index(None);
    result
}

fn logical_mismatch(
''',
    "measured step helper",
)
# test imports
text = replace_once(
    text,
    '''        CrashRecovery, DistributionMode, EngineCapabilities, KvEngine, LogicalModel,
        OperationalTimingInstrumented, OperationalTimingReport, Persistence, ReadWorkUnit, Result,
        StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
''',
    '''        CrashRecovery, DistributionMode, EngineCapabilities, KvEngine, LogicalModel,
        OperationalTimingInstrumented, OperationalTimingReport, OperationalTimingSample,
        OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit, Result, StorageArchitecture,
        StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
''',
    "experiment test operational imports",
)
# add assertions in shared runner after timing reset call assertions
needle = '''        assert_eq!(left.timing_reset_calls, 1);
        assert_eq!(right.timing_reset_calls, 1);
'''
replacement = needle + '''        let expected_reopen_indices: Vec<_> = trace
            .measured_steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| {
                matches!(step, ExperimentStep::Reopen).then_some(index as u64)
            })
            .collect();
        assert_eq!(
            report
                .left
                .operational_timing
                .reopen_samples
                .iter()
                .map(|sample| sample.measured_step_index.expect("measured sample index"))
                .collect::<Vec<_>>(),
            expected_reopen_indices
        );
        assert_eq!(
            report
                .right
                .operational_timing
                .reopen_samples
                .iter()
                .map(|sample| sample.measured_step_index.expect("measured sample index"))
                .collect::<Vec<_>>(),
            expected_reopen_indices
        );
'''
text = replace_once(text, needle, replacement, "runner association assertions")
# FakeEngine fields
text = replace_once(
    text,
    '''        reset_calls: u64,
        timing_reset_calls: u64,
''',
    '''        reset_calls: u64,
        timing_reset_calls: u64,
        operational_step_index: Option<u64>,
        operational_timing: OperationalTimingReport,
''',
    "fake timing fields",
)
text = replace_once(
    text,
    '''                reset_calls: 0,
                timing_reset_calls: 0,
''',
    '''                reset_calls: 0,
                timing_reset_calls: 0,
                operational_step_index: None,
                operational_timing: OperationalTimingReport::default(),
''',
    "fake timing init",
)
# Fake reopen
text = replace_once(
    text,
    '''        fn reopen(&mut self) -> Result<()> {
            Ok(())
        }
''',
    '''        fn reopen(&mut self) -> Result<()> {
            self.operational_timing.reopen_samples.push(OperationalTimingSample {
                measured_step_index: self.operational_step_index,
                duration_ns: 1,
                work: OperationalWork {
                    unit: if self.architecture == StorageArchitecture::BPlusTree {
                        OperationalWorkUnit::BtreePageAccess
                    } else {
                        OperationalWorkUnit::LsmRecordVersion
                    },
                    units_examined: 1,
                    bytes_examined: 1,
                },
            });
            Ok(())
        }
''',
    "fake reopen sample",
)
# Fake timing trait
old = '''    impl OperationalTimingInstrumented for FakeEngine {
        fn reset_operational_timing(&mut self) {
            self.timing_reset_calls = self.timing_reset_calls.saturating_add(1);
        }

        fn operational_timing_report(&self) -> OperationalTimingReport {
            OperationalTimingReport::default()
        }
    }
'''
new = '''    impl OperationalTimingInstrumented for FakeEngine {
        fn reset_operational_timing(&mut self) {
            self.timing_reset_calls = self.timing_reset_calls.saturating_add(1);
            self.operational_step_index = None;
            self.operational_timing = OperationalTimingReport::default();
        }

        fn set_operational_step_index(&mut self, step_index: Option<u64>) {
            self.operational_step_index = step_index;
        }

        fn operational_timing_report(&self) -> OperationalTimingReport {
            self.operational_timing.clone()
        }
    }
'''
text = replace_once(text, old, new, "fake timing trait")
EXPERIMENT.write_text(text)

# B+ tree: retain measured-step context across reopen and attach deterministic reopen work.
text = BTREE_TREE.read_text()
text = replace_once(
    text,
    '''    instrumentation: BtreeInstrumentation,
    operational_timing: OperationalTimingReport,
''',
    '''    instrumentation: BtreeInstrumentation,
    operational_timing: OperationalTimingReport,
    operational_step_index: Option<u64>,
''',
    "btree step field",
)
# two constructors
text = text.replace(
    '''            instrumentation: BtreeInstrumentation::default(),
            operational_timing: OperationalTimingReport::default(),
''',
    '''            instrumentation: BtreeInstrumentation::default(),
            operational_timing: OperationalTimingReport::default(),
            operational_step_index: None,
''',
)
if text.count("operational_step_index: None,") != 2:
    raise SystemExit("btree constructors: expected two operational step initializers")
BTREE_TREE.write_text(text)

text = BTREE_COMMON.read_text()
text = replace_once(
    text,
    '''    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalTimingReport, Persistence, StorageArchitecture,
};
''',
    '''    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalTimingReport, OperationalTimingSample, OperationalWork, OperationalWorkUnit, Persistence,
    StorageArchitecture,
};
''',
    "btree operational imports",
)
text = replace_once(
    text,
    '''use crate::BtreeError;
''',
    '''use crate::{BtreeError, PAGE_SIZE};
''',
    "btree page size import",
)
old = '''    fn reopen(&mut self) -> db_core::Result<()> {
        let path = self.path().to_path_buf();
        let instrumentation = self.instrumentation;
        let mut operational_timing = self.operational_timing.clone();
        let started = Instant::now();
        match BPlusTree::open(&path, self.cache_capacity) {
            Ok(mut reopened) => {
                operational_timing
                    .reopen_ns
                    .push(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
                reopened.instrumentation = instrumentation;
                reopened.operational_timing = operational_timing;
                *self = reopened;
                Ok(())
            }
'''
new = '''    fn reopen(&mut self) -> db_core::Result<()> {
        let path = self.path().to_path_buf();
        let instrumentation = self.instrumentation;
        let mut operational_timing = self.operational_timing.clone();
        let operational_step_index = self.operational_step_index;
        let started = Instant::now();
        match BPlusTree::open(&path, self.cache_capacity) {
            Ok(mut reopened) => {
                let page_accesses = reopened.pager.read_page_calls();
                operational_timing.reopen_samples.push(OperationalTimingSample {
                    measured_step_index: operational_step_index,
                    duration_ns: u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                    work: OperationalWork {
                        unit: OperationalWorkUnit::BtreePageAccess,
                        units_examined: page_accesses,
                        bytes_examined: page_accesses.saturating_mul(PAGE_SIZE as u64),
                    },
                });
                reopened.instrumentation = instrumentation;
                reopened.operational_timing = operational_timing;
                reopened.operational_step_index = operational_step_index;
                *self = reopened;
                Ok(())
            }
'''
text = replace_once(text, old, new, "btree reopen work sample")
old = '''impl OperationalTimingInstrumented for BPlusTree {
    fn reset_operational_timing(&mut self) {
        self.operational_timing = OperationalTimingReport::default();
    }

    fn operational_timing_report(&self) -> OperationalTimingReport {
        self.operational_timing.clone()
    }
}
'''
new = '''impl OperationalTimingInstrumented for BPlusTree {
    fn reset_operational_timing(&mut self) {
        self.operational_timing = OperationalTimingReport::default();
        self.operational_step_index = None;
    }

    fn set_operational_step_index(&mut self, step_index: Option<u64>) {
        self.operational_step_index = step_index;
    }

    fn operational_timing_report(&self) -> OperationalTimingReport {
        self.operational_timing.clone()
    }
}
'''
text = replace_once(text, old, new, "btree timing trait")
BTREE_COMMON.write_text(text)

text = BTREE_TESTS.read_text()
text = replace_once(
    text,
    '''    validate_experiment_compatibility, AmplificationInstrumented, ConcurrencyMode, CrashRecovery,
    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, Persistence, ReadWorkUnit,
    StorageArchitecture,
''',
    '''    validate_experiment_compatibility, AmplificationInstrumented, ConcurrencyMode, CrashRecovery,
    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalWorkUnit, Persistence, ReadWorkUnit, StorageArchitecture,
''',
    "btree timing test imports",
)
text += r'''

#[test]
fn reopen_sample_is_step_associated_and_counts_open_validation_work() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("reopen-work.db");
    let mut tree = BPlusTree::create_new(&path, 4).expect("create tree");
    tree.put(b"key", b"value").expect("put");
    tree.reset_operational_timing();
    tree.set_operational_step_index(Some(7));
    KvEngine::reopen(&mut tree).expect("reopen");

    let timing = tree.operational_timing_report();
    assert_eq!(timing.compaction_stall_samples, Vec::new());
    assert_eq!(timing.reopen_samples.len(), 1);
    let sample = timing.reopen_samples[0];
    assert_eq!(sample.measured_step_index, Some(7));
    assert_eq!(sample.work.unit, OperationalWorkUnit::BtreePageAccess);
    assert_eq!(sample.work.units_examined, 2, "open validates the root for tree integrity and reuse discovery");
    assert_eq!(sample.work.bytes_examined, 2 * PAGE_SIZE as u64);
    assert!(sample.duration_ns > 0);
}
'''
BTREE_TESTS.write_text(text)

# LSM WAL remembers the original bytes scanned during the most recent open, before recoverable-tail repair.
text = LSM_WAL.read_text()
text = replace_once(
    text,
    '''    record_count: u64,
    recovered_tail: Option<RecoveredWalTail>,
''',
    '''    record_count: u64,
    open_examined_bytes: u64,
    recovered_tail: Option<RecoveredWalTail>,
''',
    "wal examined bytes field",
)
text = replace_once(
    text,
    '''            next_sequence: first_sequence,
            record_count: 0,
            recovered_tail: None,
''',
    '''            next_sequence: first_sequence,
            record_count: 0,
            open_examined_bytes: WAL_HEADER_LEN_U64,
            recovered_tail: None,
''',
    "wal create examined bytes",
)
old = '''        if scan.recoverable_tail.is_some() {
            file.set_len(scan.valid_bytes)?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            wal_id: expected_wal_id,
            first_sequence: expected_first_sequence,
            next_sequence: scan.next_sequence,
            record_count: scan.record_count,
            recovered_tail: scan.recoverable_tail,
        })
'''
new = '''        if scan.recoverable_tail.is_some() {
            file.set_len(scan.valid_bytes)?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            wal_id: expected_wal_id,
            first_sequence: expected_first_sequence,
            next_sequence: scan.next_sequence,
            record_count: scan.record_count,
            open_examined_bytes: scan.file_bytes,
            recovered_tail: scan.recoverable_tail,
        })
'''
text = replace_once(text, old, new, "wal open examined bytes")
text = replace_once(
    text,
    '''    pub(super) const fn record_count(&self) -> u64 {
        self.record_count
    }
''',
    '''    pub(super) const fn record_count(&self) -> u64 {
        self.record_count
    }

    pub(super) const fn open_examined_bytes(&self) -> u64 {
        self.open_examined_bytes
    }
''',
    "wal examined bytes getter",
)
LSM_WAL.write_text(text)

# LSM: deterministic work for reopen and compaction plus measured-step association.
text = LSM.read_text()
text = replace_once(
    text,
    '''    EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalTimingReport, Persistence, ReadWorkUnit, Result, StorageArchitecture,
''',
    '''    EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalTimingReport, OperationalTimingSample, OperationalWork, OperationalWorkUnit, Persistence,
    ReadWorkUnit, Result, StorageArchitecture,
''',
    "lsm operational imports",
)
text = replace_once(
    text,
    '''    instrumentation: LsmInstrumentation,
    operational_timing: OperationalTimingReport,
    poisoned: bool,
''',
    '''    instrumentation: LsmInstrumentation,
    operational_timing: OperationalTimingReport,
    operational_step_index: Option<u64>,
    poisoned: bool,
''',
    "lsm operational step field",
)
# constructors are two exact occurrences
text = text.replace(
    '''            instrumentation: LsmInstrumentation::default(),
            operational_timing: OperationalTimingReport::default(),
            poisoned: false,
''',
    '''            instrumentation: LsmInstrumentation::default(),
            operational_timing: OperationalTimingReport::default(),
            operational_step_index: None,
            poisoned: false,
''',
)
if text.count("operational_step_index: None,") != 2:
    raise SystemExit("lsm constructors: expected two operational step initializers")
# compaction input records and sample
text = replace_once(
    text,
    '''        let input_bytes = self
            .version
            .tables
            .iter()
            .fold(0_u64, |total, table| total.saturating_add(table.file_bytes));
''',
    '''        let input_bytes = self
            .version
            .tables
            .iter()
            .fold(0_u64, |total, table| total.saturating_add(table.file_bytes));
        let input_records = self
            .version
            .tables
            .iter()
            .fold(0_u64, |total, table| total.saturating_add(table.entry_count));
''',
    "compaction input records",
)
text = replace_once(
    text,
    '''        self.operational_timing
            .compaction_stall_ns
            .push(u64::try_from(compaction_started.elapsed().as_nanos()).unwrap_or(u64::MAX));
        Ok(())
''',
    '''        self.operational_timing
            .compaction_stall_samples
            .push(OperationalTimingSample {
                measured_step_index: self.operational_step_index,
                duration_ns: u64::try_from(compaction_started.elapsed().as_nanos())
                    .unwrap_or(u64::MAX),
                work: OperationalWork {
                    unit: OperationalWorkUnit::LsmSstableRecordVersion,
                    units_examined: input_records,
                    bytes_examined: input_bytes,
                },
            });
        Ok(())
''',
    "compaction work sample",
)
# add pure recovery-work helper before maybe_rotate_wal
text = replace_once(
    text,
    '''    fn maybe_rotate_wal(&mut self) -> Result<()> {
''',
    '''    fn reopen_work(&self) -> Result<OperationalWork> {
        let wal = self.wal.as_ref().ok_or(DbError::Poisoned)?;
        let sstable_records = self
            .version
            .tables
            .iter()
            .fold(0_u64, |total, table| total.saturating_add(table.entry_count));
        let sstable_bytes = self
            .version
            .tables
            .iter()
            .fold(0_u64, |total, table| total.saturating_add(table.file_bytes));
        Ok(OperationalWork {
            unit: OperationalWorkUnit::LsmRecordVersion,
            units_examined: wal.record_count().saturating_add(sstable_records),
            bytes_examined: wal.open_examined_bytes().saturating_add(sstable_bytes),
        })
    }

    fn maybe_rotate_wal(&mut self) -> Result<()> {
''',
    "lsm reopen work helper",
)
# reopen
old = '''    fn reopen(&mut self) -> Result<()> {
        let instrumentation = self.instrumentation;
        let mut operational_timing = self.operational_timing.clone();
        let started = Instant::now();
        self.wal.take();
        match Self::open_existing(self.path.clone()) {
            Ok(mut reopened) => {
                operational_timing
                    .reopen_ns
                    .push(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
                reopened.instrumentation = instrumentation;
                reopened.operational_timing = operational_timing;
                *self = reopened;
                Ok(())
            }
'''
new = '''    fn reopen(&mut self) -> Result<()> {
        let instrumentation = self.instrumentation;
        let mut operational_timing = self.operational_timing.clone();
        let operational_step_index = self.operational_step_index;
        let started = Instant::now();
        self.wal.take();
        match Self::open_existing(self.path.clone()) {
            Ok(mut reopened) => {
                let work = reopened.reopen_work()?;
                operational_timing.reopen_samples.push(OperationalTimingSample {
                    measured_step_index: operational_step_index,
                    duration_ns: u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                    work,
                });
                reopened.instrumentation = instrumentation;
                reopened.operational_timing = operational_timing;
                reopened.operational_step_index = operational_step_index;
                *self = reopened;
                Ok(())
            }
'''
text = replace_once(text, old, new, "lsm reopen work sample")
old = '''impl OperationalTimingInstrumented for LsmEngine {
    fn reset_operational_timing(&mut self) {
        self.operational_timing = OperationalTimingReport::default();
    }

    fn operational_timing_report(&self) -> OperationalTimingReport {
        self.operational_timing.clone()
    }
}
'''
new = '''impl OperationalTimingInstrumented for LsmEngine {
    fn reset_operational_timing(&mut self) {
        self.operational_timing = OperationalTimingReport::default();
        self.operational_step_index = None;
    }

    fn set_operational_step_index(&mut self, step_index: Option<u64>) {
        self.operational_step_index = step_index;
    }

    fn operational_timing_report(&self) -> OperationalTimingReport {
        self.operational_timing.clone()
    }
}
'''
text = replace_once(text, old, new, "lsm timing trait")
LSM.write_text(text)

text = LSM_TESTS.read_text()
text = replace_once(
    text,
    '''use db_core::{KvEngine, ReadWorkUnit};
''',
    '''use db_core::{KvEngine, OperationalTimingInstrumented, OperationalWorkUnit, ReadWorkUnit};
''',
    "lsm timing test imports",
)
text += r'''

#[test]
fn operational_samples_bind_compaction_and_reopen_to_deterministic_work() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("operational-work-engine");
    let mut engine = LsmEngine::create_new(&path).expect("create LSM engine");
    engine.reset_instrumentation();
    engine.reset_operational_timing();

    for index in 0_u8..7 {
        engine
            .put(&fixed_key(index), &large_value(0x20 + index))
            .expect("seed pre-trigger puts");
    }
    engine.set_operational_step_index(Some(7));
    engine
        .put(&fixed_key(7), &large_value(0x27))
        .expect("trigger first full-set compaction");
    engine.set_operational_step_index(None);

    let counters = engine.instrumentation();
    let timing = engine.operational_timing_report();
    assert_eq!(timing.compaction_stall_samples.len(), 1);
    let compaction = timing.compaction_stall_samples[0];
    assert_eq!(compaction.measured_step_index, Some(7));
    assert_eq!(compaction.work.unit, OperationalWorkUnit::LsmSstableRecordVersion);
    assert_eq!(compaction.work.units_examined, 8);
    assert_eq!(compaction.work.bytes_examined, counters.compaction_input_sstable_bytes);
    assert!(compaction.duration_ns > 0);

    let authoritative_bytes = canonical_sstable_bytes(&path);
    engine.reset_operational_timing();
    engine.set_operational_step_index(Some(99));
    KvEngine::reopen(&mut engine).expect("measured reopen");
    let timing = engine.operational_timing_report();
    assert_eq!(timing.reopen_samples.len(), 1);
    let reopen = timing.reopen_samples[0];
    assert_eq!(reopen.measured_step_index, Some(99));
    assert_eq!(reopen.work.unit, OperationalWorkUnit::LsmRecordVersion);
    assert_eq!(reopen.work.units_examined, 8, "empty rotated WAL plus eight L1 records");
    assert_eq!(reopen.work.bytes_examined, 40 + authoritative_bytes, "WAL header plus authoritative SSTable bytes");
    assert!(reopen.duration_ns > 0);
}
'''
LSM_TESTS.write_text(text)

print("applied Phase 4 operational work sample implementation")
