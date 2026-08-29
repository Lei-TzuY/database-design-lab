use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, ValueEnum};
use db_core::{
    compare_experiment_trace_counterbalanced, CounterbalancedExperimentComparisonReport,
    CounterbalancedPairOrder, DbError, ExperimentTrace,
};
use db_storage_btree::{BPlusTree, BtreeError};
use db_storage_lsm::LsmEngine;
use serde::Serialize;

use super::{
    read_experiment_trace, rustc_version, validate_revision, write_new_json, CacheStateKind,
    CliError,
};

const COUNTERBALANCED_EVIDENCE_ARCHIVE_FORMAT_VERSION: u16 = 2;
const COUNTERBALANCED_EXECUTION_PROTOCOL: &str = "fresh_counterbalanced_ab_ba";

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CounterbalancedPairOrderKind {
    LeftThenRightFirst,
    RightThenLeftFirst,
}

impl From<CounterbalancedPairOrderKind> for CounterbalancedPairOrder {
    fn from(value: CounterbalancedPairOrderKind) -> Self {
        match value {
            CounterbalancedPairOrderKind::LeftThenRightFirst => Self::LeftThenRightFirst,
            CounterbalancedPairOrderKind::RightThenLeftFirst => Self::RightThenLeftFirst,
        }
    }
}

/// Arguments for a fresh two-repetition AB/BA evidence archive.
#[derive(Debug, Args)]
pub(super) struct CounterbalancedArchiveArgs {
    /// Versioned experiment trace JSON file.
    #[arg(long)]
    trace: PathBuf,
    /// New B+ tree page file for the first repetition.
    #[arg(long)]
    first_btree_path: PathBuf,
    /// New LSM directory for the first repetition.
    #[arg(long)]
    first_lsm_path: PathBuf,
    /// New B+ tree page file for the second repetition.
    #[arg(long)]
    second_btree_path: PathBuf,
    /// New LSM directory for the second repetition.
    #[arg(long)]
    second_lsm_path: PathBuf,
    /// Which whole-run order executes first in the two-run pair.
    #[arg(
        long,
        value_enum,
        default_value_t = CounterbalancedPairOrderKind::LeftThenRightFirst
    )]
    pair_order: CounterbalancedPairOrderKind,
    /// B+ tree validated-page cache capacity used by both repetitions.
    #[arg(long, default_value_t = 64)]
    btree_cache_pages: usize,
    /// Exact source revision represented by the binary/run, normally a full Git commit SHA.
    #[arg(long)]
    revision: String,
    /// New archive directory; existing paths are never overwritten.
    #[arg(long)]
    archive_dir: PathBuf,
    /// Human-readable host identity without secrets (for example, `lab-5090-a`).
    #[arg(long)]
    host_label: Option<String>,
    /// Filesystem under test, when known (for example, `ntfs`, `ext4`, `apfs`).
    #[arg(long)]
    filesystem: Option<String>,
    /// Storage device/model label, when known. Do not place credentials or serial numbers here.
    #[arg(long)]
    storage_device: Option<String>,
    /// Declared cache preparation state for both repetitions.
    #[arg(long, value_enum, default_value_t = CacheStateKind::Unspecified)]
    cache_state: CacheStateKind,
    /// Optional free-form experiment note. Do not include secrets.
    #[arg(long)]
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
struct CounterbalancedEvidenceArchiveEnvironment {
    format_version: u16,
    repository_revision: String,
    execution_protocol: &'static str,
    pair_order: CounterbalancedPairOrder,
    db_lab_version: &'static str,
    target_os: &'static str,
    target_arch: &'static str,
    build_profile: &'static str,
    rustc_version: Option<String>,
    host_label: Option<String>,
    filesystem: Option<String>,
    storage_device: Option<String>,
    cache_state: &'static str,
    btree_cache_pages: usize,
    recorded_unix_seconds: u64,
    notes: Option<String>,
}

#[derive(Debug, Serialize)]
struct CounterbalancedEvidenceArchiveIndex {
    format_version: u16,
    repository_revision: String,
    execution_protocol: &'static str,
    files: [&'static str; 3],
}

pub(super) fn run(args: CounterbalancedArchiveArgs) -> Result<(), CliError> {
    validate_revision(&args.revision)?;
    let trace = read_experiment_trace(&args.trace)?;
    ensure_fresh_counterbalanced_archive_targets(
        &args.first_btree_path,
        &args.first_lsm_path,
        &args.second_btree_path,
        &args.second_lsm_path,
        &args.archive_dir,
    )?;

    let pair_order: CounterbalancedPairOrder = args.pair_order.into();
    let mut btree_paths = [args.first_btree_path, args.second_btree_path].into_iter();
    let mut lsm_paths = [args.first_lsm_path, args.second_lsm_path].into_iter();
    let btree_cache_pages = args.btree_cache_pages;
    let comparison = compare_experiment_trace_counterbalanced(
        &trace,
        pair_order,
        || {
            let path = btree_paths.next().ok_or_else(|| {
                DbError::InvalidInput(
                    "counterbalanced B+ tree factory requested more than two fresh instances"
                        .to_owned(),
                )
            })?;
            BPlusTree::create_new(path, btree_cache_pages).map_err(btree_error_into_db_error)
        },
        || {
            let path = lsm_paths.next().ok_or_else(|| {
                DbError::InvalidInput(
                    "counterbalanced LSM factory requested more than two fresh instances"
                        .to_owned(),
                )
            })?;
            LsmEngine::create_new(path)
        },
    )?;

    let environment = CounterbalancedEvidenceArchiveEnvironment {
        format_version: COUNTERBALANCED_EVIDENCE_ARCHIVE_FORMAT_VERSION,
        repository_revision: args.revision.clone(),
        execution_protocol: COUNTERBALANCED_EXECUTION_PROTOCOL,
        pair_order,
        db_lab_version: env!("CARGO_PKG_VERSION"),
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        rustc_version: rustc_version(),
        host_label: args.host_label,
        filesystem: args.filesystem,
        storage_device: args.storage_device,
        cache_state: args.cache_state.as_str(),
        btree_cache_pages,
        recorded_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| CliError::Usage(format!("system clock precedes Unix epoch: {error}")))?
            .as_secs(),
        notes: args.notes,
    };
    write_counterbalanced_evidence_archive(
        &args.archive_dir,
        &args.revision,
        &trace,
        &comparison,
        &environment,
    )
}

fn ensure_fresh_counterbalanced_archive_targets(
    first_btree_path: &Path,
    first_lsm_path: &Path,
    second_btree_path: &Path,
    second_lsm_path: &Path,
    archive_dir: &Path,
) -> Result<(), CliError> {
    let targets = [
        ("first B+ tree", first_btree_path),
        ("first LSM", first_lsm_path),
        ("second B+ tree", second_btree_path),
        ("second LSM", second_lsm_path),
        ("archive", archive_dir),
    ];
    for left in 0..targets.len() {
        for right in (left + 1)..targets.len() {
            if targets[left].1 == targets[right].1 {
                return Err(CliError::Usage(format!(
                    "counterbalanced archive targets must be distinct: {} and {} both use {}",
                    targets[left].0,
                    targets[right].0,
                    targets[left].1.display()
                )));
            }
        }
    }
    for (label, path) in targets {
        if path.exists() {
            return Err(CliError::Usage(format!(
                "counterbalanced experiment {label} path already exists: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn btree_error_into_db_error(error: BtreeError) -> DbError {
    match error {
        BtreeError::InvalidInput(message) => DbError::InvalidInput(message),
        BtreeError::Io(error) => DbError::Io(error),
        BtreeError::Corruption { offset, reason } => DbError::Corruption { offset, reason },
        BtreeError::UnsupportedVersion { found, supported } => DbError::UnsupportedVersion {
            format: "B+ tree",
            found,
            supported,
        },
        BtreeError::Poisoned => DbError::Poisoned,
    }
}

fn write_counterbalanced_evidence_archive(
    archive_dir: &Path,
    revision: &str,
    trace: &ExperimentTrace,
    comparison: &CounterbalancedExperimentComparisonReport,
    environment: &CounterbalancedEvidenceArchiveEnvironment,
) -> Result<(), CliError> {
    fs::create_dir(archive_dir)?;
    let result = (|| {
        write_new_json(&archive_dir.join("trace.json"), trace)?;
        write_new_json(&archive_dir.join("counterbalanced.json"), comparison)?;
        write_new_json(&archive_dir.join("environment.json"), environment)?;
        write_new_json(
            &archive_dir.join("index.json"),
            &CounterbalancedEvidenceArchiveIndex {
                format_version: COUNTERBALANCED_EVIDENCE_ARCHIVE_FORMAT_VERSION,
                repository_revision: revision.to_owned(),
                execution_protocol: COUNTERBALANCED_EXECUTION_PROTOCOL,
                files: ["trace.json", "counterbalanced.json", "environment.json"],
            },
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(archive_dir);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::fs;

    use db_core::{generate_experiment_trace, ExperimentGeneratorConfig, ExperimentProfile};
    use serde_json::Value;
    use tempfile::tempdir;

    use super::{
        ensure_fresh_counterbalanced_archive_targets, run, CounterbalancedArchiveArgs,
        CounterbalancedPairOrderKind,
    };
    use crate::CacheStateKind;

    #[test]
    fn duplicate_counterbalanced_targets_fail_before_creation() {
        let directory = tempdir().expect("temporary directory");
        let duplicate = directory.path().join("same");
        let error = ensure_fresh_counterbalanced_archive_targets(
            &duplicate,
            &duplicate,
            &directory.path().join("btree-b.db"),
            &directory.path().join("lsm-b"),
            &directory.path().join("archive"),
        )
        .expect_err("duplicate paths must fail");
        assert!(error.to_string().contains("must be distinct"));
    }

    #[test]
    fn real_counterbalanced_archive_records_pair_provenance() {
        let directory = tempdir().expect("temporary directory");
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 0x2026_0829,
            profile: ExperimentProfile::RandomWrite,
            operations: 4,
            key_space: 8,
            value_bytes: 8,
            range_limit: 1,
            reopen_every: None,
        })
        .expect("generate trace");
        let trace_path = directory.path().join("trace.json");
        fs::write(
            &trace_path,
            serde_json::to_vec_pretty(&trace).expect("serialize trace"),
        )
        .expect("write trace");
        let archive_dir = directory.path().join("archive");

        run(CounterbalancedArchiveArgs {
            trace: trace_path,
            first_btree_path: directory.path().join("btree-a.db"),
            first_lsm_path: directory.path().join("lsm-a"),
            second_btree_path: directory.path().join("btree-b.db"),
            second_lsm_path: directory.path().join("lsm-b"),
            pair_order: CounterbalancedPairOrderKind::RightThenLeftFirst,
            btree_cache_pages: 8,
            revision: "test-revision".to_owned(),
            archive_dir: archive_dir.clone(),
            host_label: Some("test-host".to_owned()),
            filesystem: None,
            storage_device: None,
            cache_state: CacheStateKind::Warm,
            notes: None,
        })
        .expect("write counterbalanced archive");

        let comparison: Value = serde_json::from_slice(
            &fs::read(archive_dir.join("counterbalanced.json")).expect("read comparison"),
        )
        .expect("parse comparison");
        assert_eq!(comparison["pair_order"], "right_then_left_first");
        assert_eq!(comparison["first"]["execution_order"], "right_then_left");
        assert_eq!(comparison["second"]["execution_order"], "left_then_right");

        let environment: Value = serde_json::from_slice(
            &fs::read(archive_dir.join("environment.json")).expect("read environment"),
        )
        .expect("parse environment");
        assert_eq!(environment["format_version"], 2);
        assert_eq!(
            environment["execution_protocol"],
            "fresh_counterbalanced_ab_ba"
        );
        assert_eq!(environment["pair_order"], "right_then_left_first");
        assert_eq!(environment["cache_state"], "warm");

        let index: Value =
            serde_json::from_slice(&fs::read(archive_dir.join("index.json")).expect("read index"))
                .expect("parse index");
        assert_eq!(index["format_version"], 2);
        assert_eq!(index["execution_protocol"], "fresh_counterbalanced_ab_ba");
        assert_eq!(
            index["files"],
            serde_json::json!(["trace.json", "counterbalanced.json", "environment.json"])
        );
    }
}
