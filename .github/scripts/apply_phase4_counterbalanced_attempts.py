from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENGINE = ROOT / "crates/db-core/src/engine.rs"
EXPERIMENT = ROOT / "crates/db-core/src/experiment.rs"
CORE_LIB = ROOT / "crates/db-core/src/lib.rs"
BTREE_COMMON = ROOT / "crates/db-storage-btree/src/tree/common.rs"
LSM = ROOT / "crates/db-storage-lsm/src/lib.rs"
CLI = ROOT / "crates/db-cli/src/main.rs"


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected 1 marker, found {count}")
    path.write_text(text.replace(old, new, 1))


# ---------- db-core operational timing attempt evidence ----------
replace_once(
    ENGINE,
    "use crate::{ByteString, DbError, Outcome, Result, Workload, WorkloadStep};",
    "use crate::{ByteString, DbError, ErrorClass, Outcome, Result, Workload, WorkloadStep};",
    "engine ErrorClass import",
)
replace_once(
    ENGINE,
    '''pub struct OperationalTimingSample {
    /// Zero-based measured experiment step that triggered this sample, or `None` outside a measured runner.
    pub measured_step_index: Option<u64>,
    /// Wall-clock duration measured with `std::time::Instant`.
    pub duration_ns: u64,
    /// Deterministic data-path work completed by the timed operation.
    pub work: OperationalWork,
}

/// Raw process-local successful recovery and compaction-stall samples.
''',
    '''pub struct OperationalTimingSample {
    /// Zero-based measured experiment step that triggered this sample, or `None` outside a measured runner.
    pub measured_step_index: Option<u64>,
    /// Wall-clock duration measured with `std::time::Instant`.
    pub duration_ns: u64,
    /// Deterministic data-path work completed by the timed operation.
    pub work: OperationalWork,
}

/// Outcome retained for every attempted timed recovery/compaction operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OperationalAttemptOutcome {
    /// The timed operation completed successfully.
    Succeeded,
    /// The timed operation returned an error. The engine may require reopen before reuse.
    Failed {
        /// Stable common error class.
        error_class: ErrorClass,
        /// Human-readable error detail retained for forensic evidence.
        message: String,
    },
}

/// One attempted synchronous operation, including failures excluded from success distributions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperationalAttemptSample {
    /// Zero-based measured experiment step that triggered this attempt, or `None` outside a measured runner.
    pub measured_step_index: Option<u64>,
    /// Wall-clock duration measured with `std::time::Instant` until success or returned failure.
    pub duration_ns: u64,
    /// Deterministic work when it can be reconstructed without performing extra measurement I/O.
    pub work: Option<OperationalWork>,
    /// Success/failure disposition retained without filtering the raw attempt stream.
    pub outcome: OperationalAttemptOutcome,
}

/// Raw process-local recovery and compaction-stall evidence.
''',
    "operational attempt types",
)
replace_once(
    ENGINE,
    '''    /// Successful synchronous compaction samples with deterministic work and measured-step association.
    pub compaction_stall_samples: Vec<OperationalTimingSample>,
}
''',
    '''    /// Successful synchronous compaction samples with deterministic work and measured-step association.
    pub compaction_stall_samples: Vec<OperationalTimingSample>,
    /// Every same-handle `REOPEN` attempt, including failures omitted from `reopen_ns`.
    pub reopen_attempts: Vec<OperationalAttemptSample>,
    /// Every triggered synchronous compaction attempt, including failures omitted from success distributions.
    pub compaction_stall_attempts: Vec<OperationalAttemptSample>,
}
''',
    "operational attempt report fields",
)

# ---------- experiment execution order + batch ledger ----------
replace_once(
    EXPERIMENT,
    '''    AmplificationInstrumented, AmplificationReport, ByteString, DbError, EngineCapabilities,
    KvEngine, OperationalTimingInstrumented, OperationalTimingReport, Result, MAX_VALUE_BYTES,
};
''',
    '''    AmplificationInstrumented, AmplificationReport, ByteString, DbError, EngineCapabilities,
    ErrorClass, KvEngine, OperationalTimingInstrumented, OperationalTimingReport, Result,
    MAX_VALUE_BYTES,
};
''',
    "experiment imports",
)
replace_once(
    EXPERIMENT,
    '''pub const MAX_EXPERIMENT_RANGE_LIMIT: u32 = 1_000_000;

const EXPERIMENT_KEY_BYTES: u64 = 8;
''',
    '''pub const MAX_EXPERIMENT_RANGE_LIMIT: u32 = 1_000_000;
/// JSON schema version for repeated counterbalanced experiment batches.
pub const EXPERIMENT_BATCH_FORMAT_VERSION: u16 = 1;
/// Defensive bound on warmup plus included attempts retained in one batch report.
pub const MAX_EXPERIMENT_BATCH_ATTEMPTS: u32 = 128;
/// Defensive bound on total measured step executions represented by one batch.
pub const MAX_EXPERIMENT_BATCH_MEASURED_STEP_EXECUTIONS: u64 = 2_000_000;

const EXPERIMENT_KEY_BYTES: u64 = 8;
''',
    "batch constants",
)
replace_once(
    EXPERIMENT,
    '''/// Evidence produced by one engine for a shared trace.
''',
    '''/// Which candidate executes first for every paired setup/measured action in one comparison attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentExecutionOrder {
    /// Execute the logical left candidate first, then the logical right candidate.
    LeftFirst,
    /// Execute the logical right candidate first, then the logical left candidate.
    RightFirst,
}

impl ExperimentExecutionOrder {
    const fn opposite(self) -> Self {
        match self {
            Self::LeftFirst => Self::RightFirst,
            Self::RightFirst => Self::LeftFirst,
        }
    }
}

/// Repeated-run configuration used to counterbalance execution-order bias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentBatchConfig {
    /// Number of measured attempts included in later distributions. Must be positive and even.
    pub included_attempts: u32,
    /// Warmup attempts that are executed and archived but explicitly excluded from distributions.
    pub warmup_attempts: u32,
    /// Stable seed used only to choose the orientation of each included AB/BA pair and warmup order.
    pub order_seed: u64,
}

impl ExperimentBatchConfig {
    /// Validates counterbalancing and defensive resource bounds for one trace.
    pub fn validate(self, trace: &ExperimentTrace) -> Result<()> {
        if self.included_attempts == 0 || self.included_attempts % 2 != 0 {
            return Err(DbError::InvalidInput(
                "experiment batch included_attempts must be a positive even number".to_owned(),
            ));
        }
        let total = self
            .included_attempts
            .checked_add(self.warmup_attempts)
            .ok_or_else(|| DbError::InvalidInput("experiment batch attempt count overflowed".to_owned()))?;
        if total > MAX_EXPERIMENT_BATCH_ATTEMPTS {
            return Err(DbError::InvalidInput(format!(
                "experiment batch has {total} attempts; maximum is {MAX_EXPERIMENT_BATCH_ATTEMPTS}"
            )));
        }
        let measured_steps = u64::try_from(trace.measured_steps.len())
            .map_err(|_| DbError::InvalidInput("measured step count does not fit u64".to_owned()))?;
        let executions = measured_steps.checked_mul(u64::from(total)).ok_or_else(|| {
            DbError::InvalidInput("experiment batch measured execution count overflowed".to_owned())
        })?;
        if executions > MAX_EXPERIMENT_BATCH_MEASURED_STEP_EXECUTIONS {
            return Err(DbError::InvalidInput(format!(
                "experiment batch represents {executions} measured step executions; maximum is {MAX_EXPERIMENT_BATCH_MEASURED_STEP_EXECUTIONS}"
            )));
        }
        Ok(())
    }
}

/// Whether one attempt participates in measured distributions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentAttemptDisposition {
    /// Warmup attempt retained as evidence but excluded from measured distributions.
    ExcludedWarmup,
    /// Attempt included in the counterbalanced measured set.
    Included,
}

/// Stage at which a retained batch attempt failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentAttemptFailureStage {
    /// Fresh engine creation/factory failed before the trace could run.
    Factory,
    /// The two-engine comparison returned an execution or logical-equivalence error.
    Comparison,
    /// A successful pair produced deterministic outcomes different from an earlier successful attempt.
    CrossAttemptOutcomeMismatch,
}

/// Result retained for one fresh-engine batch attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ExperimentAttemptResult {
    /// Both fresh candidates completed and matched logically.
    Succeeded {
        setup_steps_executed: usize,
        measured_steps_executed: usize,
        left: ExperimentEngineEvidence,
        right: ExperimentEngineEvidence,
    },
    /// The attempt failed but remains archived instead of disappearing from evidence.
    Failed {
        stage: ExperimentAttemptFailureStage,
        error_class: ErrorClass,
        message: String,
        /// Partial left timing evidence when engines existed before failure.
        left_operational_timing: Option<OperationalTimingReport>,
        /// Partial right timing evidence when engines existed before failure.
        right_operational_timing: Option<OperationalTimingReport>,
    },
}

/// One retained repeated-run attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExperimentBatchAttempt {
    pub attempt_index: u32,
    pub disposition: ExperimentAttemptDisposition,
    pub execution_order: ExperimentExecutionOrder,
    pub result: ExperimentAttemptResult,
}

/// Aggregate counts that make inclusion/exclusion and AB/BA balance explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExperimentBatchSummary {
    pub included_successes: u32,
    pub included_failures: u32,
    pub excluded_warmups: u32,
    pub included_left_first: u32,
    pub included_right_first: u32,
}

/// Self-contained repeated experiment report with one canonical logical outcome vector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExperimentBatchReport {
    pub format_version: u16,
    pub config: ExperimentBatchConfig,
    pub trace: ExperimentTrace,
    /// Outcomes are stored once after the first successful fresh-engine comparison and must match later successes.
    pub canonical_outcomes: Option<Vec<ExperimentOutcome>>,
    pub summary: ExperimentBatchSummary,
    pub attempts: Vec<ExperimentBatchAttempt>,
}

/// Evidence produced by one engine for a shared trace.
''',
    "batch types",
)
# Convert the existing comparison function into default wrapper + ordered implementation.
replace_once(
    EXPERIMENT,
    '''/// Runs the exact same trace against two fresh candidates and refuses to report incomparable semantics.
pub fn compare_experiment_trace<L, R>(
    left: &mut L,
    right: &mut R,
    trace: &ExperimentTrace,
) -> Result<ExperimentComparisonReport>
where
    L: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    R: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
{
''',
    '''/// Runs the exact same trace against two fresh candidates using the historical left-first order.
pub fn compare_experiment_trace<L, R>(
    left: &mut L,
    right: &mut R,
    trace: &ExperimentTrace,
) -> Result<ExperimentComparisonReport>
where
    L: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    R: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
{
    compare_experiment_trace_ordered(left, right, trace, ExperimentExecutionOrder::LeftFirst)
}

/// Runs the exact same trace against two fresh candidates with an explicit paired execution order.
pub fn compare_experiment_trace_ordered<L, R>(
    left: &mut L,
    right: &mut R,
    trace: &ExperimentTrace,
    execution_order: ExperimentExecutionOrder,
) -> Result<ExperimentComparisonReport>
where
    L: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    R: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
{
''',
    "ordered comparison wrapper",
)
replace_once(
    EXPERIMENT,
    '''    for (index, step) in trace.setup_steps.iter().enumerate() {
        let left_outcome = execute_experiment_step(left, step)?;
        let right_outcome = execute_experiment_step(right, step)?;
        if left_outcome != right_outcome {
''',
    '''    for (index, step) in trace.setup_steps.iter().enumerate() {
        let (left_outcome, right_outcome) =
            execute_experiment_pair(left, right, step, execution_order)?;
        if left_outcome != right_outcome {
''',
    "ordered setup execution",
)
replace_once(
    EXPERIMENT,
    '''    for (index, step) in trace.measured_steps.iter().enumerate() {
        let left_outcome = execute_measured_experiment_step(left, step, index)?;
        let right_outcome = execute_measured_experiment_step(right, step, index)?;
        if left_outcome != right_outcome {
''',
    '''    for (index, step) in trace.measured_steps.iter().enumerate() {
        let (left_outcome, right_outcome) =
            execute_measured_experiment_pair(left, right, step, index, execution_order)?;
        if left_outcome != right_outcome {
''',
    "ordered measured execution",
)
# Insert batch runner before measured helper.
replace_once(
    EXPERIMENT,
    '''fn execute_measured_experiment_step<E>(
''',
    '''/// Runs a counterbalanced repeated batch using fresh engines supplied by `factory`.
///
/// Included attempts are arranged in deterministic opposite-order pairs, guaranteeing equal left-first
/// and right-first counts. Warmups are retained but excluded. Per-attempt factory/comparison failures are
/// evidence, not batch-level errors; schema/config/trace validation failures still fail the batch itself.
pub fn compare_experiment_batch<L, R, F>(
    trace: &ExperimentTrace,
    config: ExperimentBatchConfig,
    mut factory: F,
) -> Result<ExperimentBatchReport>
where
    L: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    R: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    F: FnMut(u32, ExperimentExecutionOrder) -> Result<(L, R)>,
{
    trace.validate()?;
    config.validate(trace)?;
    let total = config.included_attempts + config.warmup_attempts;
    let mut canonical_outcomes: Option<Vec<ExperimentOutcome>> = None;
    let mut attempts = Vec::with_capacity(usize::try_from(total).unwrap_or(usize::MAX));
    let mut summary = ExperimentBatchSummary {
        included_successes: 0,
        included_failures: 0,
        excluded_warmups: config.warmup_attempts,
        included_left_first: 0,
        included_right_first: 0,
    };

    for attempt_index in 0..total {
        let disposition = if attempt_index < config.warmup_attempts {
            ExperimentAttemptDisposition::ExcludedWarmup
        } else {
            ExperimentAttemptDisposition::Included
        };
        let execution_order = batch_execution_order(config, attempt_index);
        if disposition == ExperimentAttemptDisposition::Included {
            match execution_order {
                ExperimentExecutionOrder::LeftFirst => {
                    summary.included_left_first = summary.included_left_first.saturating_add(1)
                }
                ExperimentExecutionOrder::RightFirst => {
                    summary.included_right_first = summary.included_right_first.saturating_add(1)
                }
            }
        }

        let pair = factory(attempt_index, execution_order);
        let result = match pair {
            Err(error) => ExperimentAttemptResult::Failed {
                stage: ExperimentAttemptFailureStage::Factory,
                error_class: error.class(),
                message: error.to_string(),
                left_operational_timing: None,
                right_operational_timing: None,
            },
            Ok((mut left, mut right)) => {
                match compare_experiment_trace_ordered(
                    &mut left,
                    &mut right,
                    trace,
                    execution_order,
                ) {
                    Err(error) => ExperimentAttemptResult::Failed {
                        stage: ExperimentAttemptFailureStage::Comparison,
                        error_class: error.class(),
                        message: error.to_string(),
                        left_operational_timing: Some(left.operational_timing_report()),
                        right_operational_timing: Some(right.operational_timing_report()),
                    },
                    Ok(report) => {
                        if canonical_outcomes
                            .as_ref()
                            .is_some_and(|canonical| canonical != &report.outcomes)
                        {
                            ExperimentAttemptResult::Failed {
                                stage: ExperimentAttemptFailureStage::CrossAttemptOutcomeMismatch,
                                error_class: ErrorClass::InvalidInput,
                                message: "successful experiment attempt produced outcomes different from the canonical successful attempt".to_owned(),
                                left_operational_timing: Some(report.left.operational_timing.clone()),
                                right_operational_timing: Some(report.right.operational_timing.clone()),
                            }
                        } else {
                            if canonical_outcomes.is_none() {
                                canonical_outcomes = Some(report.outcomes.clone());
                            }
                            ExperimentAttemptResult::Succeeded {
                                setup_steps_executed: report.setup_steps_executed,
                                measured_steps_executed: report.measured_steps_executed,
                                left: report.left,
                                right: report.right,
                            }
                        }
                    }
                }
            }
        };
        if disposition == ExperimentAttemptDisposition::Included {
            match result {
                ExperimentAttemptResult::Succeeded { .. } => {
                    summary.included_successes = summary.included_successes.saturating_add(1)
                }
                ExperimentAttemptResult::Failed { .. } => {
                    summary.included_failures = summary.included_failures.saturating_add(1)
                }
            }
        }
        attempts.push(ExperimentBatchAttempt {
            attempt_index,
            disposition,
            execution_order,
            result,
        });
    }

    debug_assert_eq!(summary.included_left_first, summary.included_right_first);
    Ok(ExperimentBatchReport {
        format_version: EXPERIMENT_BATCH_FORMAT_VERSION,
        config,
        trace: trace.clone(),
        canonical_outcomes,
        summary,
        attempts,
    })
}

fn batch_execution_order(
    config: ExperimentBatchConfig,
    attempt_index: u32,
) -> ExperimentExecutionOrder {
    if attempt_index < config.warmup_attempts {
        let mut random = SplitMix64::new(
            config
                .order_seed
                .wrapping_add(u64::from(attempt_index))
                .wrapping_add(0xa076_1d64_78bd_642f),
        );
        return if random.next() & 1 == 0 {
            ExperimentExecutionOrder::LeftFirst
        } else {
            ExperimentExecutionOrder::RightFirst
        };
    }
    let included_index = attempt_index - config.warmup_attempts;
    let pair_index = included_index / 2;
    let mut random = SplitMix64::new(
        config
            .order_seed
            .wrapping_add(u64::from(pair_index).wrapping_mul(0xe703_7ed1_a0b4_28db)),
    );
    let pair_first = if random.next() & 1 == 0 {
        ExperimentExecutionOrder::LeftFirst
    } else {
        ExperimentExecutionOrder::RightFirst
    };
    if included_index % 2 == 0 {
        pair_first
    } else {
        pair_first.opposite()
    }
}

fn execute_experiment_pair<L: KvEngine, R: KvEngine>(
    left: &mut L,
    right: &mut R,
    step: &ExperimentStep,
    order: ExperimentExecutionOrder,
) -> Result<(ExperimentOutcome, ExperimentOutcome)> {
    match order {
        ExperimentExecutionOrder::LeftFirst => {
            let left_outcome = execute_experiment_step(left, step)?;
            let right_outcome = execute_experiment_step(right, step)?;
            Ok((left_outcome, right_outcome))
        }
        ExperimentExecutionOrder::RightFirst => {
            let right_outcome = execute_experiment_step(right, step)?;
            let left_outcome = execute_experiment_step(left, step)?;
            Ok((left_outcome, right_outcome))
        }
    }
}

fn execute_measured_experiment_pair<L, R>(
    left: &mut L,
    right: &mut R,
    step: &ExperimentStep,
    index: usize,
    order: ExperimentExecutionOrder,
) -> Result<(ExperimentOutcome, ExperimentOutcome)>
where
    L: KvEngine + OperationalTimingInstrumented,
    R: KvEngine + OperationalTimingInstrumented,
{
    match order {
        ExperimentExecutionOrder::LeftFirst => {
            let left_outcome = execute_measured_experiment_step(left, step, index)?;
            let right_outcome = execute_measured_experiment_step(right, step, index)?;
            Ok((left_outcome, right_outcome))
        }
        ExperimentExecutionOrder::RightFirst => {
            let right_outcome = execute_measured_experiment_step(right, step, index)?;
            let left_outcome = execute_measured_experiment_step(left, step, index)?;
            Ok((left_outcome, right_outcome))
        }
    }
}

fn execute_measured_experiment_step<E>(
''',
    "batch runner",
)
# Test imports and FakeEngine attempt streams + new tests.
replace_once(
    EXPERIMENT,
    '''        checked_add_payload, compare_experiment_trace, generate_experiment_trace,
        run_experiment_trace, ExperimentGeneratorConfig, ExperimentProfile, ExperimentStep,
        ExperimentTrace, MAX_EXPERIMENT_OUTCOME_PAYLOAD_BYTES,
''',
    '''        checked_add_payload, compare_experiment_batch, compare_experiment_trace,
        generate_experiment_trace, run_experiment_trace, ExperimentAttemptDisposition,
        ExperimentAttemptFailureStage, ExperimentAttemptResult, ExperimentBatchConfig,
        ExperimentExecutionOrder, ExperimentGeneratorConfig, ExperimentProfile, ExperimentStep,
        ExperimentTrace, MAX_EXPERIMENT_OUTCOME_PAYLOAD_BYTES,
''',
    "experiment test imports",
)
replace_once(
    EXPERIMENT,
    '''        AmplificationInstrumented, AmplificationRatio, AmplificationReport, ConcurrencyMode,
        CrashRecovery, DistributionMode, EngineCapabilities, KvEngine, LogicalModel,
        OperationalTimingInstrumented, OperationalTimingReport, OperationalTimingSample,
        OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit, Result,
''',
    '''        AmplificationInstrumented, AmplificationRatio, AmplificationReport, ConcurrencyMode,
        CrashRecovery, DbError, DistributionMode, EngineCapabilities, KvEngine, LogicalModel,
        OperationalAttemptOutcome, OperationalAttemptSample, OperationalTimingInstrumented,
        OperationalTimingReport, OperationalTimingSample, OperationalWork, OperationalWorkUnit,
        Persistence, ReadWorkUnit, Result,
''',
    "experiment core test imports",
)
replace_once(
    EXPERIMENT,
    '''        operational_timing: OperationalTimingReport,
    }
''',
    '''        operational_timing: OperationalTimingReport,
        fail_reopen: bool,
    }
''',
    "fake fail field",
)
replace_once(
    EXPERIMENT,
    '''                operational_timing: OperationalTimingReport::default(),
            }
        }
    }
''',
    '''                operational_timing: OperationalTimingReport::default(),
                fail_reopen: false,
            }
        }
    }
''',
    "fake fail init",
)
replace_once(
    EXPERIMENT,
    '''        fn reopen(&mut self) -> Result<()> {
            self.operational_timing.reopen_ns.push(1);
            self.operational_timing
                .reopen_samples
                .push(OperationalTimingSample {
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
    '''        fn reopen(&mut self) -> Result<()> {
            let work = OperationalWork {
                unit: if self.architecture == StorageArchitecture::BPlusTree {
                    OperationalWorkUnit::BtreePageAccess
                } else {
                    OperationalWorkUnit::LsmRecordVersion
                },
                units_examined: 1,
                bytes_examined: 1,
            };
            if self.fail_reopen {
                let error = DbError::Io(std::io::Error::other("injected fake reopen failure"));
                self.operational_timing.reopen_attempts.push(OperationalAttemptSample {
                    measured_step_index: self.operational_step_index,
                    duration_ns: 1,
                    work: Some(work),
                    outcome: OperationalAttemptOutcome::Failed {
                        error_class: error.class(),
                        message: error.to_string(),
                    },
                });
                return Err(error);
            }
            self.operational_timing.reopen_ns.push(1);
            self.operational_timing
                .reopen_samples
                .push(OperationalTimingSample {
                    measured_step_index: self.operational_step_index,
                    duration_ns: 1,
                    work,
                });
            self.operational_timing.reopen_attempts.push(OperationalAttemptSample {
                measured_step_index: self.operational_step_index,
                duration_ns: 1,
                work: Some(work),
                outcome: OperationalAttemptOutcome::Succeeded,
            });
            Ok(())
        }
''',
    "fake reopen attempt evidence",
)
# Insert tests before FakeEngine struct.
replace_once(
    EXPERIMENT,
    '''    struct FakeEngine {
''',
    '''    #[test]
    fn batch_counterbalances_included_attempts_and_retains_warmups() {
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 0x55,
            profile: ExperimentProfile::RandomWrite,
            operations: 4,
            key_space: 4,
            value_bytes: 4,
            range_limit: 1,
            reopen_every: Some(2),
        })
        .expect("trace");
        let config = ExperimentBatchConfig {
            included_attempts: 6,
            warmup_attempts: 2,
            order_seed: 0xdead_beef,
        };
        let mut observed_orders = Vec::new();
        let report = compare_experiment_batch(&trace, config, |index, order| {
            observed_orders.push((index, order));
            Ok((
                FakeEngine::new("left", StorageArchitecture::BPlusTree),
                FakeEngine::new("right", StorageArchitecture::LsmTree),
            ))
        })
        .expect("batch");
        assert_eq!(report.attempts.len(), 8);
        assert_eq!(report.summary.included_successes, 6);
        assert_eq!(report.summary.included_failures, 0);
        assert_eq!(report.summary.excluded_warmups, 2);
        assert_eq!(report.summary.included_left_first, 3);
        assert_eq!(report.summary.included_right_first, 3);
        assert!(report.canonical_outcomes.is_some());
        assert_eq!(observed_orders.len(), 8);
        assert!(report.attempts[..2]
            .iter()
            .all(|attempt| attempt.disposition == ExperimentAttemptDisposition::ExcludedWarmup));
        for pair in report.attempts[2..].chunks_exact(2) {
            assert_ne!(pair[0].execution_order, pair[1].execution_order);
        }
    }

    #[test]
    fn batch_retains_factory_and_runtime_failures_with_partial_timing() {
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 0x77,
            profile: ExperimentProfile::RandomWrite,
            operations: 2,
            key_space: 2,
            value_bytes: 4,
            range_limit: 1,
            reopen_every: Some(1),
        })
        .expect("trace");
        let report = compare_experiment_batch(
            &trace,
            ExperimentBatchConfig {
                included_attempts: 4,
                warmup_attempts: 0,
                order_seed: 0,
            },
            |index, _order| {
                if index == 1 {
                    return Err(DbError::Io(std::io::Error::other(
                        "injected factory failure",
                    )));
                }
                let mut left = FakeEngine::new("left", StorageArchitecture::BPlusTree);
                left.fail_reopen = index == 2;
                Ok((left, FakeEngine::new("right", StorageArchitecture::LsmTree)))
            },
        )
        .expect("batch report survives per-attempt failures");
        assert_eq!(report.summary.included_successes, 2);
        assert_eq!(report.summary.included_failures, 2);
        match &report.attempts[1].result {
            ExperimentAttemptResult::Failed {
                stage,
                left_operational_timing,
                right_operational_timing,
                ..
            } => {
                assert_eq!(*stage, ExperimentAttemptFailureStage::Factory);
                assert!(left_operational_timing.is_none());
                assert!(right_operational_timing.is_none());
            }
            other => panic!("expected retained factory failure, got {other:?}"),
        }
        match &report.attempts[2].result {
            ExperimentAttemptResult::Failed {
                stage,
                left_operational_timing,
                ..
            } => {
                assert_eq!(*stage, ExperimentAttemptFailureStage::Comparison);
                let timing = left_operational_timing
                    .as_ref()
                    .expect("partial left timing retained");
                assert!(timing.reopen_attempts.iter().any(|attempt| matches!(
                    attempt.outcome,
                    OperationalAttemptOutcome::Failed { .. }
                )));
            }
            other => panic!("expected retained runtime failure, got {other:?}"),
        }
    }

    #[test]
    fn batch_requires_even_included_attempt_count() {
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 1,
            profile: ExperimentProfile::RandomWrite,
            operations: 1,
            key_space: 1,
            value_bytes: 1,
            range_limit: 1,
            reopen_every: None,
        })
        .expect("trace");
        let error = ExperimentBatchConfig {
            included_attempts: 3,
            warmup_attempts: 0,
            order_seed: 0,
        }
        .validate(&trace)
        .expect_err("odd included count must fail");
        assert!(error.to_string().contains("positive even"));
    }

    struct FakeEngine {
''',
    "batch tests",
)

# ---------- db-core exports ----------
replace_once(
    CORE_LIB,
    '''    OperationalTimingSample, OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit,
''',
    '''    OperationalAttemptOutcome, OperationalAttemptSample, OperationalTimingSample, OperationalWork,
    OperationalWorkUnit, Persistence, ReadWorkUnit,
''',
    "core operational exports",
)
replace_once(
    CORE_LIB,
    '''    compare_experiment_trace, execute_experiment_step, generate_experiment_trace,
    run_experiment_trace, ExperimentComparisonReport, ExperimentEngineEvidence,
    ExperimentGeneratorConfig, ExperimentOutcome, ExperimentProfile, ExperimentRow,
    ExperimentRunReport, ExperimentStep, ExperimentTrace, EXPERIMENT_TRACE_FORMAT_VERSION,
    MAX_EXPERIMENT_OUTCOME_PAYLOAD_BYTES, MAX_EXPERIMENT_RANGE_LIMIT, MAX_EXPERIMENT_STEPS,
    MAX_EXPERIMENT_TRACE_PAYLOAD_BYTES,
''',
    '''    compare_experiment_batch, compare_experiment_trace, compare_experiment_trace_ordered,
    execute_experiment_step, generate_experiment_trace, run_experiment_trace, ExperimentAttemptDisposition,
    ExperimentAttemptFailureStage, ExperimentAttemptResult, ExperimentBatchAttempt, ExperimentBatchConfig,
    ExperimentBatchReport, ExperimentBatchSummary, ExperimentComparisonReport, ExperimentEngineEvidence,
    ExperimentExecutionOrder, ExperimentGeneratorConfig, ExperimentOutcome, ExperimentProfile,
    ExperimentRow, ExperimentRunReport, ExperimentStep, ExperimentTrace, EXPERIMENT_BATCH_FORMAT_VERSION,
    EXPERIMENT_TRACE_FORMAT_VERSION, MAX_EXPERIMENT_BATCH_ATTEMPTS,
    MAX_EXPERIMENT_BATCH_MEASURED_STEP_EXECUTIONS, MAX_EXPERIMENT_OUTCOME_PAYLOAD_BYTES,
    MAX_EXPERIMENT_RANGE_LIMIT, MAX_EXPERIMENT_STEPS, MAX_EXPERIMENT_TRACE_PAYLOAD_BYTES,
''',
    "core experiment exports",
)

# ---------- B+ tree attempt stream ----------
replace_once(
    BTREE_COMMON,
    '''    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalTimingReport, OperationalTimingSample, OperationalWork, OperationalWorkUnit,
    Persistence, StorageArchitecture,
''',
    '''    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalAttemptOutcome,
    OperationalAttemptSample, OperationalTimingInstrumented, OperationalTimingReport,
    OperationalTimingSample, OperationalWork, OperationalWorkUnit, Persistence, StorageArchitecture,
''',
    "btree attempt imports",
)
replace_once(
    BTREE_COMMON,
    '''                operational_timing
                    .reopen_samples
                    .push(OperationalTimingSample {
                        measured_step_index: operational_step_index,
                        duration_ns,
                        work: OperationalWork {
                            unit: OperationalWorkUnit::BtreePageAccess,
                            units_examined: page_accesses,
                            bytes_examined: page_accesses.saturating_mul(PAGE_SIZE as u64),
                        },
                    });
''',
    '''                let work = OperationalWork {
                    unit: OperationalWorkUnit::BtreePageAccess,
                    units_examined: page_accesses,
                    bytes_examined: page_accesses.saturating_mul(PAGE_SIZE as u64),
                };
                operational_timing
                    .reopen_samples
                    .push(OperationalTimingSample {
                        measured_step_index: operational_step_index,
                        duration_ns,
                        work,
                    });
                operational_timing.reopen_attempts.push(OperationalAttemptSample {
                    measured_step_index: operational_step_index,
                    duration_ns,
                    work: Some(work),
                    outcome: OperationalAttemptOutcome::Succeeded,
                });
''',
    "btree successful reopen attempt",
)
replace_once(
    BTREE_COMMON,
    '''            Err(error) => {
                self.pager.poisoned = true;
                Err(common_error(error))
            }
''',
    '''            Err(error) => {
                let error = common_error(error);
                let duration_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                self.operational_timing.reopen_attempts.push(OperationalAttemptSample {
                    measured_step_index: operational_step_index,
                    duration_ns,
                    work: None,
                    outcome: OperationalAttemptOutcome::Failed {
                        error_class: error.class(),
                        message: error.to_string(),
                    },
                });
                self.pager.poisoned = true;
                Err(error)
            }
''',
    "btree failed reopen attempt",
)

# ---------- LSM reopen + compaction attempt stream ----------
replace_once(
    LSM,
    '''    EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalTimingReport, OperationalTimingSample, OperationalWork, OperationalWorkUnit,
    Persistence, ReadWorkUnit, Result, StorageArchitecture, StructuralReadAmplification,
''',
    '''    EngineCapabilities, KvEngine, LogicalModel, OperationalAttemptOutcome,
    OperationalAttemptSample, OperationalTimingInstrumented, OperationalTimingReport,
    OperationalTimingSample, OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit, Result,
    StorageArchitecture, StructuralReadAmplification,
''',
    "lsm attempt imports",
)
replace_once(
    LSM,
    '''                operational_timing
                    .reopen_samples
                    .push(OperationalTimingSample {
                        measured_step_index: operational_step_index,
                        duration_ns,
                        work,
                    });
''',
    '''                operational_timing
                    .reopen_samples
                    .push(OperationalTimingSample {
                        measured_step_index: operational_step_index,
                        duration_ns,
                        work,
                    });
                operational_timing.reopen_attempts.push(OperationalAttemptSample {
                    measured_step_index: operational_step_index,
                    duration_ns,
                    work: Some(work),
                    outcome: OperationalAttemptOutcome::Succeeded,
                });
''',
    "lsm successful reopen attempt",
)
replace_once(
    LSM,
    '''            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
''',
    '''            Err(error) => {
                let duration_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                self.operational_timing.reopen_attempts.push(OperationalAttemptSample {
                    measured_step_index: operational_step_index,
                    duration_ns,
                    work: None,
                    outcome: OperationalAttemptOutcome::Failed {
                        error_class: error.class(),
                        message: error.to_string(),
                    },
                });
                self.poisoned = true;
                Err(error)
            }
''',
    "lsm failed reopen attempt",
)
# Wrap full-set compaction body so every triggered attempt gets success/failure evidence.
replace_once(
    LSM,
    '''        self.instrumentation.compaction_input_sstable_bytes = self
            .instrumentation
            .compaction_input_sstable_bytes
            .saturating_add(input_bytes);
        let mut merged = BTreeMap::new();
''',
    '''        self.instrumentation.compaction_input_sstable_bytes = self
            .instrumentation
            .compaction_input_sstable_bytes
            .saturating_add(input_bytes);
        let compaction_result = (|| -> Result<()> {
        let mut merged = BTreeMap::new();
''',
    "lsm compaction closure start",
)
replace_once(
    LSM,
    '''        self.reclaim_obsolete_sstables(active_table_id);
        self.reclaim_obsolete_manifests(active_manifest_id);
        let duration_ns =
            u64::try_from(compaction_started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.operational_timing
            .compaction_stall_ns
            .push(duration_ns);
        self.operational_timing
            .compaction_stall_samples
            .push(OperationalTimingSample {
                measured_step_index: self.operational_step_index,
                duration_ns,
                work: OperationalWork {
                    unit: OperationalWorkUnit::LsmSstableRecordVersion,
                    units_examined: input_records,
                    bytes_examined: input_bytes,
                },
            });
        Ok(())
    }
''',
    '''        self.reclaim_obsolete_sstables(active_table_id);
        self.reclaim_obsolete_manifests(active_manifest_id);
        Ok(())
        })();
        let duration_ns =
            u64::try_from(compaction_started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let work = OperationalWork {
            unit: OperationalWorkUnit::LsmSstableRecordVersion,
            units_examined: input_records,
            bytes_examined: input_bytes,
        };
        match compaction_result {
            Ok(()) => {
                self.operational_timing
                    .compaction_stall_ns
                    .push(duration_ns);
                self.operational_timing
                    .compaction_stall_samples
                    .push(OperationalTimingSample {
                        measured_step_index: self.operational_step_index,
                        duration_ns,
                        work,
                    });
                self.operational_timing
                    .compaction_stall_attempts
                    .push(OperationalAttemptSample {
                        measured_step_index: self.operational_step_index,
                        duration_ns,
                        work: Some(work),
                        outcome: OperationalAttemptOutcome::Succeeded,
                    });
                Ok(())
            }
            Err(error) => {
                self.operational_timing
                    .compaction_stall_attempts
                    .push(OperationalAttemptSample {
                        measured_step_index: self.operational_step_index,
                        duration_ns,
                        work: Some(work),
                        outcome: OperationalAttemptOutcome::Failed {
                            error_class: error.class(),
                            message: error.to_string(),
                        },
                    });
                Err(error)
            }
        }
    }
''',
    "lsm compaction attempt outcome",
)

# ---------- CLI batch archive ----------
replace_once(
    CLI,
    '''    compare_experiment_trace, compare_workload, execute_workload, generate_experiment_trace,
    generate_workload, DbError, DifferentialError, ExperimentComparisonReport,
    ExperimentGeneratorConfig, ExperimentProfile, ExperimentTrace, GeneratorConfig, KvEngine,
    Outcome, Workload,
''',
    '''    compare_experiment_batch, compare_experiment_trace, compare_workload, execute_workload,
    generate_experiment_trace, generate_workload, DbError, DifferentialError, ExperimentBatchConfig,
    ExperimentBatchReport, ExperimentComparisonReport, ExperimentExecutionOrder,
    ExperimentGeneratorConfig, ExperimentProfile, ExperimentTrace, GeneratorConfig, KvEngine, Outcome,
    Workload,
''',
    "cli batch imports",
)
# Add command before Verify.
replace_once(
    CLI,
    '''    /// Validate an append-log file without modifying it.
    Verify {
''',
    '''    /// Run repeated fresh-engine comparisons with deterministic AB/BA order and archive every attempt.
    ExperimentBatchArchive {
        /// Versioned experiment trace JSON file.
        #[arg(long)]
        trace: PathBuf,
        /// New workspace directory containing one fresh B+ tree/LSM target pair per attempt.
        #[arg(long)]
        workspace_dir: PathBuf,
        /// New immutable evidence archive directory.
        #[arg(long)]
        archive_dir: PathBuf,
        /// Positive even number of attempts included in measured distributions.
        #[arg(long, default_value_t = 10)]
        included_attempts: u32,
        /// Warmup attempts executed and archived but explicitly excluded from distributions.
        #[arg(long, default_value_t = 2)]
        warmup_attempts: u32,
        /// Seed controlling deterministic AB/BA pair orientation.
        #[arg(long)]
        order_seed: u64,
        /// B+ tree validated-page cache capacity.
        #[arg(long, default_value_t = 64)]
        btree_cache_pages: usize,
        /// Exact source revision represented by the binary/run.
        #[arg(long)]
        revision: String,
        /// Human-readable host identity without secrets.
        #[arg(long)]
        host_label: Option<String>,
        /// Filesystem under test, when known.
        #[arg(long)]
        filesystem: Option<String>,
        /// Storage device/model label, without credentials or serial numbers.
        #[arg(long)]
        storage_device: Option<String>,
        /// Declared cache preparation state for this run.
        #[arg(long, value_enum, default_value_t = CacheStateKind::Unspecified)]
        cache_state: CacheStateKind,
        /// Optional free-form experiment note. Do not include secrets.
        #[arg(long)]
        notes: Option<String>,
    },
    /// Validate an append-log file without modifying it.
    Verify {
''',
    "cli batch command",
)
# Add batch archive index struct.
replace_once(
    CLI,
    '''struct EvidenceArchiveIndex {
    format_version: u16,
    repository_revision: String,
    files: [&'static str; 3],
}
''',
    '''struct EvidenceArchiveIndex {
    format_version: u16,
    repository_revision: String,
    files: [&'static str; 3],
}

const BATCH_EVIDENCE_ARCHIVE_FORMAT_VERSION: u16 = 1;

#[derive(Debug, Serialize)]
struct BatchEvidenceArchiveIndex {
    format_version: u16,
    repository_revision: String,
    files: [&'static str; 3],
}
''',
    "batch archive index",
)
# Add match arm before Verify.
replace_once(
    CLI,
    '''        Command::Verify { path } => {
''',
    '''        Command::ExperimentBatchArchive {
            trace,
            workspace_dir,
            archive_dir,
            included_attempts,
            warmup_attempts,
            order_seed,
            btree_cache_pages,
            revision,
            host_label,
            filesystem,
            storage_device,
            cache_state,
            notes,
        } => {
            validate_revision(&revision)?;
            let trace = read_experiment_trace(&trace)?;
            let config = ExperimentBatchConfig {
                included_attempts,
                warmup_attempts,
                order_seed,
            };
            config.validate(&trace)?;
            ensure_fresh_batch_targets(&workspace_dir, &archive_dir)?;
            fs::create_dir(&workspace_dir)?;
            let batch = compare_experiment_batch(&trace, config, |attempt_index, order| {
                let attempt_dir = workspace_dir.join(format!("attempt-{attempt_index:06}"));
                fs::create_dir(&attempt_dir).map_err(DbError::Io)?;
                let btree_path = attempt_dir.join("btree.db");
                let lsm_path = attempt_dir.join("lsm");
                let create_btree = || {
                    BPlusTree::create_new(&btree_path, btree_cache_pages).map_err(btree_error_to_db)
                };
                let create_lsm = || LsmEngine::create_new(&lsm_path);
                match order {
                    ExperimentExecutionOrder::LeftFirst => {
                        let btree = create_btree()?;
                        let lsm = create_lsm()?;
                        Ok((btree, lsm))
                    }
                    ExperimentExecutionOrder::RightFirst => {
                        let lsm = create_lsm()?;
                        let btree = create_btree()?;
                        Ok((btree, lsm))
                    }
                }
            })?;
            let environment = EvidenceArchiveEnvironment {
                format_version: EVIDENCE_ARCHIVE_FORMAT_VERSION,
                repository_revision: revision.clone(),
                db_lab_version: env!("CARGO_PKG_VERSION"),
                target_os: std::env::consts::OS,
                target_arch: std::env::consts::ARCH,
                build_profile: if cfg!(debug_assertions) { "debug" } else { "release" },
                rustc_version: rustc_version(),
                host_label,
                filesystem,
                storage_device,
                cache_state: cache_state.as_str(),
                btree_cache_pages,
                recorded_unix_seconds: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|error| CliError::Usage(format!("system clock precedes Unix epoch: {error}")))?
                    .as_secs(),
                notes,
            };
            write_batch_evidence_archive(&archive_dir, &revision, &trace, &batch, &environment)
        }
        Command::Verify { path } => {
''',
    "cli batch arm",
)
# Helper mapping + fresh paths + archive writer before validate_revision.
replace_once(
    CLI,
    '''fn validate_revision(revision: &str) -> Result<(), CliError> {
''',
    '''fn btree_error_to_db(error: BtreeError) -> DbError {
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

fn ensure_fresh_batch_targets(workspace_dir: &Path, archive_dir: &Path) -> Result<(), CliError> {
    if workspace_dir == archive_dir {
        return Err(CliError::Usage(
            "batch workspace and archive directories must be distinct".to_owned(),
        ));
    }
    for (label, path) in [("workspace", workspace_dir), ("archive", archive_dir)] {
        if path.exists() {
            return Err(CliError::Usage(format!(
                "experiment batch {label} path already exists: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_revision(revision: &str) -> Result<(), CliError> {
''',
    "cli batch helpers",
)
# Add batch writer before existing write_evidence_archive.
replace_once(
    CLI,
    '''fn write_evidence_archive(
''',
    '''fn write_batch_evidence_archive(
    archive_dir: &Path,
    revision: &str,
    trace: &ExperimentTrace,
    batch: &ExperimentBatchReport,
    environment: &EvidenceArchiveEnvironment,
) -> Result<(), CliError> {
    fs::create_dir(archive_dir)?;
    let result = (|| {
        write_new_json(&archive_dir.join("trace.json"), trace)?;
        write_new_json(&archive_dir.join("batch.json"), batch)?;
        write_new_json(&archive_dir.join("environment.json"), environment)?;
        write_new_json(
            &archive_dir.join("index.json"),
            &BatchEvidenceArchiveIndex {
                format_version: BATCH_EVIDENCE_ARCHIVE_FORMAT_VERSION,
                repository_revision: revision.to_owned(),
                files: ["trace.json", "batch.json", "environment.json"],
            },
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(archive_dir);
    }
    result
}

fn write_evidence_archive(
''',
    "batch archive writer",
)

print("applied Phase 4 counterbalanced attempt ledger implementation")
