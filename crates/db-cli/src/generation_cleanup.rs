use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::generation_directory::{
    require_real_regular_file, scan_generation_namespace, verify_generation_directory,
    GenerationDirectoryError,
};
use crate::generation_lock::{acquire_generation_writer_lease, GenerationWriterLockError};

pub const GENERATION_CLEANUP_PROTOCOL: &str = "append_log_generation_cleanup_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerationCleanupSummary {
    pub protocol: &'static str,
    pub authoritative_generation: u64,
    pub highest_observed_generation_before: u64,
    pub highest_observed_generation_after: u64,
    pub removed_generation_ids: Vec<u64>,
    pub removed_marker_generation_ids: Vec<u64>,
    pub removed_staging_marker_generation_ids: Vec<u64>,
    pub retained_future_generation_ids: Vec<u64>,
    pub retained_future_staging_marker_generation_ids: Vec<u64>,
    pub directory_sync_confirmed: bool,
}

#[derive(Debug, Error)]
pub enum GenerationCleanupError {
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error(transparent)]
    WriterLock(#[from] GenerationWriterLockError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "generation cleanup changed authority from {expected} to {found}; cleanup stopped fail-closed"
    )]
    AuthorityChanged { expected: u64, found: u64 },
    #[error("authoritative generation {generation} changed while lower generations were cleaned")]
    AuthoritativeStateChanged { generation: u64 },
    #[cfg(unix)]
    #[error(
        "lower-generation cleanup completed in the visible namespace but parent-directory durability could not be confirmed: {source}"
    )]
    DurabilityUncertain {
        #[source]
        source: io::Error,
    },
}

/// Reclaims only generation artifacts whose ids are strictly below current committed authority.
///
/// Higher uncommitted generation logs and staging markers are deliberately retained because they
/// preserve the monotonic allocation frontier. Once a later successful switch advances authority
/// above those ids, a subsequent cleanup may reclaim them safely.
pub fn cleanup_obsolete_generations(
    directory: &Path,
) -> Result<GenerationCleanupSummary, GenerationCleanupError> {
    let lease = acquire_generation_writer_lease(directory)?;
    let before = verify_generation_directory(lease.directory())?;
    let authoritative_generation = before.summary().authoritative_generation;
    let authoritative_prefix = before.summary().committed_prefix;
    let authoritative_log_verification = before.summary().log_verification.clone();
    let highest_observed_generation_before = before.summary().highest_observed_generation;
    let namespace = scan_generation_namespace(lease.directory())?;

    let generation_candidates = below_authority(&namespace.generation_files, authoritative_generation);
    let marker_candidates = below_authority(&namespace.marker_files, authoritative_generation);
    let staging_candidates = below_authority(&namespace.staging_marker_files, authoritative_generation);

    // Validate every deletion target before the first mutation. Cleanup never follows a symlink or
    // deletes a non-regular object merely because its name looks canonical.
    validate_candidates(&generation_candidates, "obsolete generation log")?;
    validate_candidates(&marker_candidates, "obsolete commit marker")?;
    validate_candidates(&staging_candidates, "obsolete staging marker")?;

    // Remove lower final markers before their lower logs. Any interruption therefore turns retained
    // old data into a harmless uncommitted lower generation rather than leaving a marker for a log
    // that this cleanup already removed. Lower ids can never override current authority either way.
    remove_candidates(&marker_candidates)?;
    remove_candidates(&staging_candidates)?;
    remove_candidates(&generation_candidates)?;

    #[cfg(unix)]
    let directory_sync_confirmed = {
        let directory_file =
            File::open(lease.directory()).map_err(|source| GenerationCleanupError::Io {
                path: lease.directory().to_path_buf(),
                source,
            })?;
        if let Err(source) = directory_file.sync_all() {
            return Err(GenerationCleanupError::DurabilityUncertain { source });
        }
        true
    };
    #[cfg(not(unix))]
    let directory_sync_confirmed = false;

    let after = verify_generation_directory(lease.directory())?;
    let found = after.summary().authoritative_generation;
    if found != authoritative_generation {
        return Err(GenerationCleanupError::AuthorityChanged {
            expected: authoritative_generation,
            found,
        });
    }
    if after.summary().committed_prefix != authoritative_prefix
        || after.summary().log_verification != authoritative_log_verification
    {
        return Err(GenerationCleanupError::AuthoritativeStateChanged {
            generation: authoritative_generation,
        });
    }

    let highest_observed_generation_after = after.summary().highest_observed_generation;
    let after_namespace = scan_generation_namespace(lease.directory())?;
    let retained_future_generation_ids = after_namespace
        .generation_files
        .keys()
        .copied()
        .filter(|id| *id > authoritative_generation)
        .collect();
    let retained_future_staging_marker_generation_ids = after_namespace
        .staging_marker_files
        .keys()
        .copied()
        .filter(|id| *id > authoritative_generation)
        .collect();

    Ok(GenerationCleanupSummary {
        protocol: GENERATION_CLEANUP_PROTOCOL,
        authoritative_generation,
        highest_observed_generation_before,
        highest_observed_generation_after,
        removed_generation_ids: generation_candidates.iter().map(|(id, _)| *id).collect(),
        removed_marker_generation_ids: marker_candidates.iter().map(|(id, _)| *id).collect(),
        removed_staging_marker_generation_ids: staging_candidates
            .iter()
            .map(|(id, _)| *id)
            .collect(),
        retained_future_generation_ids,
        retained_future_staging_marker_generation_ids,
        directory_sync_confirmed,
    })
}

fn below_authority(
    files: &std::collections::BTreeMap<u64, PathBuf>,
    authority: u64,
) -> Vec<(u64, PathBuf)> {
    files
        .iter()
        .filter(|(id, _)| **id < authority)
        .map(|(id, path)| (*id, path.clone()))
        .collect()
}

fn validate_candidates(
    candidates: &[(u64, PathBuf)],
    label: &str,
) -> Result<(), GenerationCleanupError> {
    for (_, path) in candidates {
        require_real_regular_file(path, label)?;
    }
    Ok(())
}

fn remove_candidates(candidates: &[(u64, PathBuf)]) -> Result<(), GenerationCleanupError> {
    for (_, path) in candidates {
        fs::remove_file(path).map_err(|source| GenerationCleanupError::Io {
            path: path.clone(),
            source,
        })?;
    }
    Ok(())
}
