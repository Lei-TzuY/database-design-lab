use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use db_core::KvEngine;
use db_storage_log::LogEngine;
use serde_json::Value;
use tempfile::tempdir;

use db_cli::generation_directory::{
    canonical_generation_name, canonical_marker_name, canonical_reservation_name,
    canonical_staging_marker_name, verify_generation_directory,
};
use db_cli::generation_marker::{encode_commit_marker, CommittedPrefix, Crc32Ieee};

#[test]
fn exact_plan_rejects_drift_then_removes_only_confirmed_reserved_abandoned_artifacts() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_generation(&directory, 1, &[(b"stable", b"authority")]);
    write_synthetic_marker(&directory, 1);
    create_reservation(&directory, 2);
    create_generation(&directory, 2, &[(b"candidate", b"before")]);
    fs::write(staging_path(&directory, 2), b"staged proof").expect("write staging marker");

    let first_plan = run_plan(&directory);
    assert_success("plan abandoned cleanup", &first_plan);
    let first_json: Value = serde_json::from_slice(&first_plan.stdout).expect("decode first plan");
    assert_eq!(first_json["eligible_artifacts"][0]["generation"], 2);
    assert_eq!(
        first_json["directory"]["reservation_generation_ids"],
        serde_json::json!([2])
    );
    let first_plan_path = root.path().join("first-plan.json");
    fs::write(&first_plan_path, &first_plan.stdout).expect("save first plan");

    let unconfirmed = run_apply(&directory, &first_plan_path, false);
    assert_failure_contains(&unconfirmed, "--confirm-abandoned");
    assert!(generation_path(&directory, 2).is_file());
    assert!(staging_path(&directory, 2).is_file());

    {
        let mut candidate = LogEngine::open(generation_path(&directory, 2)).expect("open candidate");
        candidate
            .put(b"candidate", b"changed")
            .expect("mutate candidate after plan");
    }
    let stale_apply = run_apply(&directory, &first_plan_path, true);
    assert_failure_contains(&stale_apply, "plan no longer matches");
    assert!(generation_path(&directory, 2).is_file());
    assert!(staging_path(&directory, 2).is_file());

    let refreshed_plan = run_plan(&directory);
    assert_success("refresh abandoned cleanup plan", &refreshed_plan);
    let refreshed_plan_path = root.path().join("refreshed-plan.json");
    fs::write(&refreshed_plan_path, &refreshed_plan.stdout).expect("save refreshed plan");
    let applied = run_apply(&directory, &refreshed_plan_path, true);

    #[cfg(unix)]
    {
        assert_success("apply abandoned cleanup", &applied);
        let summary: Value = serde_json::from_slice(&applied.stdout).expect("decode cleanup summary");
        assert_eq!(
            summary["protocol"],
            "append_log_abandoned_generation_cleanup_unix_v1"
        );
        assert_eq!(summary["removed_generation_ids"], serde_json::json!([2]));
        assert_eq!(
            summary["removed_staging_marker_generation_ids"],
            serde_json::json!([2])
        );
        assert_eq!(
            summary["retained_reservation_generation_ids"],
            serde_json::json!([2])
        );
        assert!(!generation_path(&directory, 2).exists());
        assert!(!staging_path(&directory, 2).exists());
        assert!(reservation_path(&directory, 2).is_file());
        let verified = verify_generation_directory(&directory).expect("verify retained authority");
        assert_eq!(verified.summary().authoritative_generation, 1);
        assert_eq!(verified.summary().reservation_generation_ids, vec![2]);

        let reserve = run_reserve(&directory);
        assert_success("reserve after cleanup", &reserve);
        let reserve: Value = serde_json::from_slice(&reserve.stdout).expect("decode reservation");
        assert_eq!(reserve["generation"], 3);
    }

    #[cfg(not(unix))]
    {
        assert_failure_contains(&applied, "unsupported on this platform");
        assert!(generation_path(&directory, 2).is_file());
        assert!(staging_path(&directory, 2).is_file());
        assert!(reservation_path(&directory, 2).is_file());
    }
}

#[test]
fn unreserved_higher_artifacts_are_reported_but_never_eligible_for_deletion() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_generation(&directory, 1, &[(b"stable", b"authority")]);
    write_synthetic_marker(&directory, 1);
    create_generation(&directory, 3, &[(b"unreserved", b"candidate")]);
    fs::write(staging_path(&directory, 4), b"unreserved staging").expect("write staging");

    let plan = run_plan(&directory);
    assert_success("plan unreserved artifacts", &plan);
    let plan_json: Value = serde_json::from_slice(&plan.stdout).expect("decode plan");
    assert_eq!(plan_json["eligible_artifacts"], serde_json::json!([]));
    assert_eq!(
        plan_json["blocked_unreserved_generation_ids"],
        serde_json::json!([3])
    );
    assert_eq!(
        plan_json["blocked_unreserved_staging_marker_generation_ids"],
        serde_json::json!([4])
    );
    let plan_path = root.path().join("plan.json");
    fs::write(&plan_path, &plan.stdout).expect("save plan");
    let applied = run_apply(&directory, &plan_path, true);

    #[cfg(unix)]
    assert_success("apply empty abandoned plan", &applied);
    #[cfg(not(unix))]
    assert_failure_contains(&applied, "unsupported on this platform");

    assert!(generation_path(&directory, 3).is_file());
    assert!(staging_path(&directory, 4).is_file());
}

#[cfg(unix)]
#[test]
fn marker_publication_after_plan_invalidates_cleanup_before_any_deletion() {
    let root = tempdir().expect("temporary root");
    let directory = root.path().join("generations");
    fs::create_dir(&directory).expect("create generation directory");
    create_generation(&directory, 1, &[(b"old", b"authority")]);
    write_synthetic_marker(&directory, 1);
    create_reservation(&directory, 2);
    create_generation(&directory, 2, &[(b"new", b"authority")]);

    let plan = run_plan(&directory);
    assert_success("plan generation 2 as abandoned", &plan);
    let plan_path = root.path().join("stale-plan.json");
    fs::write(&plan_path, &plan.stdout).expect("save stale plan");

    let publish = run_publish(&directory, 2);
    assert_success("publish generation 2", &publish);
    let applied = run_apply(&directory, &plan_path, true);
    assert_failure_contains(&applied, "plan no longer matches");
    assert!(generation_path(&directory, 2).is_file());
    assert!(marker_path(&directory, 2).is_file());
    let verified = verify_generation_directory(&directory).expect("verify new authority");
    assert_eq!(verified.summary().authoritative_generation, 2);
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
    .expect("encode marker");
    fs::write(marker_path(directory, id), marker).expect("write marker");
}

fn create_reservation(directory: &Path, id: u64) {
    fs::write(reservation_path(directory, id), b"").expect("write zero-byte reservation");
}

fn generation_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_generation_name(id))
}

fn marker_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_marker_name(id))
}

fn staging_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_staging_marker_name(id))
}

fn reservation_path(directory: &Path, id: u64) -> PathBuf {
    directory.join(canonical_reservation_name(id))
}

fn run_plan(directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-abandon-cleanup"))
        .arg("plan")
        .arg("--directory")
        .arg(directory)
        .output()
        .expect("run abandoned cleanup plan")
}

fn run_apply(directory: &Path, plan: &Path, confirm: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-abandon-cleanup"));
    command
        .arg("apply")
        .arg("--directory")
        .arg(directory)
        .arg("--plan")
        .arg(plan);
    if confirm {
        command.arg("--confirm-abandoned");
    }
    command.output().expect("run abandoned cleanup apply")
}

#[cfg(unix)]
fn run_publish(directory: &Path, id: u64) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-publish"))
        .arg("--directory")
        .arg(directory)
        .arg("--generation")
        .arg(id.to_string())
        .output()
        .expect("run generation publisher")
}

#[cfg(unix)]
fn run_reserve(directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-log-generation-reserve"))
        .arg("--directory")
        .arg(directory)
        .output()
        .expect("run generation reservation")
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
