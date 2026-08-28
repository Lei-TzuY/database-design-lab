from pathlib import Path

# Manifest: let compaction carry an observed on-disk id floor into v4 even when it emits no table.
p = Path("crates/db-storage-lsm/src/manifest.rs")
text = p.read_text()
old = '''pub(super) fn prepare_install(
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
    let table_id_high_watermark = tables
        .iter()
        .fold(current.table_id_high_watermark, |high, descriptor| {
            high.max(descriptor.table_id)
        });
    validate_version_set(
        MANIFEST_FORMAT_VERSION,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        table_id_high_watermark,
        &tables,
    )?;
    let next = VersionSet {
        current_generation: generation,
        manifest_id: new_manifest_id,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        table_id_high_watermark,
        tables,
    };
    write_manifest_new(directory, &next)?;
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
    prepare_install_with_table_id_floor(
        directory,
        current,
        new_manifest_id,
        durable_sequence,
        tables,
        wal_id,
        wal_first_sequence,
        current.table_id_high_watermark,
    )
}

pub(super) fn prepare_install_with_table_id_floor(
    directory: &Path,
    current: &VersionSet,
    new_manifest_id: u64,
    durable_sequence: u64,
    tables: Vec<SstableDescriptor>,
    wal_id: u64,
    wal_first_sequence: u64,
    table_id_high_watermark_floor: u64,
) -> Result<VersionSet> {
    let generation = current
        .current_generation
        .checked_add(1)
        .ok_or_else(|| corruption(0, "CURRENT generation exhausted"))?;
    let table_id_high_watermark = tables.iter().fold(
        current
            .table_id_high_watermark
            .max(table_id_high_watermark_floor),
        |high, descriptor| high.max(descriptor.table_id),
    );
    validate_version_set(
        MANIFEST_FORMAT_VERSION,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        table_id_high_watermark,
        &tables,
    )?;
    let next = VersionSet {
        current_generation: generation,
        manifest_id: new_manifest_id,
        durable_sequence,
        wal_id,
        wal_first_sequence,
        table_id_high_watermark,
        tables,
    };
    write_manifest_new(directory, &next)?;
    Ok(next)
}
'''
if old not in text:
    raise SystemExit("missing current prepare_install implementation")
p.write_text(text.replace(old, new, 1))

# Compaction: next_table_id already incorporates every canonical SSTable observed during open, including
# unreferenced crash orphans. Persist next_table_id - 1 before cleanup can remove those names.
p = Path("crates/db-storage-lsm/src/lib.rs")
text = p.read_text()
old = '''        let compacted = manifest::prepare_install(
            &self.path,
            &self.version,
            manifest_id,
            durable_sequence,
            descriptors,
            self.version.wal_id,
            self.version.wal_first_sequence,
        )?;
'''
new = '''        let observed_table_id_high_watermark = table_id
            .checked_sub(1)
            .ok_or_else(|| corruption("next SSTable id unexpectedly reached zero"))?;
        let compacted = manifest::prepare_install_with_table_id_floor(
            &self.path,
            &self.version,
            manifest_id,
            durable_sequence,
            descriptors,
            self.version.wal_id,
            self.version.wal_first_sequence,
            observed_table_id_high_watermark,
        )?;
'''
if old not in text:
    raise SystemExit("missing compaction prepare_install call")
p.write_text(text.replace(old, new, 1))

# Regression: an ambiguous orphan id discovered only from layout must survive empty-output cleanup as a
# persisted allocation floor, otherwise reopen could silently reuse that id.
p = Path("crates/db-storage-lsm/src/tombstone_elision_tests.rs")
text = p.read_text()
append = r'''

#[test]
fn durable_empty_retry_preserves_observed_orphan_id_floor_across_cleanup() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    build_three_tombstone_l0(&path);

    {
        let mut engine = LsmEngine::open(&path).expect("open three-L0 baseline");
        engine.inject_compaction_fault_for_test(
            CompactionWriteKind::Manifest,
            CompactionFaultMode::BeforeWrite,
        );
        for index in 48_u64..63 {
            assert_eq!(engine.delete(&tombstone_key(index)).expect("pre-trigger delete"), None);
        }
        assert!(matches!(
            engine.delete(&tombstone_key(63)),
            Err(DbError::Io(_))
        ));
    }

    assert_eq!(canonical_count(&path, "sst-", ".sst"), 4);
    let orphan = path.join("sst-0000000000000099.sst");
    fs::write(&orphan, b"ambiguous crash orphan").expect("write canonical orphan id 99");

    let mut reopened = LsmEngine::open(&path).expect("open four-L0 state plus orphan 99");
    reopened
        .put(b"tail", b"v")
        .expect("retry empty compaction while retaining a WAL tail");
    let checkpoint = reopened.stats().expect("retry checkpoint stats");
    assert_eq!(checkpoint.durable_sequence, 64);
    assert_eq!(checkpoint.sstables, 0);
    assert_eq!(checkpoint.mutable_entries, 1);
    assert_eq!(canonical_count(&path, "sst-", ".sst"), 0);
    assert!(!orphan.exists(), "cleanup may remove orphan 99 only after persisting its id floor");

    reopened.reopen().expect("reopen after orphan cleanup");
    reopened
        .put(b"fill-a", &large_value(0x71))
        .expect("put first post-checkpoint filler");
    reopened
        .put(b"fill-b", &large_value(0x72))
        .expect("flush post-checkpoint table");
    assert!(
        path.join("sst-0000000000000100.sst").exists(),
        "table allocation must continue above the removed ambiguous orphan id"
    );
    assert_eq!(
        reopened.get(b"tail").expect("read WAL-tail value"),
        Some(b"v".to_vec())
    );
    reopened.reopen().expect("reopen table 100");
    assert_eq!(
        reopened.get(b"tail").expect("read persisted tail"),
        Some(b"v".to_vec())
    );
}
'''
if "durable_empty_retry_preserves_observed_orphan_id_floor_across_cleanup" in text:
    raise SystemExit("orphan-floor regression already present")
p.write_text(text + append)
