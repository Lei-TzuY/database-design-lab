use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use db_storage_log::{LogEngine, VerificationReport};

use crate::generation_directory::{
    canonical_generation_name, canonical_marker_name, canonical_real_directory,
    canonical_staging_marker_name, require_real_regular_file, scan_generation_namespace,
    GenerationDirectoryError,
};
use crate::generation_marker::{
    decode_commit_marker, encode_commit_marker, CommitMarker, CommittedPrefix, Crc32Ieee,
    COMMIT_MARKER_LEN, COMMIT_MARKER_VERSION,
};
use crate::generation_prefix::verify_committed_prefix;
use crate::generation_publication::{GenerationPublicationError, GenerationPublicationSummary};
use crate::log_compaction::WindowsDurableCompactOutput;
use crate::windows_durable::move_no_replace_write_through;

pub(crate) const GENERATION_MARKER_PUBLICATION_WINDOWS_PROTOCOL: &str =
    "append_log_generation_marker_publication_windows_v1";

const CRC_BUFFER_BYTES: usize = 64 * 1024;

/// Publishes marker-v2 authority for a Windows compact candidate whose canonical generation name was
/// created by the audited write-through compact-output path in this process. The opaque candidate
/// witness prevents this entry point from promoting an arbitrary hand-created generation file.
pub(crate) fn publish_windows_compact_generation_marker(
    directory: &Path,
    generation: u64,
    candidate: &WindowsDurableCompactOutput,
) -> Result<GenerationPublicationSummary, GenerationPublicationError> {
    if generation == 0 {
        return invalid("generation id must be greater than zero");
    }

    let directory = canonical_real_directory(directory).map_err(map_directory_error)?;
    let namespace = scan_generation_namespace(&directory).map_err(map_directory_error)?;
    if namespace.marker_files.keys().any(|id| *id >= generation) {
        return invalid(format!(
            "generation {generation} is not newer than every existing committed generation"
        ));
    }
    if !namespace.reservation_files.contains_key(&generation) {
        return invalid(format!(
            "generation {generation} has no retained durable reservation"
        ));
    }

    let log_path = directory.join(canonical_generation_name(generation));
    if candidate.path() != log_path {
        return invalid(format!(
            "Windows compact publication witness path {} does not match canonical generation path {}",
            candidate.path().display(),
            log_path.display()
        ));
    }
    require_real_regular_file(&log_path, "generation log").map_err(map_directory_error)?;

    let marker_path = directory.join(canonical_marker_name(generation));
    let staging_path = directory.join(canonical_staging_marker_name(generation));
    require_absent(&marker_path, "commit marker")?;
    require_absent(&staging_path, "staging commit marker")?;

    let baseline = require_exact_candidate(candidate)?;
    let committed_prefix = derive_prefix_proof(&log_path, &baseline)?;
    let prefix_verification = verify_committed_prefix(&log_path, committed_prefix)?;
    if prefix_verification != baseline {
        return invalid("derived committed-prefix verification disagrees with compact witness");
    }

    let encoded = encode_commit_marker(generation, committed_prefix).map_err(|error| {
        GenerationPublicationError::Invalid(format!("cannot encode commit marker: {error}"))
    })?;
    write_synced_staging(&staging_path, &encoded)?;

    // Marker authority must bind exactly the candidate that received write-through canonical-name
    // publication. Re-check after staging I/O immediately before the authority-changing move.
    let current = require_exact_candidate(candidate)?;
    if current != baseline {
        return invalid("generation changed while Windows marker publication was in progress");
    }

    if let Err(source) = move_no_replace_write_through(&staging_path, &marker_path) {
        match fs::symlink_metadata(&marker_path) {
            Ok(_) => {
                return Err(GenerationPublicationError::DurabilityUncertain {
                    marker: marker_path,
                    source,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let _ = fs::remove_file(&staging_path);
                return Err(io_error(&marker_path, source));
            }
            Err(_) => {
                return Err(GenerationPublicationError::DurabilityUncertain {
                    marker: marker_path,
                    source,
                });
            }
        }
    }

    verify_published_marker(&marker_path, generation, committed_prefix)?;
    let _ = verify_committed_prefix(&log_path, committed_prefix)?;
    let final_candidate = require_exact_candidate(candidate)?;
    if final_candidate != baseline {
        return invalid("generation changed after Windows commit marker publication");
    }

    Ok(GenerationPublicationSummary {
        protocol: GENERATION_MARKER_PUBLICATION_WINDOWS_PROTOCOL,
        marker_format_version: COMMIT_MARKER_VERSION,
        generation,
        generation_log: canonical_generation_name(generation),
        marker: canonical_marker_name(generation),
        committed_prefix,
        staging_retained: false,
    })
}

fn require_exact_candidate(
    candidate: &WindowsDurableCompactOutput,
) -> Result<VerificationReport, GenerationPublicationError> {
    let current = LogEngine::inspect(candidate.path(), true)?;
    if &current != candidate.inspection() {
        return invalid("Windows compact candidate changed after write-through publication");
    }
    if current.verification.recoverable_tail.is_some()
        || current.verification.file_bytes != current.verification.valid_bytes
    {
        return invalid("Windows compact candidate is not a complete clean append-log image");
    }
    Ok(current.verification)
}

fn derive_prefix_proof(
    path: &Path,
    report: &VerificationReport,
) -> Result<CommittedPrefix, GenerationPublicationError> {
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

fn write_synced_staging(
    path: &Path,
    encoded: &[u8; COMMIT_MARKER_LEN],
) -> Result<(), GenerationPublicationError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    file.write_all(encoded)
        .map_err(|source| io_error(path, source))?;
    file.sync_all().map_err(|source| io_error(path, source))
}

fn verify_published_marker(
    path: &Path,
    generation: u64,
    committed_prefix: CommittedPrefix,
) -> Result<CommitMarker, GenerationPublicationError> {
    require_real_regular_file(path, "published commit marker").map_err(map_directory_error)?;
    let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
    let marker = decode_commit_marker(&bytes, generation).map_err(|error| {
        GenerationPublicationError::Invalid(format!("published commit marker: {error}"))
    })?;
    if marker.committed_prefix != committed_prefix {
        return invalid("published commit marker prefix differs from staged proof");
    }
    Ok(marker)
}

fn require_absent(path: &Path, label: &str) -> Result<(), GenerationPublicationError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => invalid(format!("{label} already exists: {}", path.display())),
        Err(source) => Err(io_error(path, source)),
    }
}

fn map_directory_error(error: GenerationDirectoryError) -> GenerationPublicationError {
    match error {
        GenerationDirectoryError::Invalid(message) => GenerationPublicationError::Invalid(message),
        GenerationDirectoryError::Database(error) => GenerationPublicationError::Database(error),
        GenerationDirectoryError::Io { path, source } => {
            GenerationPublicationError::Io { path, source }
        }
    }
}

fn io_error(path: &Path, source: std::io::Error) -> GenerationPublicationError {
    GenerationPublicationError::Io {
        path: PathBuf::from(path),
        source,
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, GenerationPublicationError> {
    Err(GenerationPublicationError::Invalid(message.into()))
}
