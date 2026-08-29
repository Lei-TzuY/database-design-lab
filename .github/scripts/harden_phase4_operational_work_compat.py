from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENGINE = ROOT / "crates/db-core/src/engine.rs"
EXPERIMENT = ROOT / "crates/db-core/src/experiment.rs"
BTREE_COMMON = ROOT / "crates/db-storage-btree/src/tree/common.rs"
BTREE_TESTS = ROOT / "crates/db-storage-btree/src/tree/instrumentation_tests.rs"
LSM = ROOT / "crates/db-storage-lsm/src/lib.rs"
LSM_TESTS = ROOT / "crates/db-storage-lsm/src/instrumentation_tests.rs"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one marker, found {count}")
    return text.replace(old, new, 1)


text = ENGINE.read_text()
text = replace_once(
    text,
    '''pub struct OperationalTimingReport {
    /// Successful same-handle `REOPEN` samples.
    pub reopen_samples: Vec<OperationalTimingSample>,
    /// Successful synchronous compaction-path samples. Empty for engines without compaction.
    pub compaction_stall_samples: Vec<OperationalTimingSample>,
}
''',
    '''pub struct OperationalTimingReport {
    /// Backward-compatible projection of successful same-handle `REOPEN` durations in nanoseconds.
    pub reopen_ns: Vec<u64>,
    /// Backward-compatible projection of successful synchronous compaction durations in nanoseconds.
    pub compaction_stall_ns: Vec<u64>,
    /// Successful same-handle `REOPEN` samples with deterministic work and measured-step association.
    pub reopen_samples: Vec<OperationalTimingSample>,
    /// Successful synchronous compaction samples with deterministic work and measured-step association.
    pub compaction_stall_samples: Vec<OperationalTimingSample>,
}
''',
    "preserve legacy timing projections",
)
ENGINE.write_text(text)

text = BTREE_COMMON.read_text()
old = '''                let page_accesses = reopened.pager.read_page_calls();
                operational_timing
                    .reopen_samples
                    .push(OperationalTimingSample {
                        measured_step_index: operational_step_index,
                        duration_ns: u64::try_from(started.elapsed().as_nanos())
                            .unwrap_or(u64::MAX),
                        work: OperationalWork {
                            unit: OperationalWorkUnit::BtreePageAccess,
                            units_examined: page_accesses,
                            bytes_examined: page_accesses.saturating_mul(PAGE_SIZE as u64),
                        },
                    });
'''
new = '''                let page_accesses = reopened.pager.read_page_calls();
                let duration_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                operational_timing.reopen_ns.push(duration_ns);
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
'''
text = replace_once(text, old, new, "btree legacy timing projection")
BTREE_COMMON.write_text(text)

text = LSM.read_text()
old = '''        self.operational_timing
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
'''
new = '''        let duration_ns =
            u64::try_from(compaction_started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.operational_timing.compaction_stall_ns.push(duration_ns);
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
'''
text = replace_once(text, old, new, "lsm compaction legacy projection")
old = '''            Ok(mut reopened) => {
                let work = reopened.reopen_work()?;
                operational_timing
                    .reopen_samples
                    .push(OperationalTimingSample {
                        measured_step_index: operational_step_index,
                        duration_ns: u64::try_from(started.elapsed().as_nanos())
                            .unwrap_or(u64::MAX),
                        work,
                    });
'''
new = '''            Ok(mut reopened) => {
                let work = reopened.reopen_work()?;
                let duration_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                operational_timing.reopen_ns.push(duration_ns);
                operational_timing
                    .reopen_samples
                    .push(OperationalTimingSample {
                        measured_step_index: operational_step_index,
                        duration_ns,
                        work,
                    });
'''
text = replace_once(text, old, new, "lsm reopen legacy projection")
LSM.write_text(text)

text = EXPERIMENT.read_text()
old = '''        fn reopen(&mut self) -> Result<()> {
            self.operational_timing
                .reopen_samples
                .push(OperationalTimingSample {
                    measured_step_index: self.operational_step_index,
                    duration_ns: 1,
                    work: OperationalWork {
'''
new = '''        fn reopen(&mut self) -> Result<()> {
            self.operational_timing.reopen_ns.push(1);
            self.operational_timing
                .reopen_samples
                .push(OperationalTimingSample {
                    measured_step_index: self.operational_step_index,
                    duration_ns: 1,
                    work: OperationalWork {
'''
text = replace_once(text, old, new, "fake legacy timing projection")
EXPERIMENT.write_text(text)

text = BTREE_TESTS.read_text()
needle = '''    let sample = timing.reopen_samples[0];
    assert_eq!(sample.measured_step_index, Some(7));
'''
replacement = '''    let sample = timing.reopen_samples[0];
    assert_eq!(timing.reopen_ns, vec![sample.duration_ns]);
    assert_eq!(sample.measured_step_index, Some(7));
'''
text = replace_once(text, needle, replacement, "btree projection test")
BTREE_TESTS.write_text(text)

text = LSM_TESTS.read_text()
needle = '''    let compaction = timing.compaction_stall_samples[0];
    assert_eq!(compaction.measured_step_index, Some(7));
'''
replacement = '''    let compaction = timing.compaction_stall_samples[0];
    assert_eq!(timing.compaction_stall_ns, vec![compaction.duration_ns]);
    assert_eq!(compaction.measured_step_index, Some(7));
'''
text = replace_once(text, needle, replacement, "lsm compaction projection test")
needle = '''    let reopen = timing.reopen_samples[0];
    assert_eq!(reopen.measured_step_index, Some(99));
'''
replacement = '''    let reopen = timing.reopen_samples[0];
    assert_eq!(timing.reopen_ns, vec![reopen.duration_ns]);
    assert_eq!(reopen.measured_step_index, Some(99));
'''
text = replace_once(text, needle, replacement, "lsm reopen projection test")
LSM_TESTS.write_text(text)

print("preserved legacy operational timing vectors alongside structured samples")
