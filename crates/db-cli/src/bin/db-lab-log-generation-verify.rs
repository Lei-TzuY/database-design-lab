use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use db_cli::generation_marker::{
    decode_commit_marker, CommitMarker, CommittedPrefix, APPEND_LOG_FORMAT_VERSION,
    COMMIT_MARKER_LEN, COMMIT_MARKER_VERSION,
};
use db_cli::generation_prefix::verify_committed_prefix;
use db_core::DbError;
use db_storage_log::{LogEngine, VerificationReport};
use serde::Serialize;
use thiserror::Error;

const GENERATION_DIRECTORY_PROTOCOL: &str = "append_log_generation_directory_v2";
const GENERATION_PREFIX: &str = "generation-";
const GENERATION_SUFFIX: &str = ".log";
const COMMIT_PREFIX: &str = "commit-";
const COMMIT_SUFFIX: &str = ".marker";
const GENERATION_ID_WIDTH: usize = 20;
const MAX_DIRECTORY_ENTRIES: usize = 8_192;

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-log-generation-verify",
    version,
    about = "Read-only verification of an append-log generation directory"
)]
struct Cli {
    /// Generation directory to inspect without modifying it.
    #[arg(long)]
    directory: PathBuf,
}

#[derive(Debug, Serialize)]
struct GenerationVerificationSummary {
    protocol: &'static str,
    marker_format_version: u16,
    authoritative_generation: u64,
    authoritative_log: String,
    highest_observed_generation: u64,
    marker_generation_ids: Vec<u64>,
    uncommitted_generation_ids: Vec<u64>,
    committed_prefix: CommittedPrefix,
    committed_prefix_verification: VerificationReport,
    log_verification: VerificationReport,
}

struct GenerationNamespace {
    generation_files: BTreeMap<u64, PathBuf>,
    marker_files: BTreeMap<u64, PathBuf>,
}

#[derive(Debug, Error)]
enum VerifyError {
    #[error("invalid generation directory: {0}")]
    Invalid(String),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn main() -> ExitCode {
    match verify_generation_directory(&Cli::parse().directory) {
        Ok(summary) => match serde_json::to_string_pretty(&summary) {
            Ok(encoded) => {
                println!("{encoded}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: failed to encode verification summary: {error}");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn verify_generation_directory(
    directory: &Path,
) -> Result<GenerationVerificationSummary, VerifyError> {
    let directory = canonical_real_directory(directory)?;
    let namespace = scan_namespace(&directory)?;

    let Some((&authoritative_generation, marker_path)) = namespace.marker_files.last_key_value()
    else {
        return invalid("no committed generation marker exists");
    };
    let marker = read_commit_marker(marker_path, authoritative_generation)?;
    if marker.log_format_version != APPEND_LOG_FORMAT_VERSION {
        return invalid(format!(
            "highest commit marker requires append-log format {}, expected {APPEND_LOG_FORMAT_VERSION}",
            marker.log_format_version
        ));
    }

    let log_path = namespace
        .generation_files
        .get(&authoritative_generation)
        .ok_or_else(|| {
            VerifyError::Invalid(format!(
                "highest committed generation {authoritative_generation} has no generation log"
            ))
        })?;
    require_real_regular_file(log_path, "authoritative generation log")?;
    let log_verification = LogEngine::verify(log_path)?;
    if log_verification.file_format_version != marker.log_format_version {
        return invalid(format!(
            "authoritative log format {} disagrees with marker format {}",
            log_verification.file_format_version, marker.log_format_version
        ));
    }

    let committed_prefix_verification =
        verify_committed_prefix(log_path, marker.committed_prefix).map_err(|error| {
            VerifyError::Invalid(format!("highest commit marker prefix proof failed: {error}"))
        })?;

    if log_verification.valid_bytes < marker.committed_prefix.bytes {
        return invalid(format!(
            "authoritative log has only {} structurally valid bytes before its tail, but marker binds {} committed prefix bytes",
            log_verification.valid_bytes, marker.committed_prefix.bytes
        ));
    }
    if let Some(tail) = &log_verification.recoverable_tail {
        if tail.record_offset < marker.committed_prefix.bytes {
            return invalid(format!(
                "authoritative recoverable tail starts at byte {}, inside marker-bound committed prefix ending at byte {}",
                tail.record_offset, marker.committed_prefix.bytes
            ));
        }
    }

    let highest_observed_generation = namespace
        .generation_files
        .keys()
        .chain(namespace.marker_files.keys())
        .copied()
        .max()
        .ok_or_else(|| VerifyError::Invalid("generation directory is empty".to_owned()))?;
    let marker_generation_ids = namespace.marker_files.keys().copied().collect();
    let uncommitted_generation_ids = namespace
        .generation_files
        .keys()
        .filter(|id| !namespace.marker_files.contains_key(*id))
        .copied()
        .collect();
    let authoritative_log = log_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            VerifyError::Invalid("authoritative generation filename is not UTF-8".to_owned())
        })?
        .to_owned();

    Ok(GenerationVerificationSummary {
        protocol: GENERATION_DIRECTORY_PROTOCOL,
        marker_format_version: COMMIT_MARKER_VERSION,
        authoritative_generation,
        authoritative_log,
        highest_observed_generation,
        marker_generation_ids,
        uncommitted_generation_ids,
        committed_prefix: marker.committed_prefix,
        committed_prefix_verification,
        log_verification,
    })
}

fn scan_namespace(directory: &Path) -> Result<GenerationNamespace, VerifyError> {
    let mut generation_files = BTreeMap::new();
    let mut marker_files = BTreeMap::new();
    let entries = fs::read_dir(directory).map_err(|source| io_error(directory, source))?;

    for (index, entry) in entries.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            return invalid(format!(
                "generation directory contains more than {MAX_DIRECTORY_ENTRIES} entries"
            ));
        }
        let entry = entry.map_err(|source| io_error(directory, source))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            VerifyError::Invalid("generation directory contains a non-UTF-8 entry name".to_owned())
        })?;
        let path = entry.path();

        if let Some(id) = parse_canonical_generation_name(name)? {
            let _ = generation_files.insert(id, path);
            continue;
        }
        if let Some(id) = parse_canonical_commit_name(name)? {
            let _ = marker_files.insert(id, path);
            continue;
        }
        return invalid(format!("unexpected generation directory entry {name:?}"));
    }

    Ok(GenerationNamespace {
        generation_files,
        marker_files,
    })
}

fn parse_canonical_generation_name(name: &str) -> Result<Option<u64>, VerifyError> {
    parse_canonical_id(name, GENERATION_PREFIX, GENERATION_SUFFIX, "generation log")
}

fn parse_canonical_commit_name(name: &str) -> Result<Option<u64>, VerifyError> {
    parse_canonical_id(name, COMMIT_PREFIX, COMMIT_SUFFIX, "commit marker")
}

fn parse_canonical_id(
    name: &str,
    prefix: &str,
    suffix: &str,
    kind: &str,
) -> Result<Option<u64>, VerifyError> {
    if !name.starts_with(prefix) {
        return Ok(None);
    }
    let expected_len = prefix
        .len()
        .checked_add(GENERATION_ID_WIDTH)
        .and_then(|len| len.checked_add(suffix.len()))
        .ok_or_else(|| VerifyError::Invalid("canonical name length overflowed usize".to_owned()))?;
    if name.len() != expected_len || !name.ends_with(suffix) {
        return invalid(format!("malformed canonical {kind} name {name:?}"));
    }
    let digits = &name[prefix.len()..prefix.len() + GENERATION_ID_WIDTH];
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return invalid(format!(
            "malformed canonical {kind} generation id in {name:?}"
        ));
    }
    let id = digits
        .parse::<u64>()
        .map_err(|_| VerifyError::Invalid(format!("{kind} generation id does not fit u64")))?;
    if id == 0 || format!("{id:020}") != digits {
        return invalid(format!("non-canonical {kind} generation id in {name:?}"));
    }
    Ok(Some(id))
}

fn read_commit_marker(path: &Path, filename_generation: u64) -> Result<CommitMarker, VerifyError> {
    require_real_regular_file(path, "highest commit marker")?;
    let metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
    if metadata.len() != COMMIT_MARKER_LEN as u64 {
        return invalid(format!(
            "highest commit marker has {} bytes, expected {COMMIT_MARKER_LEN}",
            metadata.len()
        ));
    }

    let mut file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut bytes = [0_u8; COMMIT_MARKER_LEN];
    file.read_exact(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    decode_commit_marker(&bytes, filename_generation)
        .map_err(|error| VerifyError::Invalid(format!("highest commit marker {error}")))
}

fn canonical_real_directory(path: &Path) -> Result<PathBuf, VerifyError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_dir() {
        return invalid(format!(
            "generation directory must be a real directory rather than a symlink or non-directory: {}",
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|source| io_error(path, source))
}

fn require_real_regular_file(path: &Path, label: &str) -> Result<(), VerifyError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return invalid(format!(
            "{label} must be a real regular file rather than a symlink or non-file: {}",
            path.display()
        ));
    }
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> VerifyError {
    VerifyError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, VerifyError> {
    Err(VerifyError::Invalid(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_names_are_strict() {
        assert_eq!(
            parse_canonical_generation_name("generation-00000000000000000042.log")
                .expect("parse generation"),
            Some(42)
        );
        assert_eq!(
            parse_canonical_commit_name("commit-00000000000000000042.marker")
                .expect("parse marker"),
            Some(42)
        );
        assert!(parse_canonical_generation_name("generation-42.log").is_err());
        assert!(parse_canonical_commit_name("commit-00000000000000000000.marker").is_err());
    }
}
