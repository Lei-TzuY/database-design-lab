use std::fs;
use std::process::{Command, Output};

use db_core::{generate_experiment_trace, ExperimentGeneratorConfig, ExperimentProfile};
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn real_batch_archive_verifies_and_manifest_tampering_fails_closed() {
    let directory = tempdir().expect("temporary directory");
    let trace_path = directory.path().join("trace.json");
    let engine_root = directory.path().join("engines");
    let archive_dir = directory.path().join("archive");
    let trace = generate_experiment_trace(ExperimentGeneratorConfig {
        seed: 0x2026_0830,
        profile: ExperimentProfile::RandomWrite,
        operations: 2,
        key_space: 8,
        value_bytes: 8,
        range_limit: 1,
        reopen_every: None,
    })
    .expect("generate trace");
    fs::write(
        &trace_path,
        serde_json::to_vec_pretty(&trace).expect("encode trace"),
    )
    .expect("write trace");

    let producer = Command::new(env!("CARGO_BIN_EXE_db-lab-batch"))
        .arg("--trace")
        .arg(&trace_path)
        .arg("--engine-root")
        .arg(&engine_root)
        .arg("--archive-dir")
        .arg(&archive_dir)
        .arg("--pair-seed")
        .arg("0")
        .arg("--pairs")
        .arg("1")
        .arg("--btree-cache-pages")
        .arg("8")
        .arg("--revision")
        .arg("abc123")
        .output()
        .expect("run db-lab-batch");
    assert_success("db-lab-batch", &producer);

    let verified = run_verifier(&archive_dir);
    assert_success("db-lab-batch-verify", &verified);
    let summary: Value = serde_json::from_slice(&verified.stdout).expect("decode verifier summary");
    assert_eq!(summary["valid"], true);
    assert_eq!(summary["format_version"], 6);
    assert_eq!(summary["repository_revision"], "abc123");
    assert_eq!(summary["requested_pairs"], 1);
    assert_eq!(summary["included_pairs"], 1);
    assert_eq!(summary["failed_pairs"], 0);
    assert_eq!(summary["excluded_pairs"], 0);
    assert_eq!(summary["comparison_failure_sidecars"], 0);

    let environment_path = archive_dir.join("environment.json");
    let mut environment: Value =
        serde_json::from_slice(&fs::read(&environment_path).expect("read environment"))
            .expect("decode environment");
    environment["repository_revision"] = Value::String("tampered-revision".to_owned());
    fs::write(
        &environment_path,
        serde_json::to_vec_pretty(&environment).expect("encode tampered environment"),
    )
    .expect("write tampered environment");

    let rejected = run_verifier(&archive_dir);
    assert!(
        !rejected.status.success(),
        "tampered archive unexpectedly verified: stdout={} stderr={}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        stderr.contains("repository_revision differs between environment.json and index.json"),
        "unexpected verifier error: {stderr}"
    );
}

fn run_verifier(archive_dir: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_db-lab-batch-verify"))
        .arg("--archive-dir")
        .arg(archive_dir)
        .arg("--expected-revision")
        .arg("abc123")
        .output()
        .expect("run db-lab-batch-verify")
}

fn assert_success(name: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{name} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
