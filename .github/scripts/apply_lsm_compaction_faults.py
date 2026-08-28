from pathlib import Path


def replace_once(path: str, old: str, new: str, label: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        raise SystemExit(f"missing {label} in {path}: {old[:160]!r}")
    p.write_text(text.replace(old, new, 1))

# Split manifest creation from the first CURRENT publication. Existing callers keep install(), while
# compaction can now expose each durable boundary independently to the deterministic fault harness.
p = Path("crates/db-storage-lsm/src/manifest.rs")
text = p.read_text()
old = '''pub(super) fn install(
    directory: &Path,
    current: &VersionSet,
    new_manifest_id: u64,
    durable_sequence: u64,
    tables: Vec<SstableDescriptor>,
    wal_id: u64,
    wal_first_sequence: u64,
) -> Result<VersionSet> {
    let generation = current
        .current_generation
        .checked_add(1)
        .ok_or_else(|| corruption(0, "CURRENT generation exhausted"))?;
    validate_version_set(durable_sequence, wal_id, wal_first_sequence, &tables)?;
    let next = VersionSet {
        current_generation: generation,
        manifest_id: new_manifest_id,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        tables,
    };
    write_manifest_new(directory, &next)?;
    write_current_slot(directory, generation, new_manifest_id)?;
    Ok(next)
}
'''
new = '''pub(super) fn prepare_install(
    directory: &Path,
    current: &VersionSet,
    new_manifest_id: u64,
    durable_sequence: u64,
    tables: Vec<SstableDescriptor>,
    wal_id: u64,
    wal_first_sequence: u64,
) -> Result<VersionSet> {
    let generation = current
        .current_generation
        .checked_add(1)
        .ok_or_else(|| corruption(0, "CURRENT generation exhausted"))?;
    validate_version_set(durable_sequence, wal_id, wal_first_sequence, &tables)?;
    let next = VersionSet {
        current_generation: generation,
        manifest_id: new_manifest_id,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        tables,
    };
    write_manifest_new(directory, &next)?;
    Ok(next)
}

pub(super) fn publish_prepared(directory: &Path, prepared: &VersionSet) -> Result<()> {
    write_current_slot(
        directory,
        prepared.current_generation,
        prepared.manifest_id,
    )
}

pub(super) fn install(
    directory: &Path,
    current: &VersionSet,
    new_manifest_id: u64,
    durable_sequence: u64,
    tables: Vec<SstableDescriptor>,
    wal_id: u64,
    wal_first_sequence: u64,
) -> Result<VersionSet> {
    let next = prepare_install(
        directory,
        current,
        new_manifest_id,
        durable_sequence,
        tables,
        wal_id,
        wal_first_sequence,
    )?;
    publish_prepared(directory, &next)?;
    Ok(next)
}
'''
if old not in text:
    raise SystemExit("missing manifest install block")
p.write_text(text.replace(old, new, 1))

# Engine-local test seam. The production write path remains unchanged when cfg(test) is absent.
p = Path("crates/db-storage-lsm/src/lib.rs")
text = p.read_text()
text = text.replace(
    "const LEVEL0_COMPACTION_TRIGGER: usize = 4;\n",
    '''const LEVEL0_COMPACTION_TRIGGER: usize = 4;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactionWriteKind {
    L1Sstable,
    Manifest,
    FirstCurrent,
    MirrorCurrent,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactionFaultMode {
    BeforeWrite,
    TornWrite,
    AfterSync,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactionFaultSpec {
    kind: CompactionWriteKind,
    mode: CompactionFaultMode,
}
''',
    1,
)
text = text.replace(
    "    poisoned: bool,\n}\n",
    '''    poisoned: bool,
    #[cfg(test)]
    compaction_fault_spec: Option<CompactionFaultSpec>,
    #[cfg(test)]
    compaction_fault_trace: Vec<CompactionWriteKind>,
}
''',
    1,
)
text = text.replace(
    "            poisoned: false,\n        })\n    }\n\n    fn open_existing",
    '''            poisoned: false,
            #[cfg(test)]
            compaction_fault_spec: None,
            #[cfg(test)]
            compaction_fault_trace: Vec::new(),
        })
    }

    fn open_existing''',
    1,
)
text = text.replace(
    "            version,\n            poisoned: false,\n        })\n    }\n\n    /// Validates the authoritative CURRENT/manifest/SSTable set",
    '''            version,
            poisoned: false,
            #[cfg(test)]
            compaction_fault_spec: None,
            #[cfg(test)]
            compaction_fault_trace: Vec::new(),
        })
    }

    /// Validates the authoritative CURRENT/manifest/SSTable set''',
    1,
)
old_compact = '''        let table =
            SsTable::create_new_at_level(&self.path, table_id, 1, durable_sequence, &merged)?;
        let compacted = manifest::install(
            &self.path,
            &self.version,
            manifest_id,
            durable_sequence,
            vec![table.descriptor().clone()],
            self.version.wal_id,
            self.version.wal_first_sequence,
        )?;
        let mirrored = manifest::mirror_current(&self.path, &compacted)?;
        let active_table_id = table.descriptor().table_id;
        let active_manifest_id = mirrored.manifest_id;
'''
new_compact = '''        #[cfg(test)]
        self.compaction_before_write_for_test(CompactionWriteKind::L1Sstable)?;
        let table =
            SsTable::create_new_at_level(&self.path, table_id, 1, durable_sequence, &merged)?;
        #[cfg(test)]
        self.compaction_after_file_write_for_test(
            CompactionWriteKind::L1Sstable,
            &self.path.join(sstable_file_name(table_id)),
        )?;

        #[cfg(test)]
        self.compaction_before_write_for_test(CompactionWriteKind::Manifest)?;
        let compacted = manifest::prepare_install(
            &self.path,
            &self.version,
            manifest_id,
            durable_sequence,
            vec![table.descriptor().clone()],
            self.version.wal_id,
            self.version.wal_first_sequence,
        )?;
        #[cfg(test)]
        self.compaction_after_file_write_for_test(
            CompactionWriteKind::Manifest,
            &self.path.join(manifest::manifest_file_name(manifest_id)),
        )?;

        #[cfg(test)]
        self.compaction_before_write_for_test(CompactionWriteKind::FirstCurrent)?;
        manifest::publish_prepared(&self.path, &compacted)?;
        #[cfg(test)]
        self.compaction_after_current_write_for_test(
            CompactionWriteKind::FirstCurrent,
            compacted.current_generation,
        )?;

        #[cfg(test)]
        self.compaction_before_write_for_test(CompactionWriteKind::MirrorCurrent)?;
        let mirrored = manifest::mirror_current(&self.path, &compacted)?;
        #[cfg(test)]
        self.compaction_after_current_write_for_test(
            CompactionWriteKind::MirrorCurrent,
            mirrored.current_generation,
        )?;
        let active_table_id = table.descriptor().table_id;
        let active_manifest_id = mirrored.manifest_id;
'''
if old_compact not in text:
    raise SystemExit("missing compaction publication block")
text = text.replace(old_compact, new_compact, 1)
marker = '''    fn reclaim_obsolete_sstables(&self, active_table_id: u64) {
'''
helpers = r'''    #[cfg(test)]
    fn begin_compaction_fault_trace_for_test(&mut self) {
        self.compaction_fault_spec = None;
        self.compaction_fault_trace.clear();
    }

    #[cfg(test)]
    fn inject_compaction_fault_for_test(
        &mut self,
        kind: CompactionWriteKind,
        mode: CompactionFaultMode,
    ) {
        self.compaction_fault_spec = Some(CompactionFaultSpec { kind, mode });
        self.compaction_fault_trace.clear();
    }

    #[cfg(test)]
    fn compaction_fault_trace_for_test(&self) -> &[CompactionWriteKind] {
        &self.compaction_fault_trace
    }

    #[cfg(test)]
    fn compaction_before_write_for_test(&mut self, kind: CompactionWriteKind) -> Result<()> {
        self.compaction_fault_trace.push(kind);
        if self.compaction_fault_spec
            == Some(CompactionFaultSpec {
                kind,
                mode: CompactionFaultMode::BeforeWrite,
            })
        {
            return Err(injected_compaction_fault(kind, CompactionFaultMode::BeforeWrite));
        }
        Ok(())
    }

    #[cfg(test)]
    fn compaction_after_file_write_for_test(
        &self,
        kind: CompactionWriteKind,
        path: &Path,
    ) -> Result<()> {
        let Some(spec) = self.compaction_fault_spec.filter(|spec| spec.kind == kind) else {
            return Ok(());
        };
        match spec.mode {
            CompactionFaultMode::BeforeWrite => Ok(()),
            CompactionFaultMode::TornWrite => {
                let bytes = fs::metadata(path)?.len();
                let torn_len = (bytes / 2).max(1);
                let file = fs::OpenOptions::new().write(true).open(path)?;
                file.set_len(torn_len)?;
                file.sync_all()?;
                Err(injected_compaction_fault(kind, spec.mode))
            }
            CompactionFaultMode::AfterSync => Err(injected_compaction_fault(kind, spec.mode)),
        }
    }

    #[cfg(test)]
    fn compaction_after_current_write_for_test(
        &self,
        kind: CompactionWriteKind,
        generation: u64,
    ) -> Result<()> {
        use std::io::{Seek, SeekFrom, Write};

        let Some(spec) = self.compaction_fault_spec.filter(|spec| spec.kind == kind) else {
            return Ok(());
        };
        match spec.mode {
            CompactionFaultMode::BeforeWrite => Ok(()),
            CompactionFaultMode::TornWrite => {
                let slot_id = usize::try_from(generation % 2).expect("modulo two fits usize");
                let offset = u64::try_from(slot_id * manifest::CURRENT_SLOT_BYTES)
                    .expect("CURRENT slot offset fits u64");
                let mut file = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(self.path.join(CURRENT_FILE_NAME))?;
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(&vec![0xa5; manifest::CURRENT_SLOT_BYTES / 2])?;
                file.sync_data()?;
                Err(injected_compaction_fault(kind, spec.mode))
            }
            CompactionFaultMode::AfterSync => Err(injected_compaction_fault(kind, spec.mode)),
        }
    }

'''
if marker not in text:
    raise SystemExit("missing reclaim marker")
text = text.replace(marker, helpers + marker, 1)
text = text.replace(
    "#[cfg(test)]\nmod compaction_tests;\n",
    "#[cfg(test)]\nmod compaction_fault_tests;\n#[cfg(test)]\nmod compaction_tests;\n",
    1,
)
# Add a small error constructor beside the existing corruption helper.
old_corruption = '''fn corruption(reason: impl Into<String>) -> DbError {
    DbError::Corruption {
        offset: 0,
        reason: reason.into(),
    }
}
'''
new_corruption = '''fn corruption(reason: impl Into<String>) -> DbError {
    DbError::Corruption {
        offset: 0,
        reason: reason.into(),
    }
}

#[cfg(test)]
fn injected_compaction_fault(kind: CompactionWriteKind, mode: CompactionFaultMode) -> DbError {
    io::Error::other(format!(
        "injected LSM compaction durable-write fault at {kind:?} with mode {mode:?}"
    ))
    .into()
}
'''
if old_corruption not in text:
    raise SystemExit("missing corruption helper")
text = text.replace(old_corruption, new_corruption, 1)
p.write_text(text)

Path("crates/db-storage-lsm/src/compaction_fault_tests.rs").write_text(r'''use std::fs;
use std::path::{Path, PathBuf};

use db_core::{DbError, KvEngine};
use tempfile::{tempdir, TempDir};

use super::{
    CompactionFaultMode, CompactionWriteKind, LsmEngine, MUTABLE_MEMTABLE_BYTES_LIMIT,
};

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
    engine.put(&key6, &value6).expect("first fourth-L0 mutation");
    expected.push((key6, value6));
    let key7 = b"k-07".to_vec();
    let value7 = large_value(0x47);
    let error = engine
        .put(&key7, &value7)
        .expect_err("injected compaction fault must escape the triggering mutation");
    assert!(matches!(error, DbError::Io(_)), "{kind:?} {mode:?}: {error}");
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
''')
