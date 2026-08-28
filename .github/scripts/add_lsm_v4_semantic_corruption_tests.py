from pathlib import Path

p = Path("crates/db-storage-lsm/src/tombstone_elision_tests.rs")
text = p.read_text()
helper = r'''

fn rewrite_v4_table_id_high_watermark(path: &Path, high_watermark: u64) {
    let manifest_id = current_manifest_id(path);
    let manifest_path = path.join(format!("MANIFEST-{manifest_id:016}"));
    let mut bytes = fs::read(&manifest_path).expect("read v4 manifest for corruption fixture");
    assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 4);
    bytes[64..72].copy_from_slice(&high_watermark.to_le_bytes());
    let header_crc = crc32fast::hash(&bytes[..76]);
    bytes[76..80].copy_from_slice(&header_crc.to_le_bytes());
    let file_crc_offset = bytes.len() - 4;
    let file_crc = crc32fast::hash(&bytes[..file_crc_offset]);
    bytes[file_crc_offset..].copy_from_slice(&file_crc.to_le_bytes());
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(manifest_path)
        .expect("open v4 corruption fixture");
    file.write_all(&bytes).expect("write v4 corruption fixture");
    file.sync_all().expect("sync v4 corruption fixture");
}
'''
if "fn rewrite_v4_table_id_high_watermark" not in text:
    insert = text.index("\nfn delete_tombstone_range")
    text = text[:insert] + helper + text[insert:]

tests = r'''

#[test]
fn v4_rejects_table_id_high_watermark_below_active_descriptor() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create engine");
        engine.put(b"a", &large_value(0x31)).expect("put a");
        engine.put(b"b", &large_value(0x32)).expect("flush one L0");
        assert_eq!(engine.stats().expect("one-L0 stats").sstables, 1);
    }
    rewrite_v4_table_id_high_watermark(&path, 0);
    let error = LsmEngine::open(&path).expect_err("high watermark below active table must fail closed");
    assert!(
        error
            .to_string()
            .contains("table-id high watermark is below an active descriptor id"),
        "unexpected error: {error}"
    );
}

#[test]
fn v4_rejects_durable_empty_checkpoint_without_id_history() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("engine");
    {
        let mut engine = LsmEngine::create_new(&path).expect("create engine");
        delete_tombstone_range(&mut engine, 0, 64);
        let stats = engine.stats().expect("empty checkpoint stats");
        assert_eq!(stats.sstables, 0);
        assert_eq!(stats.durable_sequence, 64);
    }
    rewrite_v4_table_id_high_watermark(&path, 0);
    let error = LsmEngine::open(&path).expect_err("durable-empty checkpoint without id floor must fail");
    assert!(
        error
            .to_string()
            .contains("durable-empty checkpoint requires a nonzero table-id high watermark"),
        "unexpected error: {error}"
    );
}
'''
if "v4_rejects_table_id_high_watermark_below_active_descriptor" in text:
    raise SystemExit("semantic corruption tests already present")
p.write_text(text + tests)
