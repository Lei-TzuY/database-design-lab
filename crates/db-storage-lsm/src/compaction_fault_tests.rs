use std::fs;
use std::path::{Path, PathBuf};

use db_core::{DbError, KvEngine};
use tempfile::{tempdir, TempDir};

use super::{CompactionFaultMode, CompactionWriteKind, LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT};

fn large_value(byte: u8) -> Vec<u8> {
    vec![byte; MUTABLE_MEMTABLE_BYTES_LIMIT / 2 + 1_024]
}

fn clone_engine(source: &Path, directory: &TempDir, name: &str) -> PathBuf {
    let target = directory.path().join(name);
    fs::create_dir(&target).expect("create cloned engine directory");
    for entry in fs::read_dir(source).expect("read baseline directory") {
        let entry = entry.expect("baseline directory entry");
        assert!(entry.file_type().expect("entry type").is_file());
        fs::copy(entry.path(), target.join(entry.file_name())).expect("copy baseline file");
    }
    target
}

fn put_pair(engine: &mut LsmEngine, first: u8, expected: &mut Vec<(Vec<u8>, Vec<u8>)>) {
    for offset in 0_u8..2 {
        let index = first + offset;
        let key = format!("k-{index:02}").into_bytes();
        let value = large_value(0x40 + index);
        engine.put(&key, &value).expect("populate flush pair");
        expected.push((key, value));
    }
}

fn build_three_l0_baseline(path: &Path) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut engine = LsmEngine::create_new(path).expect("create baseline LSM");
    let mut expected = Vec::new();
    put_pair(&mut engine, 0, &mut expected);
    put_pair(&mut engine, 2, &mut expected);
    put_pair(&mut engine, 4, &mut expected);
    let stats = engine.stats().expect("three-L0 stats");
    assert_eq!(stats.level0_sstables, 3);
    assert_eq!(stats.level1_sstables, 0);
    assert_eq!(stats.durable_sequence, 6);
    drop(engine);
    expected
}

fn expected_compacted(kind: CompactionWriteKind, mode: CompactionFaultMode) -> bool {
    kind == CompactionWriteKind::MirrorCurrent
        || (kind == CompactionWriteKind::FirstCurrent && mode == CompactionFaultMode::AfterSync)
}

fn assert_fault_case(
    baseline: &Path,
    directory: &TempDir,
    case: usize,
    kind: CompactionWriteKind,
    mode: CompactionFaultMode,
    baseline_expected: &[(Vec<u8>, Vec<u8>)],
) {
    let path = clone_engine(baseline, directory, &format!("fault-{case}"));
    let mut engine = LsmEngine::open(&path).expect("open cloned baseline");
    engine.inject_compaction_fault_for_test(kind, mode);

    let mut expected = baseline_expected.to_vec();
    let key6 = b"k-06".to_vec();
    let value6 = large_value(0x46);
    engine
        .put(&key6, &value6)
        .expect("first fourth-L0 mutation");
    expected.push((key6, value6));
    let key7 = b"k-07".to_vec();
    let value7 = large_value(0x47);
    let error = engine
        .put(&key7, &value7)
        .expect_err("injected compaction fault must escape the triggering mutation");
    assert!(
        matches!(error, DbError::Io(_)),
        "{kind:?} {mode:?}: {error}"
    );
    expected.push((key7, value7));
    assert!(matches!(engine.get(b"k-00"), Err(DbError::Poisoned)));
    drop(engine);

    let mut reopened = LsmEngine::open(&path).expect("reopen injected compaction state");
    let stats = reopened.stats().expect("stats after injected reopen");
    assert_eq!(stats.durable_sequence, 8, "{kind:?} {mode:?}");
    assert_eq!(stats.sstable_entries, 8, "{kind:?} {mode:?}");
    if expected_compacted(kind, mode) {
        assert_eq!(stats.level0_sstables, 0, "{kind:?} {mode:?}");
        assert_eq!(stats.level1_sstables, 1, "{kind:?} {mode:?}");
        assert_eq!(stats.sstables, 1, "{kind:?} {mode:?}");
    } else {
        assert_eq!(stats.level0_sstables, 4, "{kind:?} {mode:?}");
        assert_eq!(stats.level1_sstables, 0, "{kind:?} {mode:?}");
        assert_eq!(stats.sstables, 4, "{kind:?} {mode:?}");
    }
    for (key, value) in expected {
        assert_eq!(
            reopened.get(&key).expect("read after injected reopen"),
            Some(value),
            "{kind:?} {mode:?}: key {:?}",
            String::from_utf8_lossy(&key)
        );
    }
    let verified = LsmEngine::verify(&path).expect("verify injected compaction state");
    assert_eq!(verified.memtables, stats, "{kind:?} {mode:?}");
}

#[test]
fn compaction_durable_write_trace_is_stable() {
    let directory = tempdir().expect("temporary directory");
    let baseline = directory.path().join("trace-baseline");
    let _ = build_three_l0_baseline(&baseline);
    let path = clone_engine(&baseline, &directory, "trace-run");
    let mut engine = LsmEngine::open(&path).expect("open trace fixture");
    engine.begin_compaction_fault_trace_for_test();
    let mut ignored = Vec::new();
    put_pair(&mut engine, 6, &mut ignored);
    assert_eq!(
        engine.compaction_fault_trace_for_test(),
        &[
            CompactionWriteKind::L1Sstable,
            CompactionWriteKind::Manifest,
            CompactionWriteKind::FirstCurrent,
            CompactionWriteKind::MirrorCurrent,
        ]
    );
}

#[test]
fn compaction_fault_matrix_reopens_only_complete_old_or_new_version() {
    let directory = tempdir().expect("temporary directory");
    let baseline = directory.path().join("fault-baseline");
    let expected = build_three_l0_baseline(&baseline);
    let kinds = [
        CompactionWriteKind::L1Sstable,
        CompactionWriteKind::Manifest,
        CompactionWriteKind::FirstCurrent,
        CompactionWriteKind::MirrorCurrent,
    ];
    let modes = [
        CompactionFaultMode::BeforeWrite,
        CompactionFaultMode::TornWrite,
        CompactionFaultMode::AfterSync,
    ];

    let mut case = 0_usize;
    for kind in kinds {
        for mode in modes {
            assert_fault_case(&baseline, &directory, case, kind, mode, &expected);
            case += 1;
        }
    }
}
