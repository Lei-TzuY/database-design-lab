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

#[test]
fn reopen_rejects_same_count_substitution_of_acknowledged_record() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("substitution.log");
    let replacement_path = directory.path().join("replacement.log");

    let mut engine = LogEngine::create_new(&path).expect("create log");
    engine
        .put(b"stable", b"original")
        .expect("persist acknowledged record");

    let mut replacement = LogEngine::create_new(&replacement_path).expect("create replacement log");
    replacement
        .put(b"stable", b"replaced")
        .expect("persist replacement record");
    drop(replacement);
    let replacement_bytes = std::fs::read(&replacement_path).expect("read valid replacement log");

    std::fs::write(&path, &replacement_bytes)
        .expect("replace acknowledged bytes while preserving the backing file identity");

    let error = engine
        .reopen()
        .expect_err("reopen must reject substitution of an acknowledged record");
    assert!(
        matches!(error, DbError::Corruption { .. })
            && error.to_string().contains("acknowledged record changed"),
        "unexpected substitution error: {error}"
    );
    assert!(matches!(engine.get(b"stable"), Err(DbError::Poisoned)));
    assert_eq!(
        std::fs::read(&path).expect("read substituted file after rejected reopen"),
        replacement_bytes,
        "substitution rejection must not mutate the backing file"
    );
}
