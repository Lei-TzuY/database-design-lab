use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-log-generation-publish",
    version,
    about = "Durably publish an append-log generation commit marker on supported hosts"
)]
struct Cli {
    /// Existing generation directory.
    #[arg(long)]
    directory: PathBuf,

    /// Generation id whose clean append-log image should become committed.
    #[arg(long)]
    generation: u64,
}

fn main() -> ExitCode {
    let args = Cli::parse();

    #[cfg(unix)]
    {
        match unix::publish_marker(&args) {
            Ok(summary) => match serde_json::to_string_pretty(&summary) {
                Ok(encoded) => {
                    println!("{encoded}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("error: failed to encode publication summary: {error}");
                    ExitCode::from(1)
                }
            },
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(1)
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = args;
        eprintln!(
            "error: durable append-log generation marker publication is unsupported on this platform; no marker was written"
        );
        ExitCode::from(1)
    }
}

#[cfg(unix)]
mod unix {
    use std::collections::BTreeSet;
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::path::{Path, PathBuf};

    use db_cli::generation_marker::{
        decode_commit_marker, encode_commit_marker, CommitMarker, CommittedPrefix, Crc32Ieee,
        COMMIT_MARKER_LEN, COMMIT_MARKER_VERSION,
    };
    use db_cli::generation_prefix::{verify_committed_prefix, CommittedPrefixVerifyError};
    use db_core::DbError;
    use db_storage_log::{LogEngine, VerificationReport};
    use serde::Serialize;
    use thiserror::Error;

    use super::Cli;

    const PUBLICATION_PROTOCOL: &str = "append_log_generation_marker_publication_unix_v1";
    const GENERATION_PREFIX: &str = "generation-";
    const GENERATION_SUFFIX: &str = ".log";
    const COMMIT_PREFIX: &str = "commit-";
    const COMMIT_SUFFIX: &str = ".marker";
    const STAGING_COMMIT_PREFIX: &str = "staging-commit-";
    const GENERATION_ID_WIDTH: usize = 20;
    const MAX_DIRECTORY_ENTRIES: usize = 8_192;
    const CRC_BUFFER_BYTES: usize = 64 * 1024;

    #[derive(Debug, Serialize)]
    pub struct PublicationSummary {
        protocol: &'static str,
        marker_format_version: u16,
        generation: u64,
        generation_log: String,
        marker: String,
        committed_prefix: CommittedPrefix,
        staging_retained: bool,
    }

    #[derive(Debug, Error)]
    pub enum PublishError {
        #[error("invalid generation marker publication: {0}")]
        Invalid(String),
        #[error(transparent)]
        Database(#[from] DbError),
        #[error(transparent)]
        Prefix(#[from] CommittedPrefixVerifyError),
        #[error("I/O error at {path}: {source}")]
        Io {
            path: PathBuf,
            #[source]
            source: io::Error,
        },
        #[error(
            "commit marker {marker} is visible but parent-directory durability could not be confirmed: {source}; preserve the old generation and treat recovery as authoritative before retrying"
        )]
        DurabilityUncertain {
            marker: PathBuf,
            #[source]
            source: io::Error,
        },
    }

    pub fn publish_marker(args: &Cli) -> Result<PublicationSummary, PublishError> {
        if args.generation == 0 {
            return invalid("generation id must be greater than zero");
        }

        let directory = canonical_real_directory(&args.directory)?;
        let existing_markers = scan_namespace(&directory)?;
        if existing_markers.iter().any(|id| *id >= args.generation) {
            return invalid(format!(
                "generation {} is not newer than every existing committed generation",
                args.generation
            ));
        }

        let log_path = generation_path(&directory, args.generation);
        let marker_path = marker_path(&directory, args.generation);
        let staging_path = staging_marker_path(&directory, args.generation);
        require_real_regular_file(&log_path, "generation log")?;
        require_absent(&marker_path, "commit marker")?;
        remove_stale_staging_if_safe(&staging_path)?;

        let baseline = require_clean_generation(&log_path)?;
        let committed_prefix = derive_prefix_proof(&log_path, &baseline)?;
        let prefix_verification = verify_committed_prefix(&log_path, committed_prefix)?;
        if prefix_verification != baseline {
            return invalid(
                "derived committed-prefix verification disagrees with clean generation",
            );
        }

        // The generation bytes and their directory entry must precede commit-marker authority.
        sync_regular_file(&log_path)?;
        sync_directory(&directory).map_err(|source| io_error(&directory, source))?;
        require_exact_clean_generation(&log_path, &baseline, committed_prefix)?;

        let encoded = encode_commit_marker(args.generation, committed_prefix).map_err(|error| {
            PublishError::Invalid(format!("cannot encode commit marker: {error}"))
        })?;
        write_synced_staging(&staging_path, &encoded)?;

        // Re-check after staging I/O so a caller-serialization violation cannot publish stale proof.
        require_exact_clean_generation(&log_path, &baseline, committed_prefix)?;
        fs::hard_link(&staging_path, &marker_path)
            .map_err(|source| io_error(&marker_path, source))?;

        if let Err(source) = sync_directory(&directory) {
            return Err(PublishError::DurabilityUncertain {
                marker: marker_path,
                source,
            });
        }

        verify_published_marker(&marker_path, args.generation, committed_prefix)?;
        let _ = verify_committed_prefix(&log_path, committed_prefix)?;

        let staging_retained = match fs::remove_file(&staging_path) {
            Ok(()) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(_) => true,
        };

        Ok(PublicationSummary {
            protocol: PUBLICATION_PROTOCOL,
            marker_format_version: COMMIT_MARKER_VERSION,
            generation: args.generation,
            generation_log: canonical_generation_name(args.generation),
            marker: canonical_marker_name(args.generation),
            committed_prefix,
            staging_retained,
        })
    }

    fn derive_prefix_proof(
        path: &Path,
        report: &VerificationReport,
    ) -> Result<CommittedPrefix, PublishError> {
        let mut file = File::open(path).map_err(|source| io_error(path, source))?;
        let mut remaining = report.file_bytes;
        let mut hasher = Crc32Ieee::new();
        let mut buffer = [0_u8; CRC_BUFFER_BYTES];

        while remaining > 0 {
            let wanted = usize::try_from(remaining.min(CRC_BUFFER_BYTES as u64))
                .expect("CRC chunk is bounded by a usize-sized constant");
            let read = file
                .read(&mut buffer[..wanted])
                .map_err(|source| io_error(path, source))?;
            if read == 0 {
                return invalid(format!(
                    "generation reached EOF with {remaining} proof bytes still expected"
                ));
            }
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }

        let mut extra = [0_u8; 1];
        let trailing = file
            .read(&mut extra)
            .map_err(|source| io_error(path, source))?;
        if trailing != 0 {
            return invalid("generation changed while its committed-prefix checksum was derived");
        }

        Ok(CommittedPrefix {
            bytes: report.file_bytes,
            crc32: hasher.finalize(),
            record_count: report.record_count,
            next_sequence: report.next_sequence,
        })
    }

    fn require_clean_generation(path: &Path) -> Result<VerificationReport, PublishError> {
        let report = LogEngine::verify(path)?;
        if report.recoverable_tail.is_some() || report.file_bytes != report.valid_bytes {
            return invalid(
                "generation must be a complete clean append-log image before marker publication",
            );
        }
        Ok(report)
    }

    fn require_exact_clean_generation(
        path: &Path,
        baseline: &VerificationReport,
        proof: CommittedPrefix,
    ) -> Result<(), PublishError> {
        let _ = verify_committed_prefix(path, proof)?;
        let current = require_clean_generation(path)?;
        if &current != baseline {
            return invalid("generation changed while commit marker publication was in progress");
        }
        Ok(())
    }

    fn write_synced_staging(
        path: &Path,
        encoded: &[u8; COMMIT_MARKER_LEN],
    ) -> Result<(), PublishError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|source| io_error(path, source))?;
        file.write_all(encoded)
            .map_err(|source| io_error(path, source))?;
        file.sync_all().map_err(|source| io_error(path, source))
    }

    fn sync_regular_file(path: &Path) -> Result<(), PublishError> {
        let file = File::open(path).map_err(|source| io_error(path, source))?;
        file.sync_all().map_err(|source| io_error(path, source))
    }

    fn sync_directory(path: &Path) -> io::Result<()> {
        File::open(path)?.sync_all()
    }

    fn verify_published_marker(
        path: &Path,
        generation: u64,
        committed_prefix: CommittedPrefix,
    ) -> Result<CommitMarker, PublishError> {
        require_real_regular_file(path, "published commit marker")?;
        let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
        let marker = decode_commit_marker(&bytes, generation)
            .map_err(|error| PublishError::Invalid(format!("published commit marker: {error}")))?;
        if marker.committed_prefix != committed_prefix {
            return invalid("published commit marker prefix differs from staged proof");
        }
        Ok(marker)
    }

    fn scan_namespace(directory: &Path) -> Result<BTreeSet<u64>, PublishError> {
        let mut committed = BTreeSet::new();
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
                PublishError::Invalid(
                    "generation directory contains a non-UTF-8 entry name".to_owned(),
                )
            })?;

            if parse_canonical_id(name, GENERATION_PREFIX, GENERATION_SUFFIX, "generation log")?
                .is_some()
            {
                continue;
            }
            if let Some(id) =
                parse_canonical_id(name, COMMIT_PREFIX, COMMIT_SUFFIX, "commit marker")?
            {
                let _ = committed.insert(id);
                continue;
            }
            if parse_canonical_id(
                name,
                STAGING_COMMIT_PREFIX,
                COMMIT_SUFFIX,
                "staging commit marker",
            )?
            .is_some()
            {
                continue;
            }
            return invalid(format!("unexpected generation directory entry {name:?}"));
        }
        Ok(committed)
    }

    fn parse_canonical_id(
        name: &str,
        prefix: &str,
        suffix: &str,
        kind: &str,
    ) -> Result<Option<u64>, PublishError> {
        if !name.starts_with(prefix) {
            return Ok(None);
        }
        let expected_len = prefix
            .len()
            .checked_add(GENERATION_ID_WIDTH)
            .and_then(|len| len.checked_add(suffix.len()))
            .ok_or_else(|| {
                PublishError::Invalid("canonical name length overflowed usize".to_owned())
            })?;
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
            .map_err(|_| PublishError::Invalid(format!("{kind} generation id does not fit u64")))?;
        if id == 0 || format!("{id:020}") != digits {
            return invalid(format!("non-canonical {kind} generation id in {name:?}"));
        }
        Ok(Some(id))
    }

    fn canonical_real_directory(path: &Path) -> Result<PathBuf, PublishError> {
        let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
        if !metadata.file_type().is_dir() {
            return invalid(format!(
                "generation directory must be a real directory rather than a symlink or non-directory: {}",
                path.display()
            ));
        }
        fs::canonicalize(path).map_err(|source| io_error(path, source))
    }

    fn require_real_regular_file(path: &Path, label: &str) -> Result<(), PublishError> {
        let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
        if !metadata.file_type().is_file() {
            return invalid(format!(
                "{label} must be a real regular file rather than a symlink or non-file: {}",
                path.display()
            ));
        }
        Ok(())
    }

    fn require_absent(path: &Path, label: &str) -> Result<(), PublishError> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => invalid(format!("{label} already exists: {}", path.display())),
            Err(source) => Err(io_error(path, source)),
        }
    }

    fn remove_stale_staging_if_safe(path: &Path) -> Result<(), PublishError> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::remove_file(path).map_err(|source| io_error(path, source))
            }
            Ok(_) => invalid(format!(
                "staging marker exists but is not a real regular file: {}",
                path.display()
            )),
            Err(source) => Err(io_error(path, source)),
        }
    }

    fn generation_path(directory: &Path, id: u64) -> PathBuf {
        directory.join(canonical_generation_name(id))
    }

    fn marker_path(directory: &Path, id: u64) -> PathBuf {
        directory.join(canonical_marker_name(id))
    }

    fn staging_marker_path(directory: &Path, id: u64) -> PathBuf {
        directory.join(format!("{STAGING_COMMIT_PREFIX}{id:020}{COMMIT_SUFFIX}"))
    }

    fn canonical_generation_name(id: u64) -> String {
        format!("{GENERATION_PREFIX}{id:020}{GENERATION_SUFFIX}")
    }

    fn canonical_marker_name(id: u64) -> String {
        format!("{COMMIT_PREFIX}{id:020}{COMMIT_SUFFIX}")
    }

    fn io_error(path: &Path, source: io::Error) -> PublishError {
        PublishError::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    fn invalid<T>(message: impl Into<String>) -> Result<T, PublishError> {
        Err(PublishError::Invalid(message.into()))
    }
}
