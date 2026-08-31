use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(unix)]
use db_cli::generation_directory::{
    canonical_generation_name, canonical_reservation_name, canonical_staging_marker_name,
    verify_generation_directory,
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
fn reservations_advance_monotonically_and_include_existing_frontier_evidence() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_initial_generation(&directory);

    let first = run_reserve(&directory);
    assert_success("reserve generation 2", &first);
    let first_summary: Value = serde_json::from_slice(&first.stdout).expect("decode first summary");
    assert_eq!(first_summary["protocol"], "append_log_generation_reservation_unix_v1");
    assert_eq!(first_summary["generation"], 2);
    assert_eq!(first_summary["highest_observed_generation"], 2);
    assert_eq!(
        fs::metadata(directory.join(canonical_reservation_name(2)))
            .expect("reservation 2 metadata")
            .len(),
        0
    );

    let second = run_reserve(&directory);
    assert_success("reserve generation 3", &second);
    let verified = verify_generation_directory(&directory).expect("verify reservations");
    assert_eq!(verified.summary().protocol, "append_log_generation_directory_v3");
    assert_eq!(verified.summary().authoritative_generation, 1);
    assert_eq!(verified.summary().reservation_generation_ids, vec![2, 3]);
    assert_eq!(verified.summary().highest_observed_generation, 3);

    fs::write(
        directory.join(canonical_generation_name(9)),
        b"abandoned-uncommitted-candidate",
    )
    .expect("write orphan generation 9");
    fs::write(
        directory.join(canonical_staging_marker_name(11)),
        b"non-authoritative-staging-residue",
    )
    .expect("write staging frontier 11");

    let third = run_reserve(&directory);
    assert_success("reserve above orphan/staging frontier", &third);
    let third_summary: Value = serde_json::from_slice(&third.stdout).expect("decode third summary");
    assert_eq!(third_summary["generation"], 12);
    assert_eq!(third_summary["highest_observed_generation"], 12);

    let verified = verify_generation_directory(&directory).expect("verify final frontier");
    assert_eq!(verified.summary().reservation_generation_ids, vec![2, 3, 12]);
    assert_eq!(verified.summary().highest_observed_generation, 12);
    assert_eq!(verified.summary().uncommitted_generation_ids, vec![9]);
    assert_eq!(verified.summary().staging_marker_generation_ids, vec![11]);
}

#[cfg(unix)]
#[test]
fn held_writer_lease_blocks_reservation_without_creating_frontier_file() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_initial_generation(&directory);

    let lease = acquire_generation_writer_lease(&directory).expect("hold writer lease");
    let output = run_reserve(&directory);
    assert_failure_contains(&output, "already held or stale");
    assert!(!directory.join(canonical_reservation_name(2)).exists());
    drop(lease);

    assert_success("reserve after lease release", &run_reserve(&directory));
}

#[cfg(unix)]
#[test]
fn nonempty_reservation_evidence_fails_closed() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_initial_generation(&directory);
    fs::write(
        directory.join(canonical_reservation_name(7)),
        b"corrupt-reservation-content",
    )
    .expect("write corrupt reservation");

    let output = run_reserve(&directory);
    assert_failure_contains(&output, "generation reservation must contain zero bytes");
    assert!(!directory.join(canonical_reservation_name(8)).exists());
}

#[cfg(not(unix))]
#[test]
fn unsupported_platform_fails_before_filesystem_access() {
    let root = tempdir().expect("temporary root");
    let missing = root.path().join("must-not-be-created");
    let output = run_reserve(&missing);
    assert_failure_contains(&output, "unsupported on this platform");
    assert!(!missing.exists());
}

#[cfg(unix)]
fn create_initial_generation(directory: &Path) {
    let generation = directory.join(canonical_generation_name(1));
    let mut engine = LogEngine::create_new(&generation).expect("create generation 1");
    engine.put(b"key", b"value").expect("write generation 1");
    drop(engine);
    publish_generation_marker(directory, 1).expect("publish generation 1");
}

fn run_reserve(directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-reserve"))
        .arg("--directory")
        .arg(directory)
        .output()
        .expect("run generation reservation CLI")
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
