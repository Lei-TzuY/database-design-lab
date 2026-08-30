use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use db_core::{DbError, KvEngine};
use db_storage_log::{InspectionReport, LogEngine, VerificationReport};
use serde::Serialize;
use thiserror::Error;

const COMPACTION_PROTOCOL: &str = "append_log_compact_copy_v1";

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-log-compact",
    version,
    about = "Publish a non-destructive compact copy of a clean append-log file"
)]
struct Cli {
    /// Existing clean append-log source. The source is opened read-only and is never repaired.
    #[arg(long)]
    source: PathBuf,
    /// Fresh compacted v1 append-log file. Existing paths are never overwritten.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Serialize)]
struct CompactionReport {
    protocol: &'static str,
    file_format_version: u16,
    source_file_bytes: u64,
    source_record_count: u64,
    live_keys: usize,
    compacted_file_bytes: u64,
    compacted_record_count: u64,
    reclaimed_bytes: u64,
    staging_retained: bool,
}

#[derive(Debug, Error)]
enum CompactError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid compaction request: {0}")]
    Invalid(String),
    #[error("failed to encode compaction report: {0}")]
    Json(#[from] serde_json::Error),
}

fn main() -> ExitCode {
    match compact(&Cli::parse()) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(encoded) => {
                println!("{encoded}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn compact(args: &Cli) -> Result<CompactionReport, CompactError> {
    let source = canonical_regular_file(&args.source, "source append log")?;
    let output = canonical_fresh_output(&args.output)?;
    if source == output {
        return invalid("source and output must be distinct paths");
    }
    let staging = staging_path(&output)?;
    if source == staging {
        return invalid("derived staging path aliases the source append log");
    }
    require_absent(&staging, "compaction staging path")?;

    let source_verification = LogEngine::verify(&source)?;
    if source_verification.recoverable_tail.is_some() {
        return invalid(
            "source has a recoverable incomplete final append; reopen/repair it explicitly before compaction",
        );
    }
    let source_inspection = LogEngine::inspect(&source, true)?;
    if source_inspection.verification != source_verification {
        return invalid("source changed between verification and replay inspection");
    }

    let staging_result = build_staging(&staging, &source_inspection);
    if let Err(error) = staging_result {
        let _ = fs::remove_file(&staging);
        return Err(error);
    }

    let compacted_inspection = LogEngine::inspect(&staging, true)?;
    validate_compacted_state(&source_inspection, &compacted_inspection)?;

    let source_after = LogEngine::inspect(&source, true)?;
    if source_after != source_inspection {
        let _ = fs::remove_file(&staging);
        return invalid("source changed while the compact copy was being constructed");
    }

    fs::hard_link(&staging, &output).map_err(|source| CompactError::Io {
        path: output.clone(),
        source,
    })?;

    let published = match LogEngine::inspect(&output, true) {
        Ok(report) if report == compacted_inspection => report,
        Ok(_) => {
            let _ = fs::remove_file(&output);
            let _ = fs::remove_file(&staging);
            return invalid("published compact output differs from its verified staging image");
        }
        Err(error) => {
            let _ = fs::remove_file(&output);
            let _ = fs::remove_file(&staging);
            return Err(error.into());
        }
    };

    let staging_retained = fs::remove_file(&staging).is_err();
    let reclaimed_bytes = source_verification
        .file_bytes
        .checked_sub(published.verification.file_bytes)
        .ok_or_else(|| {
            CompactError::Invalid(
                "compacted file is unexpectedly larger than its source append log".to_owned(),
            )
        })?;

    Ok(CompactionReport {
        protocol: COMPACTION_PROTOCOL,
        file_format_version: published.verification.file_format_version,
        source_file_bytes: source_verification.file_bytes,
        source_record_count: source_verification.record_count,
        live_keys: source_verification.live_keys,
        compacted_file_bytes: published.verification.file_bytes,
        compacted_record_count: published.verification.record_count,
        reclaimed_bytes,
        staging_retained,
    })
}

fn build_staging(path: &Path, source: &InspectionReport) -> Result<(), CompactError> {
    let result = (|| {
        let mut compacted = LogEngine::create_new(path)?;
        for entry in &source.entries {
            let value = entry.value.as_ref().ok_or_else(|| {
                CompactError::Invalid(
                    "internal compaction inspection omitted a requested live value".to_owned(),
                )
            })?;
            compacted.put(entry.key.as_slice(), value.as_slice())?;
        }
        Ok::<(), CompactError>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn validate_compacted_state(
    source: &InspectionReport,
    compacted: &InspectionReport,
) -> Result<(), CompactError> {
    if compacted.verification.recoverable_tail.is_some() {
        return invalid("compaction staging unexpectedly contains a recoverable tail");
    }
    if compacted.verification.live_keys != source.verification.live_keys
        || compacted.verification.record_count
            != u64::try_from(source.entries.len()).map_err(|_| {
                CompactError::Invalid("live-key count does not fit u64".to_owned())
            })?
        || compacted.entries != source.entries
    {
        return invalid("compaction staging does not exactly reproduce the source live state");
    }
    Ok(())
}

fn canonical_regular_file(path: &Path, label: &str) -> Result<PathBuf, CompactError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return invalid(format!(
            "{label} must be a real regular file rather than a symlink or non-file"
        ));
    }
    fs::canonicalize(path).map_err(|source| io_error(path, source))
}

fn canonical_fresh_output(path: &Path) -> Result<PathBuf, CompactError> {
    require_absent(path, "compaction output")?;
    let file_name = path.file_name().ok_or_else(|| {
        CompactError::Invalid(format!(
            "compaction output has no final path component: {}",
            path.display()
        ))
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let metadata = fs::symlink_metadata(parent).map_err(|source| io_error(parent, source))?;
    if !metadata.file_type().is_dir() {
        return invalid(format!(
            "compaction output parent must be a real directory: {}",
            parent.display()
        ));
    }
    let parent = fs::canonicalize(parent).map_err(|source| io_error(parent, source))?;
    Ok(parent.join(file_name))
}

fn staging_path(output: &Path) -> Result<PathBuf, CompactError> {
    let file_name = output.file_name().ok_or_else(|| {
        CompactError::Invalid(format!(
            "compaction output has no final path component: {}",
            output.display()
        ))
    })?;
    let mut staging_name = OsString::from(".");
    staging_name.push(file_name);
    staging_name.push(".compacting");
    Ok(output.with_file_name(staging_name))
}

fn require_absent(path: &Path, label: &str) -> Result<(), CompactError> {
    match fs::symlink_metadata(path) {
        Ok(_) => invalid(format!("{label} already exists: {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn io_error(path: &Path, source: io::Error) -> CompactError {
    CompactError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CompactError> {
    Err(CompactError::Invalid(message.into()))
}
