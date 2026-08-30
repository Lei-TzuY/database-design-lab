use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use db_cli::generation_directory::{
    canonical_generation_name, canonical_marker_name, canonical_staging_marker_name,
    verify_generation_directory,
};
use db_cli::generation_lock::acquire_generation_writer_lease;
use db_cli::generation_marker::{encode_commit_marker, CommittedPrefix, Crc32Ieee};
use db_core::KvEngine;
use db_storage_log::LogEngine;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn cleanup_reclaims_only_ids_below_authority_and_preserves_allocation_frontier() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");

    create_generation(&directory, 1, &[(b"old", b"one")]);
    write_synthetic_marker(&directory, 1);
    create_generation(&directory, 2, &[(b"orphan-low", b"two")]);
    fs::write(staging_marker_path(&directory, 2), b"low staging")
        .expect("write lower staging marker");
    create_generation(&directory, 3, &[(b"current", b"three")]);
    write_synthetic_marker(&directory, 3);
    create_generation(&directory, 8, &[(b"orphan-high", b"eight")]);
    fs::write(staging_marker_path(&directory, 9), b"high staging")
        .expect("write higher staging marker");

    let current_log_before = fs::read(generation_path(&directory, 3)).expect("read current log");
    let current_marker_before = fs::read(marker_path(&directory, 3)).expect("read current marker");

    let output = run_cleanup(&directory);
    assert_success("cleanup generation directory", &output);
    let summary: Value = serde_json::from_slice(&output.stdout).expect("decode cleanup summary");
    assert_eq!(summary["protocol"], "append_log_generation_cleanup_v1");
    assert_eq!(summary["authoritative_generation"], 3);
    assert_eq!(summary["highest_observed_generation_before"], 9);
    assert_eq!(summary["highest_observed_generation_after"], 9);
    assert_eq!(summary["removed_generation_ids"], serde_json::json!([1, 2]));
    assert_eq!(summary["removed_marker_generation_ids"], serde_json::json!([1]));
    assert_eq!(
        summary["removed_staging_marker_generation_ids"],
        serde_json::json!([2])
    );
    assert_eq!(
        summary["retained_future_generation_ids"],
        serde_json::json!([8])
    );
    assert_eq!(
        summary["retained_future_staging_marker_generation_ids"],
        serde_json::json!([9])
    );
    assert_eq!(summary["directory_sync_confirmed"], cfg!(unix));

    assert!(!generation_path(&directory, 1).exists());
    assert!(!marker_path(&directory, 1).exists());
    assert!(!generation_path(&directory, 2).exists());
    assert!(!staging_marker_path(&directory, 2).exists());
    assert_eq!(
        fs::read(generation_path(&directory, 3)).expect("re-read current log"),
        current_log_before
    );
    assert_eq!(
        fs::read(marker_path(&directory, 3)).expect("re-read current marker"),
        current_marker_before
    );
    assert!(generation_path(&directory, 8).is_file());
    assert!(staging_marker_path(&directory, 9).is_file());

    let verified = verify_generation_directory(&directory).expect("verify after cleanup");
    assert_eq!(verified.summary().authoritative_generation, 3);
    assert_eq!(
        verified.next_generation_id().expect("next generation after cleanup"),
        10,
        "cleanup must preserve the higher crash-residue allocation frontier"
    );

    let second = run_cleanup(&directory);
    assert_success("idempotent cleanup", &second);
    let second_summary: Value =
        serde_json::from_slice(&second.stdout).expect("decode second cleanup summary");
    assert_eq!(second_summary["removed_generation_ids"], serde_json::json!([]));
    assert_eq!(second_summary["removed_marker_generation_ids"], serde_json::json!([]));
    assert_eq!(
        second_summary["removed_staging_marker_generation_ids"],
        serde_json::json!([])
    );
}

#[test]
fn held_writer_lease_blocks_cleanup_before_any_deletion() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_generation(&directory, 1, &[(b"old", b"one")]);
    write_synthetic_marker(&directory, 1);
    create_generation(&directory, 2, &[(b"current", b"two")]);
    write_synthetic_marker(&directory, 2);
    let old_log = fs::read(generation_path(&directory, 1)).expect("snapshot old log");
    let old_marker = fs::read(marker_path(&directory, 1)).expect("snapshot old marker");

    let lease = acquire_generation_writer_lease(&directory).expect("hold writer lease");
    let output = run_cleanup(&directory);
    assert_failure_contains(&output, "writer lock is already held or stale");
    assert_eq!(
        fs::read(generation_path(&directory, 1)).expect("re-read blocked old log"),
        old_log
    );
    assert_eq!(
        fs::read(marker_path(&directory, 1)).expect("re-read blocked old marker"),
        old_marker
    );
    drop(lease);
}

#[test]
fn invalid_lower_object_fails_before_other_eligible_artifacts_are_deleted() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    fs::create_dir(generation_path(&directory, 1)).expect("create invalid lower generation object");
    fs::write(marker_path(&directory, 1), b"lower marker evidence").expect("write lower marker");
    create_generation(&directory, 2, &[(b"current", b"two")]);
    write_synthetic_marker(&directory, 2);

    let output = run_cleanup(&directory);
    assert_failure_contains(&output, "obsolete generation log must be a real regular file");
    assert!(generation_path(&directory, 1).is_dir());
    assert_eq!(
        fs::read(marker_path(&directory, 1)).expect("read retained lower marker"),
        b"lower marker evidence",
        "preflight failure must happen before any eligible marker is deleted"
    );
    assert_eq!(
        verify_generation_directory(&directory)
            .expect("verify authority after rejected cleanup")
            .summary()
            .authoritative_generation,
        2
    );
}

fn create_generation(directory: &Path, id: u64, entries: &[(&[u8], &[u8])]) {
    let mut log = LogEngine::create_new(generation_path(directory, id)).expect("create generation");
    for (key, value) in entries {
        log.put(key, value).expect("put generation entry");
    }
}

fn write_synthetic_marker(directory: &Path, id: u64) {
    let generation = generation_path(directory, id);
    let report = LogEngine::verify(&generation).expect("verify generation for marker");
    let bytes = fs::read(&generation).expect("read generation for marker CRC");
    let mut crc = Crc32Ieee::new();
    crc.update(&bytes);
    let marker = encode_commit_marker(
        id,
        CommittedPrefix {
            bytes: report.file_bytes,
            crc32: crc.finalize(),
            record_count: report.record_count,
            next_sequence: report.next_sequence,
        },
    )
    .expect("encode synthetic marker");
    fs::write(marker_path(directory, id), marker).expect("write synthetic marker");
}

fn generation_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_generation_name(id))
}

fn marker_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_marker_name(id))
}

fn staging_marker_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_staging_marker_name(id))
}

fn run_cleanup(directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-cleanup"))
        .arg("--directory")
        .arg(directory)
        .output()
        .expect("run generation cleanup")
}

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
