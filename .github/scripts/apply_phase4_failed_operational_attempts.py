from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENGINE = ROOT / "crates/db-core/src/engine.rs"
CORE_LIB = ROOT / "crates/db-core/src/lib.rs"
BTREE_COMMON = ROOT / "crates/db-storage-btree/src/tree/common.rs"
BTREE_TEST = ROOT / "crates/db-storage-btree/src/tree/instrumentation_tests.rs"
LSM = ROOT / "crates/db-storage-lsm/src/lib.rs"
LSM_FAULT = ROOT / "crates/db-storage-lsm/src/compaction_fault_tests.rs"
README = ROOT / "README.md"
METHOD = ROOT / "docs/amplification-methodology.md"
ROADMAP = ROOT / "docs/roadmap.md"


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected 1 marker, found {count}")
    path.write_text(text.replace(old, new, 1))


# db-core: append-only attempt stream that preserves old success-only projections.
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
    /// The timed operation returned an error and therefore does not enter success-only distributions.
    Failed {
        /// Stable common error class.
        error_class: ErrorClass,
        /// Human-readable forensic detail from the returned error.
        message: String,
    },
}

/// One attempted synchronous operation, including failures excluded from compatibility vectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperationalAttemptSample {
    /// Zero-based measured experiment step that triggered this attempt, or `None` outside a measured runner.
    pub measured_step_index: Option<u64>,
    /// Wall-clock duration until the operation succeeded or returned failure.
    pub duration_ns: u64,
    /// Deterministic work when already known from the operation without extra measurement I/O.
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
    '''/// Duration plus deterministic work is evidence to archive, not a performance claim: failed/excluded attempts,
/// execution-order counterbalancing, cache/filesystem protocol, host pinning, and scheduler/device controls remain
/// required before durations are compared across engines or revisions.
''',
    '''/// Duration plus deterministic work is evidence to archive, not a performance claim. Fresh AB/BA whole-run
/// counterbalancing is available separately; exclusion policy, cache/filesystem protocol, host pinning, and
/// scheduler/device controls remain required before durations are compared across engines or revisions.
''',
    "operational report scope comment",
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
    /// Every triggered synchronous compaction attempt, including failures omitted from success projections.
    pub compaction_stall_attempts: Vec<OperationalAttemptSample>,
}
''',
    "operational attempt report fields",
)

replace_once(
    CORE_LIB,
    '''    KvEngine, LogicalModel, OperationalTimingInstrumented, OperationalTimingReport,
    OperationalTimingSample, OperationalWork, OperationalWorkUnit, Persistence, ReadWorkUnit,
''',
    '''    KvEngine, LogicalModel, OperationalAttemptOutcome, OperationalAttemptSample,
    OperationalTimingInstrumented, OperationalTimingReport, OperationalTimingSample, OperationalWork,
    OperationalWorkUnit, Persistence, ReadWorkUnit,
''',
    "core attempt exports",
)

# B+ tree: success mirrors old sample; failed reopen remains in the raw attempt stream.
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
    '''                operational_timing.reopen_ns.push(duration_ns);
                operational_timing
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
                operational_timing.reopen_ns.push(duration_ns);
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
    "btree successful attempt",
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
    "btree failed attempt",
)

replace_once(
    BTREE_TEST,
    '''    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalWorkUnit, Persistence, ReadWorkUnit, StorageArchitecture,
''',
    '''    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalAttemptOutcome,
    OperationalTimingInstrumented, OperationalWorkUnit, Persistence, ReadWorkUnit, StorageArchitecture,
''',
    "btree test attempt import",
)
replace_once(
    BTREE_TEST,
    '''    assert_eq!(sample.work.bytes_examined, 2 * PAGE_SIZE as u64);
    assert!(sample.duration_ns > 0);
}
''',
    '''    assert_eq!(sample.work.bytes_examined, 2 * PAGE_SIZE as u64);
    assert!(sample.duration_ns > 0);
    assert_eq!(timing.reopen_attempts.len(), 1);
    let attempt = &timing.reopen_attempts[0];
    assert_eq!(attempt.measured_step_index, Some(7));
    assert_eq!(attempt.duration_ns, sample.duration_ns);
    assert_eq!(attempt.work, Some(sample.work));
    assert_eq!(attempt.outcome, OperationalAttemptOutcome::Succeeded);
}
''',
    "btree attempt projection test",
)

# LSM: retain failed reopen and triggered compaction attempts.
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
    "lsm compaction attempt closure start",
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
    "lsm compaction attempt completion",
)

replace_once(
    LSM_FAULT,
    '''use db_core::{DbError, KvEngine, MAX_KEY_BYTES};
''',
    '''use db_core::{
    DbError, ErrorClass, KvEngine, OperationalAttemptOutcome, OperationalTimingInstrumented,
    MAX_KEY_BYTES,
};
''',
    "lsm fault attempt imports",
)
replace_once(
    LSM_FAULT,
    '''    expected.push((key7, value7));
    assert!(matches!(engine.get(b"k-00"), Err(DbError::Poisoned)));
    drop(engine);
''',
    '''    expected.push((key7, value7));
    let timing = engine.operational_timing_report();
    assert!(timing.compaction_stall_ns.is_empty(), "{kind:?} {mode:?}");
    assert!(
        timing.compaction_stall_samples.is_empty(),
        "{kind:?} {mode:?}"
    );
    assert_eq!(timing.compaction_stall_attempts.len(), 1, "{kind:?} {mode:?}");
    let attempt = &timing.compaction_stall_attempts[0];
    assert_eq!(attempt.measured_step_index, None, "{kind:?} {mode:?}");
    let work = attempt.work.expect("triggered compaction has known input work");
    assert!(work.units_examined > 0, "{kind:?} {mode:?}");
    assert!(work.bytes_examined > 0, "{kind:?} {mode:?}");
    assert!(matches!(
        &attempt.outcome,
        OperationalAttemptOutcome::Failed {
            error_class: ErrorClass::Io,
            ..
        }
    ));
    assert!(matches!(engine.get(b"k-00"), Err(DbError::Poisoned)));
    drop(engine);
''',
    "lsm fault attempt assertions",
)

# Docs: acknowledge #27 counterbalancing and keep only the real remaining blockers.
replace_once(
    README,
    '''Successful REOPEN/LSM-compaction timings additionally carry their exact
measured-step index and deterministic page/record plus data-path-byte work while retaining the original raw
nanosecond vectors for compatibility; these samples are still not controlled-host performance claims.
''',
    '''Successful REOPEN/LSM-compaction timings additionally carry their exact
measured-step index and deterministic page/record plus data-path-byte work while retaining the original raw
nanosecond vectors for compatibility. Complete operational-attempt streams now also retain failed REOPEN and
triggered-compaction durations with stable error class/message; failures remain excluded from the old
success-only vectors rather than silently disappearing. Fresh AB/BA counterbalanced pairs execute both whole-run
engine orders on independent engine instances, but these samples are still not controlled-host performance claims.
''',
    "README operational evidence",
)

replace_once(
    METHOD,
    '''The compatibility duration vectors and structured vectors are appended together and tests require their
indices/durations to agree. Successful-sample work accounting is therefore deterministic and trace-associated,
but the roadmap item remains incomplete: failed/excluded recovery or compaction attempts are not retained,
engine execution order is not counterbalanced, and the archive's declared cache/filesystem state is not an
enforced protocol. Scheduler noise, build profile, host identity, cache state, filesystem, and storage device
must still be controlled before timing distributions can support a performance claim.
''',
    '''The compatibility duration vectors and structured success vectors are appended together and tests require
their indices/durations to agree. `OperationalTimingReport` additionally retains append-only attempt streams.
Successful attempts mirror the compatibility sample; failures retain elapsed duration, measured step index,
stable error class/message, and deterministic work when it is already known without extra measurement I/O.
Injected LSM compaction faults are regression-tested to appear in `compaction_stall_attempts` while remaining
absent from the success-only duration/sample projections.

Execution-order counterbalancing is no longer caller folklore: `compare_experiment_trace_counterbalanced`
creates fresh left/right engines for one `left_then_right` and one `right_then_left` repetition, preserves raw
ordered reports, and fails closed if capabilities or logical outcomes differ across repetitions. The roadmap
item nevertheless remains incomplete because warmup/exclusion policy and the archive's declared cache/filesystem
state are not enforced measurement protocols. Scheduler noise, build profile, host identity, cache state,
filesystem, and storage device must still be controlled before timing distributions can support a performance
claim; failed attempts must be reported, not filtered into a success-only distribution.
''',
    "methodology attempt streams",
)

replace_once(
    ROADMAP,
    '''- [ ] Complete recovery-cost and compaction-stall distributions. Successful samples now pair duration with
  the exact measured trace-step index and deterministic data-path work: B+ tree reopen page accesses/bytes,
  LSM reopen WAL+SSTable record versions/bytes, and LSM full-set compaction input record versions/bytes. Tests
  pin trace association and the legacy raw-nanosecond projections. Failed/excluded attempts, counterbalanced
  engine order, and an enforced cache/filesystem protocol are still required before this item is complete.
''',
    '''- [ ] Complete recovery-cost and compaction-stall distributions. Successful samples pair duration with the
  exact measured trace-step index and deterministic data-path work. Append-only attempt streams now retain failed
  REOPEN/compaction durations and stable errors, with compaction input work when known. Fresh AB/BA counterbalanced
  pairs execute both whole-run engine orders and fail closed across repetition drift. The remaining blockers are
  an explicit warmup/exclusion policy plus an enforced cache/filesystem preparation protocol on a pinned host.
''',
    "roadmap operational status",
)

print("applied failed operational attempt evidence")
