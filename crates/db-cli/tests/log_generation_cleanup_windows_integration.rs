#![cfg(windows)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use db_cli::generation_directory::{
    canonical_generation_name, canonical_marker_name, canonical_staging_marker_name,
    verify_generation_directory,
};
use db_cli::generation_lock::acquire_generation_writer_lease;
use db_cli::generation_marker::{crc32_ieee, encode_commit_marker, CommittedPrefix};
use db_core::KvEngine;
use db_storage_log::LogEngine;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn windows_cleanup_retires_only_obsolete_namespace_history() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("清理-世代");
    fs::create_dir(&directory).expect("create generation directory");
    create_generation(&directory, 1, &[(b"key", b"one"), (b"old", b"value")]);
    write_fixture_marker(&directory, 1);

    let first = run_switch(&directory);
    assert_success("switch to generation 2", &first);
    let second = run_switch(&directory);
    assert_success("switch to generation 3", &second);
    assert_eq!(
        verify_generation_directory(&directory)
            .expect("verify generation 3")
            .summary()
            .authoritative_generation,
        3
    );

    create_generation(&directory, 7, &[(b"future", b"orphan")]);
    fs::write(staging_marker_path(&directory, 2), b"obsolete staging")
        .expect("write obsolete staging");
    fs::write(staging_marker_path(&directory, 8), b"frontier staging")
        .expect("write frontier staging");

    let authoritative_log = generation_path(&directory, 3);
    let authoritative_marker = marker_path(&directory, 3);
    let orphan = generation_path(&directory, 7);
    let frontier_staging = staging_marker_path(&directory, 8);
    let authority_log_before = fs::read(&authoritative_log).expect("read authority log");
    let authority_marker_before = fs::read(&authoritative_marker).expect("read authority marker");
    let orphan_before = fs::read(&orphan).expect("read orphan");
    let frontier_before = fs::read(&frontier_staging).expect("read frontier staging");

    let cleanup = run_cleanup(&directory);
    assert_success("Windows cleanup", &cleanup);
    let summary: Value = serde_json::from_slice(&cleanup.stdout).expect("decode cleanup summary");
    assert_eq!(
        summary["protocol"],
        "append_log_generation_cleanup_windows_v1"
    );
    assert_eq!(summary["authoritative_generation"], 3);
    assert_eq!(
        summary["removed_marker_generation_ids"],
        serde_json::json!([1, 2])
    );
    assert_eq!(
        summary["removed_generation_ids"],
        serde_json::json!([1, 2])
    );
    assert_eq!(
        summary["removed_staging_marker_generation_ids"],
        serde_json::json!([2])
    );
    assert_eq!(
        summary["retained_staging_marker_generation_ids"],
        serde_json::json!([8])
    );
    assert_eq!(
        summary["retained_uncommitted_generation_ids"],
        serde_json::json!([7])
    );
    assert!(summary.get("retained_quarantine_paths").is_none());

    for id in [1, 2] {
        assert!(!generation_path(&directory, id).exists());
        assert!(!marker_path(&directory, id).exists());
        assert!(!retired_path(&directory, "generation", id).exists());
        assert!(!retired_path(&directory, "marker", id).exists());
    }
    assert!(!staging_marker_path(&directory, 2).exists());
    assert!(!retired_path(&directory, "staging", 2).exists());

    assert_eq!(
        fs::read(&authoritative_log).expect("re-read authority log"),
        authority_log_before
    );
    assert_eq!(
        fs::read(&authoritative_marker).expect("re-read authority marker"),
        authority_marker_before
    );
    assert_eq!(fs::read(&orphan).expect("re-read orphan"), orphan_before);
    assert_eq!(
        fs::read(&frontier_staging).expect("re-read frontier staging"),
        frontier_before
    );

    let verified = verify_generation_directory(&directory).expect("verify cleaned directory");
    assert_eq!(verified.summary().authoritative_generation, 3);
    assert_eq!(verified.summary().reservation_generation_ids, vec![2, 3]);
    assert_eq!(verified.summary().uncommitted_generation_ids, vec![7]);
    assert_eq!(verified.summary().staging_marker_generation_ids, vec![8]);

    let repeated = run_cleanup(&directory);
    assert_success("repeat Windows cleanup", &repeated);
    let repeated: Value = serde_json::from_slice(&repeated.stdout).expect("decode repeat summary");
    assert_eq!(repeated["removed_marker_generation_ids"], serde_json::json!([]));
    assert_eq!(repeated["removed_generation_ids"], serde_json::json!([]));
    assert_eq!(
        repeated["removed_staging_marker_generation_ids"],
        serde_json::json!([])
    );
}

#[test]
fn windows_cleanup_respects_writer_lease_and_no_overwrite_quarantine() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("lease-cleanup");
    fs::create_dir(&directory).expect("create generation directory");
    create_generation(&directory, 1, &[(b"key", b"one")]);
    write_fixture_marker(&directory, 1);
    assert_success("switch to generation 2", &run_switch(&directory));

    let lease = acquire_generation_writer_lease(&directory).expect("hold writer lease");
    let blocked = run_cleanup(&directory);
    assert_failure_contains(&blocked, "already held or stale");
    assert!(generation_path(&directory, 1).is_file());
    assert!(marker_path(&directory, 1).is_file());
    drop(lease);

    let marker_quarantine = retired_path(&directory, "marker", 1);
    fs::write(&marker_quarantine, b"collision").expect("write quarantine collision");
    let collision = run_cleanup(&directory);
    assert_failure_contains(&collision, "cleanup quarantine already exists");
    assert!(generation_path(&directory, 1).is_file());
    assert!(marker_path(&directory, 1).is_file());
    assert_eq!(
        fs::read(&marker_quarantine).expect("read collision"),
        b"collision"
    );
}

fn create_generation(directory: &Path, id: u64, entries: &[(&[u8], &[u8])]) {
    let mut engine =
        LogEngine::create_new(generation_path(directory, id)).expect("create generation");
    for (key, value) in entries {
        engine.put(key, value).expect("put generation entry");
    }
}

fn write_fixture_marker(directory: &Path, id: u64) {
    let generation = generation_path(directory, id);
    let report = LogEngine::verify(&generation).expect("verify fixture generation");
    let bytes = fs::read(&generation).expect("read fixture generation");
    let proof = CommittedPrefix {
        bytes: report.file_bytes,
        crc32: crc32_ieee(&bytes),
        record_count: report.record_count,
        next_sequence: report.next_sequence,
    };
    fs::write(
        marker_path(directory, id),
        encode_commit_marker(id, proof).expect("encode fixture marker"),
    )
    .expect("write fixture marker");
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

fn retired_path(directory: &Path, kind: &str, id: u64) -> PathBuf {
    let name = directory.file_name().expect("directory final component");
    let mut retired = OsString::from(".");
    retired.push(name);
    retired.push(format!(
        ".append-log-retired-{kind}-{id:020}.quarantine"
    ));
    directory.with_file_name(retired)
}

fn run_switch(directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-compact-switch"))
        .arg("--directory")
        .arg(directory)
        .output()
        .expect("run compact switch")
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
