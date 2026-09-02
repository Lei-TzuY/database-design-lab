use db_core::{DbError, KvEngine};
use db_storage_log::LogEngine;
use tempfile::tempdir;

#[test]
fn reopen_rejects_same_file_rollback_below_acknowledged_record_count() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("rollback.log");

    let mut engine = LogEngine::create_new(&path).expect("create log");
    let empty_log = std::fs::read(&path).expect("capture empty valid log");

    engine
        .put(b"stable", b"acknowledged")
        .expect("persist acknowledged record");

    std::fs::write(&path, &empty_log).expect("truncate same backing file to an older valid state");

    let error = engine
        .reopen()
        .expect_err("reopen must reject rollback below acknowledged records");
    assert!(
        matches!(error, DbError::Corruption { .. })
            && error.to_string().contains("record count regressed"),
        "unexpected rollback error: {error}"
    );
    assert!(matches!(engine.get(b"stable"), Err(DbError::Poisoned)));
    assert_eq!(
        std::fs::read(&path).expect("read rolled-back file after rejected reopen"),
        empty_log,
        "rollback rejection must not mutate the backing file"
    );
}
