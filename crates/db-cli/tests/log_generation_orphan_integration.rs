use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::{Command, Output};

#[cfg(unix)]
use db_cli::generation_directory::{
    canonical_generation_name, canonical_staging_marker_name, verify_generation_directory,
};
#[cfg(unix)]
use db_cli::generation_lock::acquire_generation_writer_lease;
#[cfg(unix)]
use db_cli::generation_publication::publish_generation_marker;
#[cfg(unix)]
use db_core::KvEngine;
#[cfg(unix)]
use db_storage_log::LogEngine;
#[cfg(unix)]
use serde_json::Value;
use tempfile::tempdir;

#[cfg(unix)]
#[test]
fn guarded_retirement_reclaims_orphan_and_preserves_generation_frontier() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_and_publish(&directory, 1, &[(b"key", b"authority")]);
    create_generation(&directory, 5, &[(b"candidate", b"large-enough")]);

    let inspected = run_inspect(&directory, 5);
    assert_success("inspect orphan", &inspected);
    let inspection: Value = serde_json::from_slice(&inspected.stdout).expect("decode inspection");
    assert_eq!(inspection["protocol"], "append_log_generation_orphan_inspect_v1");
    assert_eq!(inspection["authoritative_generation"], 1);
    assert_eq!(inspection["orphan_generation"], 5);
    assert_eq!(inspection["staging_frontier_present"], false);
    let bytes = inspection["fingerprint"]["bytes"].as_u64().expect("fingerprint bytes");
    let crc32 = inspection["fingerprint"]["crc32"].as_u64().expect("fingerprint crc") as u32;

    let unconfirmed = run_retire(&directory, 5, 1, bytes, crc32, false);
    assert_failure_contains(&unconfirmed, "requires --confirm-generation-builder-stopped");
    assert!(generation_path(&directory, 5).is_file());
    assert!(!staging_path(&directory, 5).exists());

    let wrong_fingerprint = run_retire(&directory, 5, 1, bytes, crc32 ^ 1, true);
    assert_failure_contains(&wrong_fingerprint, "fingerprint changed");
    assert!(generation_path(&directory, 5).is_file());
    assert!(!staging_path(&directory, 5).exists());

    let retired = run_retire(&directory, 5, 1, bytes, crc32, true);
    assert_success("retire orphan", &retired);
    let summary: Value = serde_json::from_slice(&retired.stdout).expect("decode retirement");
    assert_eq!(summary["protocol"], "append_log_generation_orphan_retire_unix_v1");
    assert_eq!(summary["authoritative_generation"], 1);
    assert_eq!(summary["retired_generation"], 5);
    assert_eq!(summary["staging_frontier_created"], true);
    assert!(!generation_path(&directory, 5).exists());
    assert!(staging_path(&directory, 5).is_file());

    let verified = verify_generation_directory(&directory).expect("verify after retirement");
    assert_eq!(verified.summary().authoritative_generation, 1);
    assert_eq!(verified.summary().highest_observed_generation, 5);
    assert_eq!(verified.next_generation_id().expect("next generation"), 6);
}

#[cfg(unix)]
#[test]
fn retirement_never_overwrites_existing_frontier_evidence() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_and_publish(&directory, 1, &[(b"key", b"authority")]);
    create_generation(&directory, 7, &[(b"candidate", b"abandoned")]);
    let staging = staging_path(&directory, 7);
    fs::write(&staging, b"existing-frontier-evidence").expect("write existing frontier");
    let staging_before = fs::read(&staging).expect("read existing frontier");

    let inspection = inspect_json(&directory, 7);
    assert_eq!(inspection["staging_frontier_present"], true);
    let bytes = inspection["fingerprint"]["bytes"].as_u64().expect("fingerprint bytes");
    let crc32 = inspection["fingerprint"]["crc32"].as_u64().expect("fingerprint crc") as u32;

    let retired = run_retire(&directory, 7, 1, bytes, crc32, true);
    assert_success("retire with existing frontier", &retired);
    let summary: Value = serde_json::from_slice(&retired.stdout).expect("decode retirement");
    assert_eq!(summary["staging_frontier_created"], false);
    assert_eq!(
        fs::read(&staging).expect("re-read existing frontier"),
        staging_before,
        "retirement must not rewrite existing staging-frontier bytes"
    );
    assert!(!generation_path(&directory, 7).exists());
    let verified = verify_generation_directory(&directory).expect("verify after retirement");
    assert_eq!(verified.summary().highest_observed_generation, 7);
    assert_eq!(verified.next_generation_id().expect("next generation"), 8);
}

#[cfg(unix)]
#[test]
fn changed_orphan_or_held_writer_lease_fails_without_retirement() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_and_publish(&directory, 1, &[(b"key", b"authority")]);
    create_generation(&directory, 4, &[(b"candidate", b"before")]);

    let inspection = inspect_json(&directory, 4);
    let bytes = inspection["fingerprint"]["bytes"].as_u64().expect("fingerprint bytes");
    let crc32 = inspection["fingerprint"]["crc32"].as_u64().expect("fingerprint crc") as u32;
    {
        let mut engine = LogEngine::open(generation_path(&directory, 4)).expect("open orphan");
        engine.put(b"late", b"change").expect("mutate orphan after inspect");
    }
    let changed = run_retire(&directory, 4, 1, bytes, crc32, true);
    assert_failure_contains(&changed, "fingerprint changed");
    assert!(generation_path(&directory, 4).is_file());
    assert!(!staging_path(&directory, 4).exists());

    let current = inspect_json(&directory, 4);
    let current_bytes = current["fingerprint"]["bytes"].as_u64().expect("current bytes");
    let current_crc = current["fingerprint"]["crc32"].as_u64().expect("current crc") as u32;
    let lease = acquire_generation_writer_lease(&directory).expect("hold writer lease");
    let blocked = run_retire(&directory, 4, 1, current_bytes, current_crc, true);
    assert_failure_contains(&blocked, "already held or stale");
    assert!(generation_path(&directory, 4).is_file());
    assert!(!staging_path(&directory, 4).exists());
    drop(lease);
}

#[cfg(not(unix))]
#[test]
fn unsupported_retirement_fails_before_filesystem_access() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("must-not-be-created");
    assert!(!directory.exists());

    let output = run_retire(&directory, 5, 1, 123, 456, true);
    assert_failure_contains(&output, "unsupported on this platform");
    assert!(
        !directory.exists(),
        "unsupported retirement must fail before touching the supplied path"
    );
}

#[cfg(unix)]
fn inspect_json(directory: &Path, generation: u64) -> Value {
    let output = run_inspect(directory, generation);
    assert_success("inspect orphan", &output);
    serde_json::from_slice(&output.stdout).expect("decode orphan inspection")
}

#[cfg(unix)]
fn create_and_publish(directory: &Path, id: u64, entries: &[(&[u8], &[u8])]) {
    create_generation(directory, id, entries);
    publish_generation_marker(directory, id).expect("publish generation marker");
}

#[cfg(unix)]
fn create_generation(directory: &Path, id: u64, entries: &[(&[u8], &[u8])]) {
    let mut engine = LogEngine::create_new(generation_path(directory, id)).expect("create generation");
    for (key, value) in entries {
        engine.put(key, value).expect("put generation entry");
    }
}

#[cfg(unix)]
fn generation_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_generation_name(id))
}

#[cfg(unix)]
fn staging_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_staging_marker_name(id))
}

fn run_inspect(directory: &Path, generation: u64) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-orphan"))
        .arg("inspect")
        .arg("--directory")
        .arg(directory)
        .arg("--generation")
        .arg(generation.to_string())
        .output()
        .expect("run orphan inspection")
}

fn run_retire(
    directory: &Path,
    generation: u64,
    authority: u64,
    bytes: u64,
    crc32: u32,
    confirm: bool,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-orphan"));
    command
        .arg("retire")
        .arg("--directory")
        .arg(directory)
        .arg("--generation")
        .arg(generation.to_string())
        .arg("--expected-authority")
        .arg(authority.to_string())
        .arg("--expected-bytes")
        .arg(bytes.to_string())
        .arg("--expected-crc32")
        .arg(crc32.to_string());
    if confirm {
        command.arg("--confirm-generation-builder-stopped");
    }
    command.output().expect("run orphan retirement")
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
