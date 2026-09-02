use db_core::{DbError, KvEngine};
use db_storage_log::{is_canonical_generation_path, LogEngine};
use tempfile::tempdir;

#[test]
fn standalone_constructors_reject_canonical_generation_paths_before_mutation() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("generation-00000000000000000001.log");

    assert!(is_canonical_generation_path(&path));
    let create_error = LogEngine::create_new(&path).expect_err("standalone create must fail");
    assert!(create_error
        .to_string()
        .contains("standalone append-log constructor refuses canonical generation path"));
    assert!(
        !path.exists(),
        "rejected standalone create must not create a file"
    );

    {
        let mut managed =
            LogEngine::create_new_managed_generation(&path).expect("managed generation create");
        managed.put(b"key", b"value").expect("managed put");
    }
    let before = std::fs::read(&path).expect("read managed generation");

    let open_error = LogEngine::open(&path).expect_err("standalone open must fail");
    assert!(open_error
        .to_string()
        .contains("standalone append-log constructor refuses canonical generation path"));
    assert_eq!(
        std::fs::read(&path).expect("read after rejected standalone open"),
        before,
        "rejected standalone open must not repair or mutate the generation file"
    );
}

#[test]
fn managed_generation_constructor_preserves_intent_across_reopen() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("generation-00000000000000000042.log");

    let mut engine =
        LogEngine::create_new_managed_generation(&path).expect("create managed generation");
    engine.put(b"a", b"one").expect("initial put");
    engine.reopen().expect("managed reopen");
    assert_eq!(
        engine.get(b"a").expect("get after reopen"),
        Some(b"one".to_vec())
    );
    engine.put(b"b", b"two").expect("post-reopen put");
    drop(engine);

    let mut reopened = LogEngine::open_managed_generation(&path).expect("managed open");
    assert_eq!(reopened.get(b"a").expect("get a"), Some(b"one".to_vec()));
    assert_eq!(reopened.get(b"b").expect("get b"), Some(b"two".to_vec()));

    let verification = LogEngine::verify(&path).expect("read-only verify remains allowed");
    assert_eq!(verification.record_count, 2);
    let inspection = LogEngine::inspect(&path, true).expect("read-only inspect remains allowed");
    assert_eq!(inspection.entries.len(), 2);
}

#[test]
fn failed_managed_reopen_poisoning_is_fail_closed_until_explicit_recovery() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("generation-00000000000000000077.log");

    let mut engine =
        LogEngine::create_new_managed_generation(&path).expect("create managed generation");
    engine.put(b"stable", b"value").expect("initial put");
    let clean = std::fs::read(&path).expect("read clean generation");

    let mut corrupted = clean.clone();
    corrupted[0] ^= 0xff;
    std::fs::write(&path, &corrupted).expect("publish corruption fixture");

    let reopen_error = engine
        .reopen()
        .expect_err("corrupt managed generation must fail reopen");
    assert!(matches!(reopen_error, DbError::Corruption { .. }));
    assert!(matches!(engine.get(b"stable"), Err(DbError::Poisoned)));
    assert!(matches!(
        engine.put(b"blocked", b"write"),
        Err(DbError::Poisoned)
    ));
    assert!(matches!(engine.delete(b"stable"), Err(DbError::Poisoned)));
    assert_eq!(
        std::fs::read(&path).expect("read after poisoned operations"),
        corrupted,
        "poisoned operations must not mutate the failed-reopen image"
    );

    std::fs::write(&path, &clean).expect("restore clean generation");
    assert!(matches!(engine.get(b"stable"), Err(DbError::Poisoned)));

    engine.reopen().expect("explicit reopen must recover after repair");
    assert_eq!(
        engine.get(b"stable").expect("get after explicit recovery"),
        Some(b"value".to_vec())
    );
    engine
        .put(b"after", b"recovery")
        .expect("write after explicit recovery");
    let verification = LogEngine::verify(&path).expect("verify recovered generation");
    assert_eq!(verification.record_count, 2);
}

#[test]
fn managed_generation_constructor_is_not_a_general_raw_bypass() {
    let directory = tempdir().expect("temporary directory");
    let ordinary = directory.path().join("ordinary.log");
    let zero = directory.path().join("generation-00000000000000000000.log");
    let malformed = directory.path().join("generation-1.log");

    for path in [&ordinary, &zero, &malformed] {
        assert!(!is_canonical_generation_path(path));
        let error = LogEngine::create_new_managed_generation(path)
            .expect_err("managed constructor must require canonical generation name");
        assert!(error
            .to_string()
            .contains("managed-generation append-log constructor requires canonical"));
        assert!(!path.exists());
    }

    let mut ordinary_engine = LogEngine::create_new(&ordinary).expect("ordinary standalone create");
    ordinary_engine.put(b"key", b"value").expect("ordinary put");
    ordinary_engine
        .reopen()
        .expect("ordinary standalone reopen");
}
