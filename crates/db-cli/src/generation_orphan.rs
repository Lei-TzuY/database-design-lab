use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::generation_directory::{
    canonical_staging_marker_name, require_real_regular_file, scan_generation_namespace,
    verify_generation_directory, GenerationDirectoryError, GenerationVerificationSummary,
};
use crate::generation_lock::{acquire_generation_writer_lease, GenerationWriterLockError};
use crate::generation_marker::Crc32Ieee;

pub const GENERATION_ORPHAN_RETIRE_PROTOCOL: &str = "append_log_generation_orphan_retire_unix_v1";
pub const GENERATION_ORPHAN_INSPECT_PROTOCOL: &str = "append_log_generation_orphan_inspect_v1";
const FRONTIER_SENTINEL_BYTES: &[u8] = b"DBL-GENERATION-FRONTIER-STAGING-V1\n";
const FINGERPRINT_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GenerationFileFingerprint {
    pub bytes: u64,
    pub crc32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerationOrphanInspection {
    pub protocol: &'static str,
    pub authoritative_generation: u64,
    pub orphan_generation: u64,
    pub orphan_log: String,
    pub fingerprint: GenerationFileFingerprint,
    pub staging_frontier_present: bool,
    pub highest_observed_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerationOrphanRetirementSummary {
    pub protocol: &'static str,
    pub authoritative_generation: u64,
    pub retired_generation: u64,
    pub retired_fingerprint: GenerationFileFingerprint,
    pub staging_frontier: String,
    pub staging_frontier_created: bool,
    pub final_generation: GenerationVerificationSummary,
}

#[derive(Debug, Error)]
pub enum GenerationOrphanError {
    #[error("append-log generation orphan retirement is unsupported on this platform; no retained artifact was changed")]
    UnsupportedPlatform,
    #[error("generation orphan retirement requires --confirm-generation-builder-stopped")]
    ConfirmationRequired,
    #[error(transparent)]
    Lock(#[from] GenerationWriterLockError),
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error("invalid generation orphan state: {0}")]
    Invalid(String),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "orphan generation {generation} fingerprint changed: expected bytes={expected_bytes} crc32={expected_crc32:08x}, found bytes={found_bytes} crc32={found_crc32:08x}"
    )]
    FingerprintChanged {
        generation: u64,
        expected_bytes: u64,
        expected_crc32: u32,
        found_bytes: u64,
        found_crc32: u32,
    },
    #[error(
        "expected authoritative generation {expected}, found {found}; orphan retirement stopped fail-closed"
    )]
    AuthorityChanged { expected: u64, found: u64 },
    #[error(
        "generation orphan retirement made {stage} visible but parent-directory durability could not be confirmed at {directory}: {source}"
    )]
    DurabilityUncertain {
        stage: &'static str,
        directory: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Inspects one higher uncommitted generation without mutating the directory.
pub fn inspect_generation_orphan(
    directory: &Path,
    generation: u64,
) -> Result<GenerationOrphanInspection, GenerationOrphanError> {
    let verified = verify_generation_directory(directory)?;
    let namespace = scan_generation_namespace(verified.directory())?;
    let path = validate_orphan_candidate(
        &namespace,
        verified.summary().authoritative_generation,
        generation,
    )?;
    let fingerprint = fingerprint_file(path)?;
    let orphan_log = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| GenerationOrphanError::Invalid("orphan filename is not UTF-8".to_owned()))?
        .to_owned();

    Ok(GenerationOrphanInspection {
        protocol: GENERATION_ORPHAN_INSPECT_PROTOCOL,
        authoritative_generation: verified.summary().authoritative_generation,
        orphan_generation: generation,
        orphan_log,
        fingerprint,
        staging_frontier_present: namespace.staging_marker_files.contains_key(&generation),
        highest_observed_generation: verified.summary().highest_observed_generation,
    })
}

/// Reclaims one explicitly abandoned higher uncommitted generation while retaining its allocation id.
pub fn retire_generation_orphan(
    directory: &Path,
    generation: u64,
    expected_authority: u64,
    expected_fingerprint: GenerationFileFingerprint,
    confirm_generation_builder_stopped: bool,
) -> Result<GenerationOrphanRetirementSummary, GenerationOrphanError> {
    if !confirm_generation_builder_stopped {
        return Err(GenerationOrphanError::ConfirmationRequired);
    }

    #[cfg(unix)]
    {
        retire_generation_orphan_unix(
            directory,
            generation,
            expected_authority,
            expected_fingerprint,
        )
    }

    #[cfg(not(unix))]
    {
        let _ = (directory, generation, expected_authority, expected_fingerprint);
        Err(GenerationOrphanError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
fn retire_generation_orphan_unix(
    directory: &Path,
    generation: u64,
    expected_authority: u64,
    expected_fingerprint: GenerationFileFingerprint,
) -> Result<GenerationOrphanRetirementSummary, GenerationOrphanError> {
    let lease = acquire_generation_writer_lease(directory)?;
    let before = verify_expected_authority(lease.directory(), expected_authority)?;
    let namespace = scan_generation_namespace(lease.directory())?;
    let orphan_path = validate_orphan_candidate(&namespace, expected_authority, generation)?;
    require_fingerprint(generation, orphan_path, expected_fingerprint)?;

    let staging_name = canonical_staging_marker_name(generation);
    let staging_path = lease.directory().join(&staging_name);
    let staging_frontier_created = ensure_durable_staging_frontier(&staging_path)?;
    sync_directory(lease.directory()).map_err(|source| GenerationOrphanError::DurabilityUncertain {
        stage: "allocation-frontier staging publication",
        directory: lease.directory().to_path_buf(),
        source,
    })?;

    let _ = verify_expected_authority(lease.directory(), expected_authority)?;
    let after_frontier = scan_generation_namespace(lease.directory())?;
    let orphan_path = validate_orphan_candidate(&after_frontier, expected_authority, generation)?;
    require_real_regular_file(&staging_path, "orphan-retirement staging frontier")?;
    require_fingerprint(generation, orphan_path, expected_fingerprint)?;

    fs::remove_file(orphan_path).map_err(|source| io_error(orphan_path, source))?;
    sync_directory(lease.directory()).map_err(|source| GenerationOrphanError::DurabilityUncertain {
        stage: "orphan generation removal",
        directory: lease.directory().to_path_buf(),
        source,
    })?;

    let final_verified = verify_expected_authority(lease.directory(), expected_authority)?;
    let final_namespace = scan_generation_namespace(lease.directory())?;
    if final_namespace.generation_files.contains_key(&generation) {
        return invalid(format!(
            "retired generation {generation} still exists after synchronized removal"
        ));
    }
    require_real_regular_file(&staging_path, "retained allocation-frontier staging marker")?;
    if final_verified.summary().highest_observed_generation < generation {
        return invalid(format!(
            "retirement lost allocation frontier {generation}; highest observed generation is {}",
            final_verified.summary().highest_observed_generation
        ));
    }

    Ok(GenerationOrphanRetirementSummary {
        protocol: GENERATION_ORPHAN_RETIRE_PROTOCOL,
        authoritative_generation: before.summary().authoritative_generation,
        retired_generation: generation,
        retired_fingerprint: expected_fingerprint,
        staging_frontier: staging_name,
        staging_frontier_created,
        final_generation: final_verified.summary().clone(),
    })
}

fn validate_orphan_candidate<'a>(
    namespace: &'a crate::generation_directory::GenerationNamespace,
    authoritative_generation: u64,
    generation: u64,
) -> Result<&'a Path, GenerationOrphanError> {
    if generation == 0 {
        return invalid("orphan generation id must be greater than zero");
    }
    if generation <= authoritative_generation {
        return invalid(format!(
            "generation {generation} is not above current authority {authoritative_generation}"
        ));
    }
    if namespace.marker_files.contains_key(&generation) {
        return invalid(format!(
            "generation {generation} has a final commit marker and is not an orphan"
        ));
    }
    let path = namespace.generation_files.get(&generation).ok_or_else(|| {
        GenerationOrphanError::Invalid(format!(
            "generation {generation} has no canonical uncommitted generation log"
        ))
    })?;
    require_real_regular_file(path, "uncommitted generation orphan")?;
    Ok(path)
}

fn fingerprint_file(path: &Path) -> Result<GenerationFileFingerprint, GenerationOrphanError> {
    require_real_regular_file(path, "uncommitted generation orphan")?;
    let mut file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut buffer = [0_u8; FINGERPRINT_BUFFER_BYTES];
    let mut bytes = 0_u64;
    let mut hasher = Crc32Ieee::new();

    loop {
        let read = file.read(&mut buffer).map_err(|source| io_error(path, source))?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or_else(|| GenerationOrphanError::Invalid("orphan byte count overflowed u64".to_owned()))?;
        hasher.update(&buffer[..read]);
    }

    Ok(GenerationFileFingerprint {
        bytes,
        crc32: hasher.finalize(),
    })
}

fn require_fingerprint(
    generation: u64,
    path: &Path,
    expected: GenerationFileFingerprint,
) -> Result<(), GenerationOrphanError> {
    let found = fingerprint_file(path)?;
    if found != expected {
        return Err(GenerationOrphanError::FingerprintChanged {
            generation,
            expected_bytes: expected.bytes,
            expected_crc32: expected.crc32,
            found_bytes: found.bytes,
            found_crc32: found.crc32,
        });
    }
    Ok(())
}

#[cfg(unix)]
fn verify_expected_authority(
    directory: &Path,
    expected: u64,
) -> Result<crate::generation_directory::VerifiedGenerationDirectory, GenerationOrphanError> {
    let verified = verify_generation_directory(directory)?;
    let found = verified.summary().authoritative_generation;
    if found != expected {
        return Err(GenerationOrphanError::AuthorityChanged { expected, found });
    }
    Ok(verified)
}

#[cfg(unix)]
fn ensure_durable_staging_frontier(path: &Path) -> Result<bool, GenerationOrphanError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                return invalid(format!(
                    "allocation-frontier staging path is not a real regular file: {}",
                    path.display()
                ));
            }
            let file = File::open(path).map_err(|source| io_error(path, source))?;
            file.sync_all().map_err(|source| io_error(path, source))?;
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|source| io_error(path, source))?;
            file.write_all(FRONTIER_SENTINEL_BYTES)
                .map_err(|source| io_error(path, source))?;
            file.sync_all().map_err(|source| io_error(path, source))?;
            Ok(true)
        }
        Err(source) => Err(io_error(path, source)),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn io_error(path: &Path, source: io::Error) -> GenerationOrphanError {
    GenerationOrphanError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, GenerationOrphanError> {
    Err(GenerationOrphanError::Invalid(message.into()))
}
