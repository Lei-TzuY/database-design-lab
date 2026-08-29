use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};
use db_core::{
    run_counterbalanced_experiment_batch, CounterbalancedExperimentBatchReport, DbError,
    ExperimentAttemptAdmission, ExperimentInstanceContext, ExperimentTrace,
    MAX_EXPERIMENT_BATCH_PAIRS,
};
use db_storage_btree::{BPlusTree, BtreeError};
use db_storage_lsm::LsmEngine;
use serde::Serialize;
use thiserror::Error;

const MAX_JSON_INPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXCLUSION_REASON_BYTES: usize = 4 * 1024;
const BATCH_ARCHIVE_FORMAT_VERSION: u16 = 6;
const BATCH_EXECUTION_PROTOCOL: &str = "fresh_counterbalanced_repeated_batch_v1";
const BATCH_ATTEMPT_PROTOCOL: &str = "retain_all_requested_pairs_v1";
const ENGINE_LAYOUT: &str = "pair-{pair_index:06}/repetition-{repetition_index}/{btree.db|lsm}";

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-batch",
    version,
    about = "Archive every included, failed, and excluded Phase 4 counterbalanced pair"
)]
struct Cli {
    /// Versioned experiment trace JSON file.
    #[arg(long)]
    trace: PathBuf,
    /// New root directory used for all fresh B+ tree and LSM instances.
    #[arg(long)]
    engine_root: PathBuf,
    /// New immutable evidence archive directory.
    #[arg(long)]
    archive_dir: PathBuf,
    /// Seed whose low bit chooses the first outer pair order; later pairs alternate.
    #[arg(long)]
    pair_seed: u64,
    /// Number of fresh counterbalanced pairs to request.
    #[arg(long)]
    pairs: u32,
    /// B+ tree validated-page cache capacity for every fresh instance.
    #[arg(long, default_value_t = 64)]
    btree_cache_pages: usize,
    /// Exact source revision represented by the binary/run, normally a full Git commit SHA.
    #[arg(long)]
    revision: String,
    /// Exclude one zero-based pair before engine creation as `INDEX=REASON`; may be repeated.
    #[arg(long = "exclude-pair")]
    exclude_pairs: Vec<String>,
    /// Human-readable host identity without secrets.
    #[arg(long)]
    host_label: Option<String>,
    /// Filesystem under test, when known.
    #[arg(long)]
    filesystem: Option<String>,
    /// Storage device/model label, when known. Do not include serial numbers or credentials.
    #[arg(long)]
    storage_device: Option<String>,
    /// Declared cache state for this exploratory batch.
    #[arg(long, value_enum, default_value_t = CacheStateKind::Unspecified)]
    cache_state: CacheStateKind,
    /// Optional free-form experiment note. Do not include secrets.
    #[arg(long)]
    notes: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CacheStateKind {
    Unspecified,
    Warm,
    ColdBestEffort,
}

impl CacheStateKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Warm => "warm",
            Self::ColdBestEffort => "cold_best_effort",
        }
    }
}

#[derive(Debug, Serialize)]
struct BatchArchiveEnvironment {
    format_version: u16,
    repository_revision: String,
    execution_protocol: &'static str,
    attempt_protocol: &'static str,
    pair_seed: u64,
    requested_pairs: u32,
    engine_layout: &'static str,
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
struct BatchArchiveIndex {
    format_version: u16,
    repository_revision: String,
    execution_protocol: &'static str,
    attempt_protocol: &'static str,
    files: [&'static str; 3],
}

#[derive(Debug, Error)]
enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("batch archive retained {failed_pairs} failed pair(s); evidence was written to {archive_dir}")]
    BatchFailures {
        failed_pairs: u32,
        archive_dir: String,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: Cli) -> Result<(), CliError> {
    validate_revision(&args.revision)?;
    if args.pairs == 0 || args.pairs > MAX_EXPERIMENT_BATCH_PAIRS {
        return Err(CliError::Usage(format!(
            "--pairs is {}; expected 1..={MAX_EXPERIMENT_BATCH_PAIRS}",
            args.pairs
        )));
    }
    let trace = read_experiment_trace(&args.trace)?;
    let exclusions = parse_exclusions(&args.exclude_pairs, args.pairs)?;
    ensure_fresh_targets(&args.engine_root, &args.archive_dir)?;

    fs::create_dir_all(&args.engine_root)?;
    let engine_root = args.engine_root.clone();
    let btree_cache_pages = args.btree_cache_pages;
    let report = run_counterbalanced_experiment_batch(
        &trace,
        args.pair_seed,
        args.pairs,
        |context| {
            let path = instance_path(&engine_root, context, "btree.db");
            prepare_instance_parent(&path)?;
            BPlusTree::create_new(path, btree_cache_pages).map_err(btree_error_into_db_error)
        },
        |context| {
            let path = instance_path(&engine_root, context, "lsm");
            prepare_instance_parent(&path)?;
            LsmEngine::create_new(path)
        },
        |context| match exclusions.get(&context.pair_index) {
            Some(reason) => ExperimentAttemptAdmission::Exclude {
                reason: reason.clone(),
            },
            None => ExperimentAttemptAdmission::Include,
        },
    )?;

    let environment = build_environment(&args)?;
    write_batch_archive(
        &args.archive_dir,
        &args.revision,
        &trace,
        &report,
        &environment,
    )?;

    if report.failed_pairs > 0 {
        return Err(CliError::BatchFailures {
            failed_pairs: report.failed_pairs,
            archive_dir: args.archive_dir.display().to_string(),
        });
    }
    Ok(())
}

fn read_experiment_trace(path: &Path) -> Result<ExperimentTrace, CliError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_JSON_INPUT_BYTES {
        return Err(CliError::Usage(format!(
            "experiment trace file has {} bytes; maximum is {MAX_JSON_INPUT_BYTES}",
            metadata.len()
        )));
    }
    let encoded = fs::read(path)?;
    let trace: ExperimentTrace = serde_json::from_slice(&encoded)?;
    trace.validate()?;
    Ok(trace)
}

fn parse_exclusions(values: &[String], pairs: u32) -> Result<BTreeMap<u32, String>, CliError> {
    let mut exclusions = BTreeMap::new();
    for encoded in values {
        let (index, reason) = encoded.split_once('=').ok_or_else(|| {
            CliError::Usage(format!(
                "--exclude-pair must use INDEX=REASON; got {encoded:?}"
            ))
        })?;
        let index = index.trim().parse::<u32>().map_err(|_| {
            CliError::Usage(format!(
                "--exclude-pair index must be a zero-based integer; got {index:?}"
            ))
        })?;
        if index >= pairs {
            return Err(CliError::Usage(format!(
                "--exclude-pair index {index} is outside requested pair range 0..{}",
                pairs - 1
            )));
        }
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > MAX_EXCLUSION_REASON_BYTES {
            return Err(CliError::Usage(format!(
                "--exclude-pair reason must contain 1..={MAX_EXCLUSION_REASON_BYTES} UTF-8 bytes after trimming"
            )));
        }
        if exclusions.insert(index, reason.to_owned()).is_some() {
            return Err(CliError::Usage(format!(
                "--exclude-pair index {index} was specified more than once"
            )));
        }
    }
    Ok(exclusions)
}

fn ensure_fresh_targets(engine_root: &Path, archive_dir: &Path) -> Result<(), CliError> {
    if engine_root == archive_dir
        || engine_root.starts_with(archive_dir)
        || archive_dir.starts_with(engine_root)
    {
        return Err(CliError::Usage(
            "--engine-root and --archive-dir must be distinct, non-nested paths".to_owned(),
        ));
    }
    for (label, path) in [("engine root", engine_root), ("archive", archive_dir)] {
        if path.exists() {
            return Err(CliError::Usage(format!(
                "batch experiment {label} path already exists: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn instance_path(root: &Path, context: ExperimentInstanceContext, leaf: &str) -> PathBuf {
    root.join(format!("pair-{:06}", context.attempt.pair_index))
        .join(format!("repetition-{}", context.repetition_index))
        .join(leaf)
}

fn prepare_instance_parent(path: &Path) -> Result<(), DbError> {
    let parent = path.parent().ok_or_else(|| {
        DbError::InvalidInput(format!(
            "engine instance path has no parent: {}",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(DbError::Io)
}

fn validate_revision(revision: &str) -> Result<(), CliError> {
    let revision = revision.trim();
    if revision.is_empty()
        || revision.len() > 128
        || !revision
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CliError::Usage(
            "--revision must be 1..=128 ASCII alphanumeric, '-', '_', or '.' characters".to_owned(),
        ));
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

fn build_environment(args: &Cli) -> Result<BatchArchiveEnvironment, CliError> {
    Ok(BatchArchiveEnvironment {
        format_version: BATCH_ARCHIVE_FORMAT_VERSION,
        repository_revision: args.revision.clone(),
        execution_protocol: BATCH_EXECUTION_PROTOCOL,
        attempt_protocol: BATCH_ATTEMPT_PROTOCOL,
        pair_seed: args.pair_seed,
        requested_pairs: args.pairs,
        engine_layout: ENGINE_LAYOUT,
        db_lab_version: env!("CARGO_PKG_VERSION"),
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        rustc_version: rustc_version(),
        host_label: args.host_label.clone(),
        filesystem: args.filesystem.clone(),
        storage_device: args.storage_device.clone(),
        cache_state: args.cache_state.as_str(),
        btree_cache_pages: args.btree_cache_pages,
        recorded_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| CliError::Usage(format!("system clock precedes Unix epoch: {error}")))?
            .as_secs(),
        notes: args.notes.clone(),
    })
}

fn rustc_version() -> Option<String> {
    let output = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn write_batch_archive(
    archive_dir: &Path,
    revision: &str,
    trace: &ExperimentTrace,
    report: &CounterbalancedExperimentBatchReport,
    environment: &BatchArchiveEnvironment,
) -> Result<(), CliError> {
    fs::create_dir(archive_dir)?;
    let result = (|| {
        write_new_json(&archive_dir.join("trace.json"), trace)?;
        write_new_json(&archive_dir.join("batch.json"), report)?;
        write_new_json(&archive_dir.join("environment.json"), environment)?;
        write_new_json(
            &archive_dir.join("index.json"),
            &BatchArchiveIndex {
                format_version: BATCH_ARCHIVE_FORMAT_VERSION,
                repository_revision: revision.to_owned(),
                execution_protocol: BATCH_EXECUTION_PROTOCOL,
                attempt_protocol: BATCH_ATTEMPT_PROTOCOL,
                files: ["trace.json", "batch.json", "environment.json"],
            },
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(archive_dir);
    }
    result
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), CliError> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use db_core::{generate_experiment_trace, ExperimentGeneratorConfig, ExperimentProfile};
    use serde_json::Value;
    use tempfile::tempdir;

    use super::{parse_exclusions, run, CacheStateKind, Cli};

    #[test]
    fn exclusion_specs_are_bounded_unique_and_in_range() {
        let values = vec!["1=scheduled cooldown".to_owned(), "3=host noise".to_owned()];
        let parsed = parse_exclusions(&values, 4).expect("parse exclusions");
        assert_eq!(
            parsed.get(&1).map(String::as_str),
            Some("scheduled cooldown")
        );
        assert_eq!(parsed.get(&3).map(String::as_str), Some("host noise"));

        let duplicate = vec!["1=first".to_owned(), "1=second".to_owned()];
        assert!(parse_exclusions(&duplicate, 2)
            .expect_err("duplicate index must fail")
            .to_string()
            .contains("more than once"));
        assert!(parse_exclusions(&["2=outside".to_owned()], 2)
            .expect_err("out-of-range index must fail")
            .to_string()
            .contains("outside requested pair range"));
    }

    #[test]
    fn real_batch_archive_retains_included_and_excluded_pairs() {
        let directory = tempdir().expect("temporary directory");
        let trace_path = directory.path().join("trace.json");
        let trace = generate_experiment_trace(ExperimentGeneratorConfig {
            seed: 0x2026_0829,
            profile: ExperimentProfile::RandomWrite,
            operations: 2,
            key_space: 8,
            value_bytes: 4,
            range_limit: 1,
            reopen_every: None,
        })
        .expect("generate trace");
        fs::write(
            &trace_path,
            serde_json::to_vec_pretty(&trace).expect("encode trace"),
        )
        .expect("write trace");

        let engine_root = directory.path().join("engines");
        let archive_dir = directory.path().join("archive");
        run(Cli {
            trace: trace_path,
            engine_root: engine_root.clone(),
            archive_dir: archive_dir.clone(),
            pair_seed: 0,
            pairs: 3,
            btree_cache_pages: 8,
            revision: "abc123".to_owned(),
            exclude_pairs: vec!["1=scheduled thermal cooldown".to_owned()],
            host_label: Some("test-host".to_owned()),
            filesystem: Some("test-fs".to_owned()),
            storage_device: Some("test-device".to_owned()),
            cache_state: CacheStateKind::Warm,
            notes: Some("integration test".to_owned()),
        })
        .expect("archive batch");

        let batch: Value =
            serde_json::from_slice(&fs::read(archive_dir.join("batch.json")).expect("read batch"))
                .expect("parse batch");
        assert_eq!(batch["requested_pairs"], 3);
        assert_eq!(batch["included_pairs"], 2);
        assert_eq!(batch["failed_pairs"], 0);
        assert_eq!(batch["excluded_pairs"], 1);
        assert_eq!(
            batch["attempts"][0]["context"]["pair_order"],
            "left_then_right_first"
        );
        assert_eq!(batch["attempts"][1]["disposition"], "excluded");
        assert_eq!(
            batch["attempts"][1]["exclusion_reason"],
            "scheduled thermal cooldown"
        );
        assert_eq!(
            batch["attempts"][2]["context"]["pair_order"],
            "left_then_right_first"
        );

        let environment: Value = serde_json::from_slice(
            &fs::read(archive_dir.join("environment.json")).expect("read environment"),
        )
        .expect("parse environment");
        assert_eq!(environment["format_version"], 6);
        assert_eq!(
            environment["execution_protocol"],
            "fresh_counterbalanced_repeated_batch_v1"
        );
        assert_eq!(
            environment["attempt_protocol"],
            "retain_all_requested_pairs_v1"
        );
        assert_eq!(environment["pair_seed"], 0);
        assert_eq!(environment["requested_pairs"], 3);
        assert_eq!(environment["cache_state"], "warm");

        let index: Value =
            serde_json::from_slice(&fs::read(archive_dir.join("index.json")).expect("read index"))
                .expect("parse index");
        assert_eq!(index["format_version"], 6);
        assert_eq!(
            index["files"],
            serde_json::json!(["trace.json", "batch.json", "environment.json"])
        );

        assert!(engine_root
            .join("pair-000000/repetition-0/btree.db")
            .exists());
        assert!(engine_root.join("pair-000000/repetition-0/lsm").exists());
        assert!(!engine_root.join("pair-000001").exists());
        assert!(engine_root.join("pair-000002/repetition-1/lsm").exists());
    }
}
