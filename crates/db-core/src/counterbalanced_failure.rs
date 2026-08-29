use std::fmt;

use serde::Serialize;

use crate::{
    compare_experiment_trace_ordered, AmplificationInstrumented,
    CounterbalancedExperimentComparisonReport, CounterbalancedPairOrder, DbError, ErrorClass,
    ExperimentExecutionOrder, ExperimentTrace, KvEngine, OperationalTimingInstrumented,
    OperationalTimingReport, OrderedExperimentComparisonReport, Result,
};

/// Raw operational telemetry retained when one ordered comparison fails after both engines exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CounterbalancedComparisonFailureEvidence {
    /// Pair-level outer-order provenance.
    pub pair_order: CounterbalancedPairOrder,
    /// Zero for the first ordered comparison in the pair, one for the second.
    pub repetition_index: u8,
    /// Whole-engine order active when the comparison failed.
    pub execution_order: ExperimentExecutionOrder,
    /// Stable coarse failure class.
    pub class: ErrorClass,
    /// Diagnostic error text from the failed ordered comparison.
    pub message: String,
    /// Process-local left-engine operational report at the failure boundary.
    pub left_operational_timing: OperationalTimingReport,
    /// Process-local right-engine operational report at the failure boundary.
    pub right_operational_timing: OperationalTimingReport,
}

/// Counterbalanced execution error that may carry failure-boundary operational telemetry.
#[derive(Debug)]
pub struct CounterbalancedExperimentExecutionError {
    error: DbError,
    comparison_failure: Option<CounterbalancedComparisonFailureEvidence>,
}

impl CounterbalancedExperimentExecutionError {
    /// Stable error class without parsing diagnostic text.
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        self.error.class()
    }

    /// Failure-boundary telemetry when both engines had been created and ordered execution started.
    #[must_use]
    pub fn comparison_failure(&self) -> Option<&CounterbalancedComparisonFailureEvidence> {
        self.comparison_failure.as_ref()
    }

    /// Consumes the wrapper and returns the original database error.
    #[must_use]
    pub fn into_db_error(self) -> DbError {
        self.error
    }
}

impl fmt::Display for CounterbalancedExperimentExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for CounterbalancedExperimentExecutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Runs one fresh counterbalanced pair while retaining operational reports at comparison failure.
///
/// Factory failures carry no comparison telemetry because one or both engine instances do not yet
/// exist. Once both engines exist, an ordered-comparison error snapshots both process-local timing
/// reports before the instances are dropped. Successful behavior and report shape match
/// `compare_experiment_trace_counterbalanced` exactly.
pub fn compare_experiment_trace_counterbalanced_with_failure_evidence<L, R, MakeLeft, MakeRight>(
    trace: &ExperimentTrace,
    pair_order: CounterbalancedPairOrder,
    mut make_left: MakeLeft,
    mut make_right: MakeRight,
) -> std::result::Result<CounterbalancedExperimentComparisonReport, CounterbalancedExperimentExecutionError>
where
    L: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    R: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    MakeLeft: FnMut() -> Result<L>,
    MakeRight: FnMut() -> Result<R>,
{
    trace.validate().map_err(preflight_or_factory_error)?;

    let first_order = first_execution_order(pair_order);
    let mut first_left = make_left().map_err(preflight_or_factory_error)?;
    let mut first_right = make_right().map_err(preflight_or_factory_error)?;
    let first = compare_experiment_trace_ordered(
        &mut first_left,
        &mut first_right,
        trace,
        first_order,
    )
    .map_err(|error| {
        comparison_error(
            error,
            pair_order,
            0,
            first_order,
            &first_left,
            &first_right,
        )
    })?;

    let second_order = second_execution_order(pair_order);
    let mut second_left = make_left().map_err(preflight_or_factory_error)?;
    let mut second_right = make_right().map_err(preflight_or_factory_error)?;
    let second = compare_experiment_trace_ordered(
        &mut second_left,
        &mut second_right,
        trace,
        second_order,
    )
    .map_err(|error| {
        comparison_error(
            error,
            pair_order,
            1,
            second_order,
            &second_left,
            &second_right,
        )
    })?;

    validate_repetitions(&first, &second).map_err(preflight_or_factory_error)?;
    Ok(CounterbalancedExperimentComparisonReport {
        pair_order,
        first,
        second,
    })
}

fn first_execution_order(pair_order: CounterbalancedPairOrder) -> ExperimentExecutionOrder {
    match pair_order {
        CounterbalancedPairOrder::LeftThenRightFirst => ExperimentExecutionOrder::LeftThenRight,
        CounterbalancedPairOrder::RightThenLeftFirst => ExperimentExecutionOrder::RightThenLeft,
    }
}

fn second_execution_order(pair_order: CounterbalancedPairOrder) -> ExperimentExecutionOrder {
    match pair_order {
        CounterbalancedPairOrder::LeftThenRightFirst => ExperimentExecutionOrder::RightThenLeft,
        CounterbalancedPairOrder::RightThenLeftFirst => ExperimentExecutionOrder::LeftThenRight,
    }
}

fn validate_repetitions(
    first: &OrderedExperimentComparisonReport,
    second: &OrderedExperimentComparisonReport,
) -> Result<()> {
    if first.comparison.left.capabilities != second.comparison.left.capabilities {
        return Err(DbError::InvalidInput(format!(
            "counterbalanced left-engine capabilities changed between repetitions: {} then {}",
            first.comparison.left.capabilities.name, second.comparison.left.capabilities.name
        )));
    }
    if first.comparison.right.capabilities != second.comparison.right.capabilities {
        return Err(DbError::InvalidInput(format!(
            "counterbalanced right-engine capabilities changed between repetitions: {} then {}",
            first.comparison.right.capabilities.name, second.comparison.right.capabilities.name
        )));
    }
    if first.comparison.outcomes != second.comparison.outcomes {
        return Err(DbError::InvalidInput(
            "counterbalanced measured logical outcomes changed between repetitions".to_owned(),
        ));
    }
    Ok(())
}

fn preflight_or_factory_error(error: DbError) -> CounterbalancedExperimentExecutionError {
    CounterbalancedExperimentExecutionError {
        error,
        comparison_failure: None,
    }
}

fn comparison_error<L, R>(
    error: DbError,
    pair_order: CounterbalancedPairOrder,
    repetition_index: u8,
    execution_order: ExperimentExecutionOrder,
    left: &L,
    right: &R,
) -> CounterbalancedExperimentExecutionError
where
    L: OperationalTimingInstrumented,
    R: OperationalTimingInstrumented,
{
    let evidence = CounterbalancedComparisonFailureEvidence {
        pair_order,
        repetition_index,
        execution_order,
        class: error.class(),
        message: error.to_string(),
        left_operational_timing: left.operational_timing_report(),
        right_operational_timing: right.operational_timing_report(),
    };
    CounterbalancedExperimentExecutionError {
        error,
        comparison_failure: Some(evidence),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::compare_experiment_trace_counterbalanced_with_failure_evidence;
    use crate::{
        generate_experiment_trace, AmplificationInstrumented, AmplificationRatio,
        AmplificationReport, ConcurrencyMode, CounterbalancedPairOrder, CrashRecovery, DbError,
        DistributionMode, EngineCapabilities, ExperimentGeneratorConfig, ExperimentProfile, KvEngine,
        LogicalModel, OperationalTimingFailureSample, OperationalTimingInstrumented,
        OperationalTimingReport, OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit,
        Result, StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
    };

    #[test]
    fn comparison_failure_snapshots_both_engine_timing_reports() {
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 17,
            profile: ExperimentProfile::RandomWrite,
            operations: 1,
            key_space: 4,
            value_bytes: 1,
            range_limit: 1,
            reopen_every: None,
        })
        .expect("trace");

        let error = compare_experiment_trace_counterbalanced_with_failure_evidence(
            &trace,
            CounterbalancedPairOrder::LeftThenRightFirst,
            || Ok(FailingEngine::new("left", StorageArchitecture::BPlusTree, false)),
            || Ok(FailingEngine::new("right", StorageArchitecture::LsmTree, true)),
        )
        .expect_err("right engine must fail during first comparison");

        let evidence = error
            .comparison_failure()
            .expect("comparison failure telemetry");
        assert_eq!(evidence.repetition_index, 0);
        assert_eq!(evidence.class, crate::ErrorClass::InvalidInput);
        assert_eq!(
            evidence
                .right_operational_timing
                .compaction_stall_failure_samples
                .len(),
            1
        );
        assert!(evidence
            .left_operational_timing
            .compaction_stall_failure_samples
            .is_empty());
    }

    struct FailingEngine {
        name: &'static str,
        architecture: StorageArchitecture,
        fail_put: bool,
        map: BTreeMap<Vec<u8>, Vec<u8>>,
        timing: OperationalTimingReport,
        step: Option<u64>,
    }

    impl FailingEngine {
        fn new(name: &'static str, architecture: StorageArchitecture, fail_put: bool) -> Self {
            Self {
                name,
                architecture,
                fail_put,
                map: BTreeMap::new(),
                timing: OperationalTimingReport::default(),
                step: None,
            }
        }
    }

    impl KvEngine for FailingEngine {
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
            if self.fail_put {
                self.timing
                    .compaction_stall_failure_samples
                    .push(OperationalTimingFailureSample {
                        measured_step_index: self.step,
                        duration_ns: 7,
                        work: Some(OperationalWork {
                            unit: OperationalWorkUnit::LsmSstableRecordVersion,
                            units_examined: 3,
                            bytes_examined: 99,
                        }),
                        error_class: crate::ErrorClass::InvalidInput,
                    });
                return Err(DbError::InvalidInput("injected comparison failure".to_owned()));
            }
            Ok(self.map.insert(key.to_vec(), value.to_vec()))
        }

        fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            Ok(self.map.get(key).cloned())
        }

        fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            Ok(self.map.remove(key))
        }

        fn range_scan(
            &mut self,
            _start: &[u8],
            _end: Option<&[u8]>,
            _limit: usize,
        ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
            Ok(Vec::new())
        }

        fn reopen(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl OperationalTimingInstrumented for FailingEngine {
        fn reset_operational_timing(&mut self) {
            self.timing = OperationalTimingReport::default();
            self.step = None;
        }

        fn set_operational_step_index(&mut self, step_index: Option<u64>) {
            self.step = step_index;
        }

        fn operational_timing_report(&self) -> OperationalTimingReport {
            self.timing.clone()
        }
    }

    impl AmplificationInstrumented for FailingEngine {
        fn reset_amplification(&mut self) {}

        fn amplification_report(&mut self) -> Result<AmplificationReport> {
            Ok(AmplificationReport {
                point_read: StructuralReadAmplification {
                    ratio: AmplificationRatio::default(),
                    unit: ReadWorkUnit::BtreePageAccess,
                },
                range_read: StructuralReadAmplification {
                    ratio: AmplificationRatio::default(),
                    unit: ReadWorkUnit::BtreePageAccess,
                },
                data_write_bytes_per_logical_byte: AmplificationRatio::default(),
                primary_structure_bytes_per_live_byte: AmplificationRatio::default(),
            })
        }
    }
}
