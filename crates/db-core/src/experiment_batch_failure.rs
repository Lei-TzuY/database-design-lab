use serde::Serialize;

use crate::{
    compare_experiment_trace_counterbalanced_with_failure_evidence, AmplificationInstrumented,
    CounterbalancedComparisonFailureEvidence, CounterbalancedExperimentBatchReport,
    CounterbalancedExperimentComparisonReport, CounterbalancedPairOrder, DbError,
    ExperimentAttemptAdmission, ExperimentAttemptContext, ExperimentAttemptDisposition,
    ExperimentAttemptFailure, ExperimentAttemptFailureStage, ExperimentAttemptRecord,
    ExperimentEngineRole, ExperimentInstanceContext, ExperimentTrace, KvEngine,
    OperationalTimingInstrumented, MAX_EXPERIMENT_BATCH_PAIRS,
};

/// Stable batch ledger plus sidecar-ready operational telemetry captured at comparison failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CounterbalancedExperimentBatchFailureEvidenceReport {
    /// Existing batch report shape; legacy callers can keep archiving this object unchanged.
    pub batch: CounterbalancedExperimentBatchReport,
    /// Comparison/runtime failures for which both fresh engines existed and timing reports were snapshot.
    pub comparison_failures: Vec<CounterbalancedComparisonFailureEvidence>,
}

/// Runs repeated fresh counterbalanced pairs and preserves timing reports at comparison failure boundaries.
///
/// Inclusion/exclusion/failure semantics match `run_counterbalanced_experiment_batch`. Factory failures remain
/// in the nested batch ledger but carry no comparison telemetry because both engine instances did not exist.
pub fn run_counterbalanced_experiment_batch_with_failure_evidence<
    L,
    R,
    MakeLeft,
    MakeRight,
    Admit,
>(
    trace: &ExperimentTrace,
    pair_seed: u64,
    requested_pairs: u32,
    mut make_left: MakeLeft,
    mut make_right: MakeRight,
    mut admit: Admit,
) -> std::result::Result<CounterbalancedExperimentBatchFailureEvidenceReport, DbError>
where
    L: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    R: KvEngine + AmplificationInstrumented + OperationalTimingInstrumented,
    MakeLeft: FnMut(ExperimentInstanceContext) -> std::result::Result<L, DbError>,
    MakeRight: FnMut(ExperimentInstanceContext) -> std::result::Result<R, DbError>,
    Admit: FnMut(ExperimentAttemptContext) -> ExperimentAttemptAdmission,
{
    trace.validate()?;
    if requested_pairs == 0 || requested_pairs > MAX_EXPERIMENT_BATCH_PAIRS {
        return Err(DbError::InvalidInput(format!(
            "experiment batch pairs is {requested_pairs}; expected 1..={MAX_EXPERIMENT_BATCH_PAIRS}"
        )));
    }

    let mut attempts = Vec::with_capacity(requested_pairs as usize);
    let mut comparison_failures = Vec::new();
    let mut included_pairs = 0_u32;
    let mut failed_pairs = 0_u32;
    let mut excluded_pairs = 0_u32;

    for pair_index in 0..requested_pairs {
        let pair_order = pair_order(pair_seed, pair_index);
        let context = ExperimentAttemptContext {
            pair_index,
            pair_order,
        };
        match admit(context) {
            ExperimentAttemptAdmission::Exclude { reason } => {
                let reason = reason.trim().to_owned();
                if reason.is_empty() {
                    return Err(DbError::InvalidInput(format!(
                        "experiment batch exclusion for pair {pair_index} must include a non-empty reason"
                    )));
                }
                excluded_pairs = excluded_pairs.saturating_add(1);
                attempts.push(ExperimentAttemptRecord {
                    context,
                    disposition: ExperimentAttemptDisposition::Excluded,
                    report: None,
                    failure: None,
                    exclusion_reason: Some(reason),
                });
            }
            ExperimentAttemptAdmission::Include => {
                let mut left_repetition = 0_u8;
                let mut right_repetition = 0_u8;
                let mut left_factory_failure = None;
                let mut right_factory_failure = None;
                let result = compare_experiment_trace_counterbalanced_with_failure_evidence(
                    trace,
                    pair_order,
                    || {
                        let repetition_index = left_repetition;
                        left_repetition = left_repetition.saturating_add(1);
                        let instance = ExperimentInstanceContext {
                            attempt: context,
                            repetition_index,
                        };
                        make_left(instance).inspect_err(|error| {
                            left_factory_failure.get_or_insert_with(|| ExperimentAttemptFailure {
                                stage: ExperimentAttemptFailureStage::EngineFactory,
                                engine_role: Some(ExperimentEngineRole::Left),
                                repetition_index: Some(repetition_index),
                                class: error.class(),
                                message: error.to_string(),
                            });
                        })
                    },
                    || {
                        let repetition_index = right_repetition;
                        right_repetition = right_repetition.saturating_add(1);
                        let instance = ExperimentInstanceContext {
                            attempt: context,
                            repetition_index,
                        };
                        make_right(instance).inspect_err(|error| {
                            right_factory_failure.get_or_insert_with(|| ExperimentAttemptFailure {
                                stage: ExperimentAttemptFailureStage::EngineFactory,
                                engine_role: Some(ExperimentEngineRole::Right),
                                repetition_index: Some(repetition_index),
                                class: error.class(),
                                message: error.to_string(),
                            });
                        })
                    },
                );
                match result {
                    Ok(report) => {
                        included_pairs = included_pairs.saturating_add(1);
                        attempts.push(included_record(context, report));
                    }
                    Err(error) => {
                        failed_pairs = failed_pairs.saturating_add(1);
                        if let Some(evidence) = error.comparison_failure().cloned() {
                            comparison_failures.push(evidence);
                        }
                        let class = error.class();
                        let message = error.to_string();
                        let failure = left_factory_failure
                            .or(right_factory_failure)
                            .unwrap_or(ExperimentAttemptFailure {
                                stage: ExperimentAttemptFailureStage::Comparison,
                                engine_role: None,
                                repetition_index: None,
                                class,
                                message,
                            });
                        attempts.push(ExperimentAttemptRecord {
                            context,
                            disposition: ExperimentAttemptDisposition::Failed,
                            report: None,
                            failure: Some(failure),
                            exclusion_reason: None,
                        });
                    }
                }
            }
        }
    }

    Ok(CounterbalancedExperimentBatchFailureEvidenceReport {
        batch: CounterbalancedExperimentBatchReport {
            trace: trace.clone(),
            pair_seed,
            requested_pairs,
            included_pairs,
            failed_pairs,
            excluded_pairs,
            attempts,
        },
        comparison_failures,
    })
}

fn included_record(
    context: ExperimentAttemptContext,
    report: CounterbalancedExperimentComparisonReport,
) -> ExperimentAttemptRecord {
    ExperimentAttemptRecord {
        context,
        disposition: ExperimentAttemptDisposition::Included,
        report: Some(report),
        failure: None,
        exclusion_reason: None,
    }
}

fn pair_order(seed: u64, pair_index: u32) -> CounterbalancedPairOrder {
    if ((seed & 1) ^ u64::from(pair_index & 1)) == 0 {
        CounterbalancedPairOrder::LeftThenRightFirst
    } else {
        CounterbalancedPairOrder::RightThenLeftFirst
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::run_counterbalanced_experiment_batch_with_failure_evidence;
    use crate::{
        generate_experiment_trace, AmplificationInstrumented, AmplificationRatio,
        AmplificationReport, ConcurrencyMode, CrashRecovery, DbError, DistributionMode,
        EngineCapabilities, ExperimentAttemptAdmission, ExperimentGeneratorConfig, ExperimentProfile,
        KvEngine, LogicalModel, OperationalTimingFailureSample, OperationalTimingInstrumented,
        OperationalTimingReport, OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit,
        Result, StorageArchitecture, StructuralReadAmplification, MAX_KEY_BYTES, MAX_VALUE_BYTES,
    };

    #[test]
    fn batch_retains_comparison_failure_telemetry_and_continues() {
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 91,
            profile: ExperimentProfile::RandomWrite,
            operations: 1,
            key_space: 4,
            value_bytes: 1,
            range_limit: 1,
            reopen_every: None,
        })
        .expect("trace");

        let report = run_counterbalanced_experiment_batch_with_failure_evidence(
            &trace,
            0,
            2,
            |context| {
                Ok(FailingEngine::new(
                    "left",
                    StorageArchitecture::BPlusTree,
                    false,
                    context.attempt.pair_index,
                ))
            },
            |context| {
                Ok(FailingEngine::new(
                    "right",
                    StorageArchitecture::LsmTree,
                    true,
                    context.attempt.pair_index,
                ))
            },
            |_| ExperimentAttemptAdmission::Include,
        )
        .expect("batch");

        assert_eq!(report.batch.requested_pairs, 2);
        assert_eq!(report.batch.failed_pairs, 1);
        assert_eq!(report.batch.included_pairs, 1);
        assert_eq!(report.comparison_failures.len(), 1);
        assert_eq!(report.comparison_failures[0].repetition_index, 0);
        assert_eq!(
            report.comparison_failures[0]
                .right_operational_timing
                .compaction_stall_failure_samples[0]
                .measured_step_index,
            Some(0)
        );
    }

    struct FailingEngine {
        name: &'static str,
        architecture: StorageArchitecture,
        fail_put: bool,
        pair_index: u32,
        map: BTreeMap<Vec<u8>, Vec<u8>>,
        timing: OperationalTimingReport,
        step: Option<u64>,
    }

    impl FailingEngine {
        fn new(
            name: &'static str,
            architecture: StorageArchitecture,
            fail_put: bool,
            pair_index: u32,
        ) -> Self {
            Self {
                name,
                architecture,
                fail_put,
                pair_index,
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
            if self.fail_put && self.pair_index == 0 {
                self.timing
                    .compaction_stall_failure_samples
                    .push(OperationalTimingFailureSample {
                        measured_step_index: self.step,
                        duration_ns: 11,
                        work: Some(OperationalWork {
                            unit: OperationalWorkUnit::LsmSstableRecordVersion,
                            units_examined: 4,
                            bytes_examined: 128,
                        }),
                        error_class: crate::ErrorClass::Io,
                    });
                return Err(DbError::Io(std::io::Error::other(
                    "injected failed compaction",
                )));
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
