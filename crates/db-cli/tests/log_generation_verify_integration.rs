use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use db_core::KvEngine;
use db_storage_log::LogEngine;
use serde_json::Value;
use tempfile::tempdir;

const COMMIT_MARKER_MAGIC: [u8; 8] = *b"DBLGCMT\0";
const COMMIT_MARKER_VERSION: u16 = 1;
const COMMIT_MARKER_LEN: usize = 32;
const APPEND_LOG_FORMAT_VERSION: u16 = 1;

#[test]
fn verifier_selects_highest_commit_and_ignores_higher_uncommitted_orphans() {
    let directory = tempdir().expect("temporary directory");
    create_generation(directory.path(), 1, &[(b"a", b"old")]);
    write_marker(directory.path(), 1, 1);

    create_generation(
        directory.path(),
        2,
        &[(b"a", b"new"), (b"b", b"three")],
    );
    write_marker(directory.path(), 2, 2);

    create_generation(directory.path(), 3, &[(b"orphan", b"complete")]);
    fs::write(generation_path(directory.path(), 4), b"incomplete-or-corrupt-orphan")
        .expect("write corrupt uncommitted orphan");

    let output = run_verify(directory.path());
    assert_success("verify generation directory", &output);
    let summary: Value = serde_json::from_slice(&output.stdout).expect("decode summary");
    assert_eq!(summary["protocol"], "append_log_generation_directory_v1");
    assert_eq!(summary["marker_format_version"], 1);
    assert_eq!(summary["authoritative_generation"], 2);
    assert_eq!(
        summary["authoritative_log"],
        "generation-00000000000000000002.log"
    );
    assert_eq!(summary["highest_observed_generation"], 4);
    assert_eq!(summary["marker_generation_ids"], serde_json::json!([1, 2]));
    assert_eq!(
        summary["uncommitted_generation_ids"],
        serde_json::json!([3, 4])
    );
    assert_eq!(summary["log_verification"]["record_count"], 2);
    assert_eq!(summary["log_verification"]["live_keys"], 2);
    assert!(summary["log_verification"]["recoverable_tail"].is_null());
}

#[test]
fn corrupt_highest_marker_never_falls_back_to_older_commit() {
    let directory = tempdir().expect("temporary directory");
    create_generation(directory.path(), 1, &[(b"key", b"old")]);
    write_marker(directory.path(), 1, 1);
    create_generation(directory.path(), 2, &[(b"key", b"new")]);
    write_marker(directory.path(), 2, 2);

    let marker = marker_path(directory.path(), 2);
    let mut bytes = fs::read(&marker).expect("read marker");
    bytes[12] ^= 0x80;
    fs::write(&marker, bytes).expect("corrupt highest marker");

    let output = run_verify(directory.path());
    assert_failure_contains(&output, "highest commit marker checksum mismatch");
}

#[test]
fn missing_highest_committed_log_never_falls_back() {
    let directory = tempdir().expect("temporary directory");
    create_generation(directory.path(), 1, &[(b"key", b"old")]);
    write_marker(directory.path(), 1, 1);
    write_marker(directory.path(), 2, 2);

    let output = run_verify(directory.path());
    assert_failure_contains(
        &output,
        "highest committed generation 2 has no generation log",
    );
}

#[test]
fn verifier_reports_recoverable_authoritative_tail_without_repairing_it() {
    let directory = tempdir().expect("temporary directory");
    create_generation(
        directory.path(),
        1,
        &[(b"a", b"one"), (b"b", b"two")],
    );
    write_marker(directory.path(), 1, 1);
    let log = generation_path(directory.path(), 1);
    let length = fs::metadata(&log).expect("log metadata").len();
    fs::OpenOptions::new()
        .write(true)
        .open(&log)
        .expect("open log for truncation")
        .set_len(length - 1)
        .expect("truncate final append");
    let before = fs::read(&log).expect("read truncated log");

    let output = run_verify(directory.path());
    assert_success("verify recoverable authoritative tail", &output);
    let summary: Value = serde_json::from_slice(&output.stdout).expect("decode summary");
    assert_eq!(summary["authoritative_generation"], 1);
    assert!(summary["log_verification"]["recoverable_tail"].is_object());
    assert_eq!(
        fs::read(&log).expect("read log after verification"),
        before,
        "read-only generation verification must not repair a recoverable tail"
    );
}

#[test]
fn marker_generation_must_match_filename_and_namespace_is_strict() {
    let directory = tempdir().expect("temporary directory");
    create_generation(directory.path(), 1, &[(b"key", b"value")]);
    write_marker(directory.path(), 1, 2);

    let mismatch = run_verify(directory.path());
    assert_failure_contains(&mismatch, "disagrees with filename generation 1");

    fs::remove_file(marker_path(directory.path(), 1)).expect("remove mismatched marker");
    write_marker(directory.path(), 1, 1);
    fs::write(directory.path().join("README.txt"), b"unexpected")
        .expect("write unexpected entry");
    let unexpected = run_verify(directory.path());
    assert_failure_contains(&unexpected, "unexpected generation directory entry");
}

fn create_generation(directory: &Path, id: u64, entries: &[(&[u8], &[u8])]) {
    let mut engine = LogEngine::create_new(generation_path(directory, id)).expect("create generation");
    for (key, value) in entries {
        engine.put(key, value).expect("put generation entry");
    }
}

fn write_marker(directory: &Path, filename_id: u64, encoded_id: u64) {
    fs::write(marker_path(directory, filename_id), encode_marker(encoded_id)).expect("write marker");
}

fn generation_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(format!("generation-{id:020}.log"))
}

fn marker_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(format!("commit-{id:020}.marker"))
}

fn encode_marker(generation_id: u64) -> [u8; COMMIT_MARKER_LEN] {
    let mut bytes = [0_u8; COMMIT_MARKER_LEN];
    bytes[..8].copy_from_slice(&COMMIT_MARKER_MAGIC);
    bytes[8..10].copy_from_slice(&COMMIT_MARKER_VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(COMMIT_MARKER_LEN as u16).to_le_bytes());
    bytes[12..20].copy_from_slice(&generation_id.to_le_bytes());
    bytes[20..22].copy_from_slice(&APPEND_LOG_FORMAT_VERSION.to_le_bytes());
    bytes[22..24].copy_from_slice(&0_u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&0_u32.to_le_bytes());
    let checksum = crc32fast::hash(&bytes[..28]);
    bytes[28..32].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

fn run_verify(directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-verify"))
        .arg("--directory")
        .arg(directory)
        .output()
        .expect("run generation verifier")
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
