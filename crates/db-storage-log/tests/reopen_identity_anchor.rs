use db_core::{DbError, KvEngine};
use db_storage_log::LogEngine;
use tempfile::tempdir;

#[test]
fn failed_reopen_preserves_identity_anchor_against_later_replacement() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("ordinary.log");
    let corrupted_original = directory.path().join("ordinary.corrupted.log");
    let replacement = directory.path().join("replacement.log");

    let mut engine = LogEngine::create_new(&path).expect("create original log");
    engine
        .put(b"stable", b"original")
        .expect("write original value");

    let mut corrupted = std::fs::read(&path).expect("read original bytes");
    corrupted[0] ^= 0xff;
    std::fs::write(&path, &corrupted).expect("publish in-place corruption");

    let first_error = engine
        .reopen()
        .expect_err("corrupt original must fail reopen");
    assert!(matches!(first_error, DbError::Corruption { .. }));
    assert!(matches!(engine.get(b"stable"), Err(DbError::Poisoned)));

    {
        let mut other = LogEngine::create_new(&replacement).expect("create replacement log");
        other
            .put(b"stable", b"replacement")
            .expect("write replacement value");
    }
    let replacement_bytes = std::fs::read(&replacement).expect("read replacement bytes");

    std::fs::rename(&path, &corrupted_original).expect("move corrupted original aside");
    std::fs::rename(&replacement, &path).expect("publish valid replacement at original pathname");

    let second_error = engine
        .reopen()
        .expect_err("failed reopen must retain original identity anchor");
    assert!(
        second_error
            .to_string()
            .contains("backing file identity changed"),
        "unexpected second reopen error: {second_error}"
    );
    assert!(matches!(engine.get(b"stable"), Err(DbError::Poisoned)));
    assert_eq!(
        std::fs::read(&path).expect("read replacement after rejected reopen"),
        replacement_bytes,
        "identity rejection must not mutate the replacement file"
    );
}
