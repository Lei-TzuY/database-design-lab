from pathlib import Path

# Explicit trigger after the finalization validator was added to this staging branch.
ROOT = Path(__file__).resolve().parents[2]
BTREE_TEST = ROOT / "crates/db-storage-btree/src/tree/instrumentation_tests.rs"
LSM_FAULT = ROOT / "crates/db-storage-lsm/src/compaction_fault_tests.rs"
CLI = ROOT / "crates/db-cli/src/main.rs"
README = ROOT / "README.md"
METHOD = ROOT / "docs/amplification-methodology.md"
ROADMAP = ROOT / "docs/roadmap.md"


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected 1 marker, found {count}")
    path.write_text(text.replace(old, new, 1))


replace_once(
    BTREE_TEST,
    '''    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalTimingInstrumented,
    OperationalWorkUnit, Persistence, ReadWorkUnit, StorageArchitecture,
''',
    '''    DistributionMode, EngineCapabilities, KvEngine, LogicalModel, OperationalAttemptOutcome,
    OperationalTimingInstrumented, OperationalWorkUnit, Persistence, ReadWorkUnit, StorageArchitecture,
''',
    "btree attempt test import",
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
    "btree attempt projection assertion",
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
    "lsm failed compaction attempt assertion",
)

replace_once(
    CLI,
    '''    fn experiment_archive_shape_parses_and_revision_validation_is_strict() {
''',
    '''    fn experiment_batch_archive_shape_parses() {
        let batch = Cli::try_parse_from([
            "db-lab",
            "experiment-batch-archive",
            "--trace",
            "trace.json",
            "--workspace-dir",
            "work/run-001",
            "--archive-dir",
            "evidence/run-001",
            "--included-attempts",
            "6",
            "--warmup-attempts",
            "2",
            "--order-seed",
            "99",
            "--revision",
            "1073efafb752b6ae318ed01c667253a7406ae2fa",
            "--cache-state",
            "warm",
        ])
        .expect("parse experiment batch archive");
        assert!(matches!(
            batch.command,
            Command::ExperimentBatchArchive {
                included_attempts: 6,
                warmup_attempts: 2,
                order_seed: 99,
                cache_state: CacheStateKind::Warm,
                ..
            }
        ));
    }

    #[test]
    fn experiment_archive_shape_parses_and_revision_validation_is_strict() {
''',
    "cli batch parse test",
)

replace_once(
    README,
    '''| `db-cli` | Correctness `generate`/`run`/`differential`, Phase 4 `experiment-generate`/`experiment-compare`, plus append-log `verify`/`inspect` |
''',
    '''| `db-cli` | Correctness `generate`/`run`/`differential`, Phase 4 trace/compare/single-archive/counterbalanced-batch archive commands, plus append-log `verify`/`inspect` |
''',
    "README cli row",
)
replace_once(
    README,
    '''engines' amplification evidence. Successful REOPEN/LSM-compaction timings additionally carry their exact
measured-step index and deterministic page/record plus data-path-byte work while retaining the original raw
nanosecond vectors for compatibility; these samples are still not controlled-host performance claims.
`experiment-archive` adds a create-new raw evidence directory plus an explicit environment manifest.
''',
    '''engines' amplification evidence. Successful REOPEN/LSM-compaction timings additionally carry their exact
measured-step index and deterministic page/record plus data-path-byte work while retaining the original raw
nanosecond vectors for compatibility. `ExperimentBatch` v1 adds repeated fresh-engine attempts with a
seeded, exactly balanced AB/BA order among included attempts; warmups execute and remain archived but are
explicitly excluded. Factory/comparison failures remain in the attempt ledger, and timed REOPEN/compaction
failure attempts retain duration, error class/message, and deterministic work when it is already known.
These samples are still not controlled-host performance claims. `experiment-archive` adds a create-new raw
evidence directory plus an explicit environment manifest; `experiment-batch-archive` additionally preserves
fresh per-attempt engine state in a separate workspace and writes `batch.json` beside trace/environment data.
''',
    "README batch narrative",
)
replace_once(
    README,
    '''cargo run -p db-cli -- experiment-compare --trace mixed-42.json \\
  --btree-path btree-42.db --lsm-path lsm-42 --output mixed-42-report.json
''',
    '''cargo run -p db-cli -- experiment-compare --trace mixed-42.json \\
  --btree-path btree-42.db --lsm-path lsm-42 --output mixed-42-report.json
cargo run -p db-cli -- experiment-batch-archive --trace mixed-42.json \\
  --workspace-dir work/mixed-42 --archive-dir evidence/mixed-42 --included-attempts 10 \\
  --warmup-attempts 2 --order-seed 4242 --revision REVISION
''',
    "README batch example",
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
their indices/durations to agree. `OperationalTimingReport` also retains complete attempt streams: successful
REOPEN/compaction attempts mirror the compatibility sample, while failures keep elapsed duration, measured
step index, stable error class/message, and deterministic work when it is known without extra measurement I/O.
Injected LSM compaction failures are regression-tested to appear only in the attempt stream, not in the
success-only duration projection.

`ExperimentBatch` v1 addresses execution-order and exclusion bias at the runner layer. Included attempts must
be a positive even count; every pair executes the same trace once B+ tree-first and once LSM-first, while a
stable `order_seed` chooses each pair's orientation. Warmups are executed, indexed, and archived as
`excluded_warmup` rather than discarded. Every attempt uses fresh engine targets and records success or the
factory/comparison failure that ended it; comparison failures preserve each already-created engine's partial
operational attempt stream. The first successful attempt supplies one canonical logical-outcome vector and
later successes must match it. `experiment-batch-archive` stores `trace.json`, `batch.json`, and the environment
manifest while retaining per-attempt engine state in a distinct workspace for forensic inspection.

The roadmap item still remains incomplete because the archive's declared cache/filesystem state is metadata,
not an enforced preparation protocol. Scheduler noise, build profile, host identity, cache state, filesystem,
and storage device must still be controlled on a pinned host before timing distributions can support a
performance claim. A batch with any included failure must be reported as such; it is not silently filtered into
a success-only distribution.
''',
    "methodology batch section",
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
  exact measured trace-step index and deterministic data-path work. Full operational attempt streams now retain
  failed REOPEN/compaction durations and stable errors, with compaction input work when known. `ExperimentBatch`
  v1 runs fresh engines repeatedly, archives warmups as excluded attempts, and guarantees equal B+ tree-first /
  LSM-first counts through deterministic AB/BA pairs; factory/comparison failures remain in the ledger instead
  of disappearing. The remaining blocker is an enforced cache/filesystem preparation protocol on a pinned host.
''',
    "roadmap attempt status",
)

print("finalized Phase 4 counterbalanced attempt evidence and methodology")
