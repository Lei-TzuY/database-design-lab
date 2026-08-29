use serde::{Deserialize, Serialize};

use crate::{
    validate_experiment_compatibility, validate_key, validate_key_value, validate_range_scan,
    AmplificationInstrumented, AmplificationRatio, AmplificationReport, ByteString, DbError,
    EngineCapabilities, KvEngine, ReadWorkUnit, Result, StructuralReadAmplification, MAX_KEY_BYTES,
    MAX_VALUE_BYTES,
};

/// JSON schema version for Phase 4 experiment traces.
pub const EXPERIMENT_TRACE_FORMAT_VERSION: u16 = 1;
/// Defensive upper bound across setup plus measured trace steps.
pub const MAX_EXPERIMENT_STEPS: usize = 1_000_000;

/// Stable workload family used by the Phase 4 comparison runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentProfile {
    /// Seed a fixed key space, then issue deterministic hit/miss point reads.
    PointRead,
    /// Seed a fixed ordered key space, then issue bounded half-open range scans.
    RangeScan,
    /// Write distinct keys in ascending key order from an empty engine.
    SequentialWrite,
    /// Write uniformly selected keys from an empty engine.
    RandomWrite,
    /// Seed half of the key space, then mix puts, gets, range scans, and deletes.
    Mixed,
}

/// Generator inputs embedded in generated traces so the exact input can be reconstructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentGeneratorConfig {
    /// SplitMix64 seed.
    pub seed: u64,
    /// Experiment family.
    pub profile: ExperimentProfile,
    /// Number of measured logical operations, excluding inserted reopen steps.
    pub operations: u32,
    /// Number of reusable logical key ids.
    pub key_space: u32,
    /// Fixed generated value size.
    pub value_bytes: u32,
    /// Maximum number of rows requested by generated range scans.
    pub range_limit: u32,
    /// Insert a measured reopen after this many logical operations.
    pub reopen_every: Option<u32>,
}

impl ExperimentGeneratorConfig {
    fn validate(self) -> Result<()> {
        if self.operations == 0 {
            return Err(DbError::InvalidInput(
                "experiment operations must be greater than zero".to_owned(),
            ));
        }
        if self.key_space == 0 {
            return Err(DbError::InvalidInput(
                "experiment key_space must be greater than zero".to_owned(),
            ));
        }
        if usize::try_from(self.value_bytes).unwrap_or(usize::MAX) > MAX_VALUE_BYTES {
            return Err(DbError::InvalidInput(format!(
                "experiment value_bytes is {}; maximum is {MAX_VALUE_BYTES}",
                self.value_bytes
            )));
        }
        if self.reopen_every == Some(0) {
            return Err(DbError::InvalidInput(
                "experiment reopen_every must be greater than zero".to_owned(),
            ));
        }
        if self.profile == ExperimentProfile::SequentialWrite && self.operations > self.key_space {
            return Err(DbError::InvalidInput(
                "sequential_write requires key_space >= operations so measured keys stay distinct"
                    .to_owned(),
            ));
        }
        if matches!(
            self.profile,
            ExperimentProfile::RangeScan | ExperimentProfile::Mixed
        ) && (self.range_limit == 0 || self.range_limit > self.key_space)
        {
            return Err(DbError::InvalidInput(
                "range_scan and mixed traces require 0 < range_limit <= key_space".to_owned(),
            ));
        }

        let setup_steps = match self.profile {
            ExperimentProfile::PointRead | ExperimentProfile::RangeScan => {
                u64::from(self.key_space)
            }
            ExperimentProfile::Mixed => (u64::from(self.key_space) + 1) / 2,
            ExperimentProfile::SequentialWrite | ExperimentProfile::RandomWrite => 0,
        };
        let reopen_steps = self
            .reopen_every
            .map_or(0, |every| u64::from(self.operations / every));
        let total_steps = setup_steps
            .checked_add(u64::from(self.operations))
            .and_then(|count| count.checked_add(reopen_steps))
            .and_then(|count| usize::try_from(count).ok())
            .ok_or_else(|| DbError::InvalidInput("experiment trace is too large".to_owned()))?;
        if total_steps > MAX_EXPERIMENT_STEPS {
            return Err(DbError::InvalidInput(format!(
                "generated experiment would have {total_steps} steps; maximum is {MAX_EXPERIMENT_STEPS}"
            )));
        }
        Ok(())
    }
}

/// One deterministic experiment action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ExperimentStep {
    /// Set `key` to `value`.
    Put { key: ByteString, value: ByteString },
    /// Read one key.
    Get { key: ByteString },
    /// Delete one key.
    Delete { key: ByteString },
    /// Read `[start, end)` in ascending bytewise key order.
    RangeScan {
        start: ByteString,
        end: Option<ByteString>,
        limit: u32,
    },
    /// Close and reopen the current engine handle.
    Reopen,
}

/// A versioned, self-contained experiment trace.
///
/// `setup_steps` establish identical starting state on each fresh engine. Their instrumentation is
/// discarded. The runner resets amplification counters exactly once before `measured_steps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentTrace {
    /// Trace JSON schema version.
    pub format_version: u16,
    /// Human/machine-readable workload family.
    pub profile: ExperimentProfile,
    /// Generator seed for generated traces; `None` is reserved for hand-authored traces.
    pub seed: Option<u64>,
    /// Full generator inputs when the trace came from the stable generator.
    pub generator: Option<ExperimentGeneratorConfig>,
    /// State-building actions excluded from the measurement window.
    pub setup_steps: Vec<ExperimentStep>,
    /// Actions included in the measurement window.
    pub measured_steps: Vec<ExperimentStep>,
}

impl ExperimentTrace {
    /// Validates schema, operation bounds, generator metadata, and every common operation.
    pub fn validate(&self) -> Result<()> {
        if self.format_version != EXPERIMENT_TRACE_FORMAT_VERSION {
            return Err(DbError::UnsupportedVersion {
                format: "experiment trace",
                found: u64::from(self.format_version),
                supported: u64::from(EXPERIMENT_TRACE_FORMAT_VERSION),
            });
        }
        let total_steps = self
            .setup_steps
            .len()
            .checked_add(self.measured_steps.len())
            .ok_or_else(|| DbError::InvalidInput("experiment step count overflowed".to_owned()))?;
        if total_steps > MAX_EXPERIMENT_STEPS {
            return Err(DbError::InvalidInput(format!(
                "experiment trace has {total_steps} steps; maximum is {MAX_EXPERIMENT_STEPS}"
            )));
        }
        if self.measured_steps.is_empty() {
            return Err(DbError::InvalidInput(
                "experiment trace must contain at least one measured step".to_owned(),
            ));
        }
        if let Some(generator) = self.generator {
            generator.validate()?;
            if self.seed != Some(generator.seed) {
                return Err(DbError::InvalidInput(
                    "experiment trace seed does not match embedded generator config".to_owned(),
                ));
            }
            if self.profile != generator.profile {
                return Err(DbError::InvalidInput(
                    "experiment trace profile does not match embedded generator config".to_owned(),
                ));
            }
        }
        for step in self.setup_steps.iter().chain(&self.measured_steps) {
            validate_experiment_step(step)?;
        }
        Ok(())
    }

    /// Returns whether any setup or measured action requires ordered range support.
    #[must_use]
    pub fn requires_ordered_range(&self) -> bool {
        self.setup_steps
            .iter()
            .chain(&self.measured_steps)
            .any(|step| matches!(step, ExperimentStep::RangeScan { .. }))
    }
}

/// One row returned by a measured range scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentRow {
    pub key: ByteString,
    pub value: ByteString,
}

/// Observable result of one measured experiment action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ExperimentOutcome {
    Put { previous: Option<ByteString> },
    Get { value: Option<ByteString> },
    Delete { previous: Option<ByteString> },
    RangeScan { rows: Vec<ExperimentRow> },
    Reopened,
}

/// Evidence produced by one engine for a shared trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExperimentEngineEvidence {
    pub capabilities: EngineCapabilities,
    pub amplification: AmplificationReport,
}

/// Self-contained report for one engine run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExperimentRunReport {
    pub trace: ExperimentTrace,
    pub setup_steps_executed: usize,
    pub measured_steps_executed: usize,
    pub outcomes: Vec<ExperimentOutcome>,
    pub engine: ExperimentEngineEvidence,
}

/// Self-contained cross-engine report. Logical outcomes are stored once after exact equality is proven.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExperimentComparisonReport {
    pub trace: ExperimentTrace,
    pub setup_steps_executed: usize,
    pub measured_steps_executed: usize,
    pub outcomes: Vec<ExperimentOutcome>,
    pub left: ExperimentEngineEvidence,
    pub right: ExperimentEngineEvidence,
}

/// Generates one of the stable Phase 4 trace families.
pub fn generate_experiment_trace(config: ExperimentGeneratorConfig) -> Result<ExperimentTrace> {
    config.validate()?;
    let mut random = SplitMix64::new(config.seed);
    let mut setup_steps = Vec::new();
    match config.profile {
        ExperimentProfile::PointRead | ExperimentProfile::RangeScan => {
            for key_id in 0..u64::from(config.key_space) {
                setup_steps.push(ExperimentStep::Put {
                    key: ByteString::from(experiment_key(key_id)),
                    value: ByteString::from(random_value(&mut random, config.value_bytes)),
                });
            }
        }
        ExperimentProfile::Mixed => {
            for key_id in 0..u64::from(config.key_space) {
                if key_id % 2 == 0 {
                    setup_steps.push(ExperimentStep::Put {
                        key: ByteString::from(experiment_key(key_id)),
                        value: ByteString::from(random_value(&mut random, config.value_bytes)),
                    });
                }
            }
        }
        ExperimentProfile::SequentialWrite | ExperimentProfile::RandomWrite => {}
    }

    let reopen_count = config
        .reopen_every
        .map_or(0, |every| config.operations / every);
    let measured_capacity = usize::try_from(u64::from(config.operations) + u64::from(reopen_count))
        .map_err(|_| DbError::InvalidInput("experiment measured step count overflowed".to_owned()))?;
    let mut measured_steps = Vec::with_capacity(measured_capacity);
    for operation_index in 0..config.operations {
        let step = match config.profile {
            ExperimentProfile::PointRead => {
                let hit = random.bounded(100) < 80;
                let key_id = if hit {
                    random.bounded(u64::from(config.key_space))
                } else {
                    u64::from(config.key_space)
                        + random.bounded(u64::from(config.key_space))
                };
                ExperimentStep::Get {
                    key: ByteString::from(experiment_key(key_id)),
                }
            }
            ExperimentProfile::RangeScan => {
                generated_range_step(&mut random, config.key_space, config.range_limit)
            }
            ExperimentProfile::SequentialWrite => ExperimentStep::Put {
                key: ByteString::from(experiment_key(u64::from(operation_index))),
                value: ByteString::from(random_value(&mut random, config.value_bytes)),
            },
            ExperimentProfile::RandomWrite => ExperimentStep::Put {
                key: ByteString::from(experiment_key(
                    random.bounded(u64::from(config.key_space)),
                )),
                value: ByteString::from(random_value(&mut random, config.value_bytes)),
            },
            ExperimentProfile::Mixed => match random.bounded(100) {
                0..=39 => ExperimentStep::Put {
                    key: ByteString::from(experiment_key(
                        random.bounded(u64::from(config.key_space)),
                    )),
                    value: ByteString::from(random_value(&mut random, config.value_bytes)),
                },
                40..=69 => ExperimentStep::Get {
                    key: ByteString::from(experiment_key(
                        random.bounded(u64::from(config.key_space)),
                    )),
                },
                70..=84 => {
                    generated_range_step(&mut random, config.key_space, config.range_limit)
                }
                _ => ExperimentStep::Delete {
                    key: ByteString::from(experiment_key(
                        random.bounded(u64::from(config.key_space)),
                    )),
                },
            },
        };
        measured_steps.push(step);
        if config
            .reopen_every
            .is_some_and(|every| (operation_index + 1) % every == 0)
        {
            measured_steps.push(ExperimentStep::Reopen);
        }
    }

    let trace = ExperimentTrace {
        format_version: EXPERIMENT_TRACE_FORMAT_VERSION,
        profile: config.profile,
        seed: Some(config.seed),
        generator: Some(config),
        setup_steps,
        measured_steps,
    };
    trace.validate()?;
    Ok(trace)
}

/// Executes one experiment action using the common KV contract.
pub fn execute_experiment_step<E: KvEngine>(
    engine: &mut E,
    step: &ExperimentStep,
) -> Result<ExperimentOutcome> {
    match step {
        ExperimentStep::Put { key, value } => engine
            .put(key.as_slice(), value.as_slice())
            .map(|previous| ExperimentOutcome::Put {
                previous: previous.map(ByteString::from),
            }),
        ExperimentStep::Get { key } => engine
            .get(key.as_slice())
            .map(|value| ExperimentOutcome::Get {
                value: value.map(ByteString::from),
            }),
        ExperimentStep::Delete { key } => engine
            .delete(key.as_slice())
            .map(|previous| ExperimentOutcome::Delete {
                previous: previous.map(ByteString::from),
            }),
        ExperimentStep::RangeScan { start, end, limit } => engine
            .range_scan(
                start.as_slice(),
                end.as_ref().map(ByteString::as_slice),
                usize::try_from(*limit).unwrap_or(usize::MAX),
            )
            .map(|rows| ExperimentOutcome::RangeScan {
                rows: rows
                    .into_iter()
                    .map(|(key, value)| ExperimentRow {
                        key: ByteString::from(key),
                        value: ByteString::from(value),
                    })
                    .collect(),
            }),
        ExperimentStep::Reopen => {
            engine.reopen()?;
            Ok(ExperimentOutcome::Reopened)
        }
    }
}

/// Runs setup, resets instrumentation, runs the measured window, and captures exact evidence.
pub fn run_experiment_trace<E>(
    engine: &mut E,
    trace: &ExperimentTrace,
) -> Result<ExperimentRunReport>
where
    E: KvEngine + AmplificationInstrumented,
{
    trace.validate()?;
    let capabilities = engine.capabilities();
    if trace.requires_ordered_range() && !capabilities.ordered_range_scan {
        return Err(DbError::InvalidInput(format!(
            "experiment trace requires ordered ranges but engine {} does not expose them",
            capabilities.name
        )));
    }
    for step in &trace.setup_steps {
        execute_experiment_step(engine, step)?;
    }
    engine.reset_amplification();
    let outcomes = trace
        .measured_steps
        .iter()
        .map(|step| execute_experiment_step(engine, step))
        .collect::<Result<Vec<_>>>()?;
    let amplification = engine.amplification_report()?;
    Ok(ExperimentRunReport {
        trace: trace.clone(),
        setup_steps_executed: trace.setup_steps.len(),
        measured_steps_executed: outcomes.len(),
        outcomes,
        engine: ExperimentEngineEvidence {
            capabilities,
            amplification,
        },
    })
}

/// Runs the exact same trace against two fresh candidates and refuses to report incomparable semantics.
pub fn compare_experiment_trace<L, R>(
    left: &mut L,
    right: &mut R,
    trace: &ExperimentTrace,
) -> Result<ExperimentComparisonReport>
where
    L: KvEngine + AmplificationInstrumented,
    R: KvEngine + AmplificationInstrumented,
{
    trace.validate()?;
    validate_experiment_compatibility(
        left.capabilities(),
        right.capabilities(),
        trace.requires_ordered_range(),
    )?;
    let left_report = run_experiment_trace(left, trace)?;
    let right_report = run_experiment_trace(right, trace)?;
    if left_report.outcomes != right_report.outcomes {
        let mismatch = left_report
            .outcomes
            .iter()
            .zip(&right_report.outcomes)
            .position(|(left, right)| left != right)
            .unwrap_or_else(|| left_report.outcomes.len().min(right_report.outcomes.len()));
        return Err(DbError::InvalidInput(format!(
            "experiment logical outcomes diverged at measured step {mismatch}"
        )));
    }
    Ok(ExperimentComparisonReport {
        trace: trace.clone(),
        setup_steps_executed: trace.setup_steps.len(),
        measured_steps_executed: trace.measured_steps.len(),
        outcomes: left_report.outcomes,
        left: left_report.engine,
        right: right_report.engine,
    })
}

fn validate_experiment_step(step: &ExperimentStep) -> Result<()> {
    match step {
        ExperimentStep::Put { key, value } => validate_key_value(key.as_slice(), value.as_slice()),
        ExperimentStep::Get { key } | ExperimentStep::Delete { key } => validate_key(key.as_slice()),
        ExperimentStep::RangeScan { start, end, .. } => {
            validate_range_scan(start.as_slice(), end.as_ref().map(ByteString::as_slice))
        }
        ExperimentStep::Reopen => Ok(()),
    }
}

fn generated_range_step(
    random: &mut SplitMix64,
    key_space: u32,
    range_limit: u32,
) -> ExperimentStep {
    let start_id = random.bounded(u64::from(key_space));
    let end_id = start_id
        .saturating_add(u64::from(range_limit))
        .min(u64::from(key_space));
    ExperimentStep::RangeScan {
        start: ByteString::from(experiment_key(start_id)),
        end: Some(ByteString::from(experiment_key(end_id))),
        limit: range_limit,
    }
}

fn experiment_key(key_id: u64) -> Vec<u8> {
    key_id.to_be_bytes().to_vec()
}

fn random_value(random: &mut SplitMix64, length: u32) -> Vec<u8> {
    let mut value = vec![0_u8; usize::try_from(length).unwrap_or(usize::MAX)];
    random.fill(&mut value);
    value
}

/// Small specified PRNG duplicated intentionally so experiment generation is stable independently of
/// future changes to the correctness-workload generator.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn bounded(&mut self, upper_exclusive: u64) -> u64 {
        self.next() % upper_exclusive
    }

    fn fill(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            let random = self.next().to_le_bytes();
            chunk.copy_from_slice(&random[..chunk.len()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        compare_experiment_trace, generate_experiment_trace, run_experiment_trace,
        ExperimentGeneratorConfig, ExperimentProfile, ExperimentStep,
    };
    use crate::{
        AmplificationInstrumented, AmplificationRatio, AmplificationReport, ConcurrencyMode,
        CrashRecovery, DistributionMode, EngineCapabilities, KvEngine, LogicalModel, Persistence,
        ReadWorkUnit, Result, StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES,
        MAX_VALUE_BYTES,
    };

    #[test]
    fn every_generated_profile_is_repeatable_and_valid() {
        for profile in [
            ExperimentProfile::PointRead,
            ExperimentProfile::RangeScan,
            ExperimentProfile::SequentialWrite,
            ExperimentProfile::RandomWrite,
            ExperimentProfile::Mixed,
        ] {
            let config = ExperimentGeneratorConfig {
                seed: 0x51_7eed,
                profile,
                operations: 32,
                key_space: 64,
                value_bytes: 24,
                range_limit: 8,
                reopen_every: Some(7),
            };
            let first = generate_experiment_trace(config).expect("generate trace");
            let second = generate_experiment_trace(config).expect("regenerate trace");
            assert_eq!(first, second);
            first.validate().expect("validate generated trace");
            assert_eq!(first.seed, Some(config.seed));
            assert_eq!(first.generator, Some(config));
        }
    }

    #[test]
    fn point_profile_keeps_seed_writes_outside_the_measured_window() {
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 7,
            profile: ExperimentProfile::PointRead,
            operations: 10,
            key_space: 4,
            value_bytes: 8,
            range_limit: 1,
            reopen_every: Some(5),
        })
        .expect("point trace");
        assert_eq!(trace.setup_steps.len(), 4);
        assert!(trace
            .setup_steps
            .iter()
            .all(|step| matches!(step, ExperimentStep::Put { .. })));
        assert_eq!(trace.measured_steps.len(), 12);
        assert!(trace.measured_steps.iter().all(|step| matches!(
            step,
            ExperimentStep::Get { .. } | ExperimentStep::Reopen
        )));
    }

    #[test]
    fn sequential_write_rejects_key_reuse() {
        let error = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 1,
            profile: ExperimentProfile::SequentialWrite,
            operations: 5,
            key_space: 4,
            value_bytes: 1,
            range_limit: 1,
            reopen_every: None,
        })
        .expect_err("sequential writes should require distinct keys");
        assert!(error.to_string().contains("key_space >= operations"));
    }

    #[test]
    fn shared_runner_resets_after_setup_and_compares_exact_outcomes() {
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 0xabc,
            profile: ExperimentProfile::Mixed,
            operations: 40,
            key_space: 16,
            value_bytes: 12,
            range_limit: 4,
            reopen_every: Some(11),
        })
        .expect("mixed trace");
        let mut left = FakeEngine::new("left", StorageArchitecture::BPlusTree);
        let mut right = FakeEngine::new("right", StorageArchitecture::LsmTree);
        let report = compare_experiment_trace(&mut left, &mut right, &trace).expect("compare");
        assert_eq!(report.outcomes.len(), trace.measured_steps.len());
        assert_eq!(left.reset_calls, 1);
        assert_eq!(right.reset_calls, 1);
        assert_eq!(
            report.left.amplification.point_read.unit,
            ReadWorkUnit::BtreePageAccess
        );
        assert_eq!(
            report.right.amplification.point_read.unit,
            ReadWorkUnit::LsmSstableConsult
        );
    }

    #[test]
    fn single_runner_is_self_contained() {
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 9,
            profile: ExperimentProfile::RandomWrite,
            operations: 8,
            key_space: 4,
            value_bytes: 4,
            range_limit: 1,
            reopen_every: None,
        })
        .expect("random write trace");
        let mut engine = FakeEngine::new("single", StorageArchitecture::BPlusTree);
        let report = run_experiment_trace(&mut engine, &trace).expect("run");
        assert_eq!(report.trace, trace);
        assert_eq!(report.measured_steps_executed, 8);
        assert_eq!(report.engine.capabilities.name, "single");
    }

    struct FakeEngine {
        name: &'static str,
        architecture: StorageArchitecture,
        map: BTreeMap<Vec<u8>, Vec<u8>>,
        point_reads: u64,
        range_rows: u64,
        logical_bytes: u64,
        reset_calls: u64,
    }

    impl FakeEngine {
        fn new(name: &'static str, architecture: StorageArchitecture) -> Self {
            Self {
                name,
                architecture,
                map: BTreeMap::new(),
                point_reads: 0,
                range_rows: 0,
                logical_bytes: 0,
                reset_calls: 0,
            }
        }
    }

    impl KvEngine for FakeEngine {
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                name: self.name,
                logical_model: LogicalModel::KeyValue,
                storage_architecture: self.architecture,
                concurrency: ConcurrencyMode::CallerSerialized,
                persistence: Persistence::Persistent,
                crash_recovery: match self.architecture {
                    StorageArchitecture::BPlusTree => CrashRecovery::MirroredCopyOnWritePages,
                    _ => CrashRecovery::WriteAheadLogReplay,
                },
                distribution: DistributionMode::Standalone,
                ordered_range_scan: true,
                max_key_bytes: MAX_KEY_BYTES,
                max_value_bytes: MAX_VALUE_BYTES,
            }
        }

        fn put(&mut self, key: &[u8], value: &[u8]) -> Result<Option<Vec<u8>>> {
            self.logical_bytes = self
                .logical_bytes
                .saturating_add(u64::try_from(key.len() + value.len()).unwrap_or(u64::MAX));
            Ok(self.map.insert(key.to_vec(), value.to_vec()))
        }

        fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            self.point_reads = self.point_reads.saturating_add(1);
            Ok(self.map.get(key).cloned())
        }

        fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            self.logical_bytes = self
                .logical_bytes
                .saturating_add(u64::try_from(key.len()).unwrap_or(u64::MAX));
            Ok(self.map.remove(key))
        }

        fn range_scan(
            &mut self,
            start: &[u8],
            end: Option<&[u8]>,
            limit: usize,
        ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
            let rows: Vec<_> = self
                .map
                .iter()
                .filter(|(key, _)| key.as_slice() >= start)
                .filter(|(key, _)| end.is_none_or(|end| key.as_slice() < end))
                .take(limit)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            self.range_rows = self
                .range_rows
                .saturating_add(u64::try_from(rows.len()).unwrap_or(u64::MAX));
            Ok(rows)
        }

        fn reopen(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl AmplificationInstrumented for FakeEngine {
        fn reset_amplification(&mut self) {
            self.point_reads = 0;
            self.range_rows = 0;
            self.logical_bytes = 0;
            self.reset_calls = self.reset_calls.saturating_add(1);
        }

        fn amplification_report(&mut self) -> Result<AmplificationReport> {
            let point_unit = if self.architecture == StorageArchitecture::BPlusTree {
                ReadWorkUnit::BtreePageAccess
            } else {
                ReadWorkUnit::LsmSstableConsult
            };
            Ok(AmplificationReport {
                point_read: StructuralReadAmplification {
                    ratio: AmplificationRatio {
                        numerator: self.point_reads,
                        denominator: self.point_reads,
                    },
                    unit: point_unit,
                },
                range_read: StructuralReadAmplification {
                    ratio: AmplificationRatio {
                        numerator: self.range_rows,
                        denominator: self.range_rows,
                    },
                    unit: if self.architecture == StorageArchitecture::BPlusTree {
                        ReadWorkUnit::BtreePageAccess
                    } else {
                        ReadWorkUnit::LsmSstableVersionDecoded
                    },
                },
                data_write_bytes_per_logical_byte: AmplificationRatio {
                    numerator: self.logical_bytes,
                    denominator: self.logical_bytes,
                },
                primary_structure_bytes_per_live_byte: AmplificationRatio {
                    numerator: 0,
                    denominator: 0,
                },
            })
        }
    }
}
