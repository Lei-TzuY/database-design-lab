from pathlib import Path

# Compaction already has the correct observed allocation floor in next_table_id because open() takes the
# maximum of active descriptors, persisted v4 high watermark, and every canonical SSTable name. Carry
# next_table_id - 1 into a temporary VersionSet clone before prepare_install(); no new manifest API needed.
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
        let mut compaction_base = self.version.clone();
        compaction_base.table_id_high_watermark = compaction_base
            .table_id_high_watermark
            .max(observed_table_id_high_watermark);
        let compacted = manifest::prepare_install(
            &self.path,
            &compaction_base,
            manifest_id,
            durable_sequence,
            descriptors,
            self.version.wal_id,
            self.version.wal_first_sequence,
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
