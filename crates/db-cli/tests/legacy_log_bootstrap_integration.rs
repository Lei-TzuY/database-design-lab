use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::{Command, Output};

#[cfg(unix)]
use db_cli::generation_directory::{
    canonical_generation_name, canonical_marker_name, canonical_reservation_name,
    canonical_staging_marker_name, verify_generation_directory,
};
#[cfg(unix)]
use db_cli::generation_engine::GenerationLogEngine;
#[cfg(unix)]
use db_core::KvEngine;
#[cfg(unix)]
use db_storage_log::LogEngine;
#[cfg(unix)]
use serde_json::Value;
use tempfile::tempdir;

#[cfg(unix)]
#[test]
fn bootstrap_preserves_legacy_bytes_and_produces_routable_generation_one() {
    let root = tempdir().expect("temporary root");
    let source = root.path().join("legacy.db");
    let target = root.path().join("generations");
    {
        let mut engine = LogEngine::create_new(&source).expect("create legacy source");
        engine.put(b"a", b"one").expect("put a one");
        engine.put(b"a", b"two").expect("overwrite a");
        engine.put(b"b", b"three").expect("put b");
        engine.delete(b"b").expect("delete b");
        engine.put(b"c", b"").expect("put c");
        engine.delete(b"missing").expect("delete missing");
    }
    let source_before = fs::read(&source).expect("read source before bootstrap");

    let output = run_bootstrap(&source, &target);
    assert_success("bootstrap legacy log", &output);
    let summary: Value = serde_json::from_slice(&output.stdout).expect("decode bootstrap summary");
    assert_eq!(summary["protocol"], "append_log_legacy_bootstrap_unix_v1");
    assert_eq!(summary["generation"], 1);
    assert_eq!(summary["reservation"], canonical_reservation_name(1));
    assert_eq!(summary["source_verification"]["record_count"], 6);
    assert_eq!(summary["compaction"]["live_keys"], 2);
    assert_eq!(summary["compaction"]["compacted_record_count"], 2);
    assert_eq!(summary["publication"]["generation"], 1);
    assert_eq!(summary["final_generation"]["authoritative_generation"], 1);
    assert_eq!(summary["final_generation"]["reservation_generation_ids"], serde_json::json!([1]));

    assert_eq!(
        fs::read(&source).expect("read source after bootstrap"),
        source_before,
        "bootstrap must not mutate legacy source bytes"
    );
    assert!(target.join(canonical_generation_name(1)).is_file());
    assert!(target.join(canonical_marker_name(1)).is_file());
    assert!(target.join(canonical_reservation_name(1)).is_file());
    assert!(!target.join(canonical_staging_marker_name(1)).exists());

    let verified = verify_generation_directory(&target).expect("verify bootstrapped directory");
    assert_eq!(verified.summary().authoritative_generation, 1);
    assert_eq!(verified.summary().highest_observed_generation, 1);
    assert_eq!(verified.summary().reservation_generation_ids, vec![1]);
    assert_eq!(verified.summary().log_verification.record_count, 2);

    let mut routed = GenerationLogEngine::open(&target).expect("open generation routing engine");
    assert_eq!(routed.get(b"a").expect("get a"), Some(b"two".to_vec()));
    assert_eq!(routed.get(b"b").expect("get b"), None);
    assert_eq!(routed.get(b"c").expect("get c"), Some(Vec::new()));
    routed.put(b"new", b"generation-only").expect("write new generation");
    assert_eq!(
        fs::read(&source).expect("read source after routed mutation"),
        source_before,
        "post-cutover routed mutations must not touch the retained legacy source"
    );
    assert_eq!(
        routed.get(b"new").expect("get routed mutation"),
        Some(b"generation-only".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn bootstrap_rejects_recoverable_legacy_tail_before_creating_target() {
    let root = tempdir().expect("temporary root");
    let source = root.path().join("legacy-tail.db");
    let target = root.path().join("generations");
    {
        let mut engine = LogEngine::create_new(&source).expect("create legacy source");
        engine.put(b"a", b"one").expect("put a");
        engine.put(b"b", b"two").expect("put b");
    }
    let length = fs::metadata(&source).expect("source metadata").len();
    fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .expect("open source for truncation")
        .set_len(length - 1)
        .expect("truncate final append");
    let source_before = fs::read(&source).expect("read truncated source");

    let output = run_bootstrap(&source, &target);
    assert_failure_contains(&output, "complete clean append-log image");
    assert!(!target.exists(), "invalid source must not create bootstrap target");
    assert_eq!(
        fs::read(&source).expect("read source after rejection"),
        source_before,
        "bootstrap must not repair legacy tails implicitly"
    );
}

#[cfg(unix)]
#[test]
fn bootstrap_requires_fresh_target_and_preserves_existing_directory() {
    let root = tempdir().expect("temporary root");
    let source = root.path().join("legacy.db");
    create_legacy(&source, &[(b"key", b"value")]);
    let target = root.path().join("generations");
    fs::create_dir(&target).expect("create pre-existing target");
    fs::write(target.join("sentinel"), b"keep-me").expect("write sentinel");

    let output = run_bootstrap(&source, &target);
    assert_failure_contains(&output, "target generation directory already exists");
    assert_eq!(
        fs::read(target.join("sentinel")).expect("read sentinel"),
        b"keep-me"
    );
}

#[cfg(not(unix))]
#[test]
fn unsupported_bootstrap_fails_before_filesystem_access() {
    let root = tempdir().expect("temporary root");
    let source = root.path().join("missing-legacy.db");
    let target = root.path().join("must-not-exist");
    assert!(!source.exists());
    assert!(!target.exists());

    let output = run_bootstrap(&source, &target);
    assert_failure_contains(&output, "unsupported on this platform");
    assert!(!source.exists());
    assert!(!target.exists());
}

#[cfg(unix)]
fn create_legacy(path: &Path, entries: &[(&[u8], &[u8])]) {
    let mut engine = LogEngine::create_new(path).expect("create legacy source");
    for (key, value) in entries {
        engine.put(key, value).expect("put legacy entry");
    }
}

fn run_bootstrap(source: &Path, target: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-bootstrap"))
        .arg("--source")
        .arg(source)
        .arg("--target-directory")
        .arg(target)
        .output()
        .expect("run legacy bootstrap")
}

#[cfg(unix)]
fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure_contains(output: &Output, needle: &str) {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(needle),
        "stderr did not contain {needle:?}:\n{stderr}"
    );
}
