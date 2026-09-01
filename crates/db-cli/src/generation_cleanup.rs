#[cfg(windows)]
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::generation_directory::{
    scan_generation_namespace, verify_generation_directory, GenerationDirectoryError,
    GenerationVerificationSummary,
};
use crate::generation_lock::{acquire_generation_writer_lease, GenerationWriterLockError};
#[cfg(windows)]
use crate::windows_durable::move_no_replace_write_through;

pub const GENERATION_CLEANUP_PROTOCOL: &str = "append_log_generation_cleanup_unix_v1";
pub const GENERATION_CLEANUP_WINDOWS_PROTOCOL: &str =
    "append_log_generation_cleanup_windows_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerationCleanupSummary {
    pub protocol: &'static str,
    pub authoritative_generation: u64,
    pub removed_marker_generation_ids: Vec<u64>,
    pub removed_generation_ids: Vec<u64>,
    pub removed_staging_marker_generation_ids: Vec<u64>,
    pub retained_staging_marker_generation_ids: Vec<u64>,
    pub retained_uncommitted_generation_ids: Vec<u64>,
    /// Windows first retires obsolete names to write-through sibling quarantine paths. Physical
    /// deletion of those non-authoritative files is best-effort; any path still present is reported.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retained_quarantine_paths: Vec<String>,
    pub final_generation: GenerationVerificationSummary,
}

#[derive(Debug, Error)]
pub enum GenerationCleanupError {
    #[error("append-log generation cleanup is unsupported on this platform; no retained artifact was removed")]
    UnsupportedPlatform,
    #[error(transparent)]
    Lock(#[from] GenerationWriterLockError),
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error("invalid generation cleanup state: {0}")]
    Invalid(String),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "generation cleanup changed or observed drift in authoritative generation {generation} during {stage}; cleanup stopped fail-closed"
    )]
    AuthorityChanged {
        generation: u64,
        stage: &'static str,
    },
    #[error(
        "generation cleanup removed {phase} names but parent-directory durability could not be confirmed at {directory}: {source}"
    )]
    DurabilityUncertain {
        phase: &'static str,
        directory: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "Windows cleanup retirement may have moved {source_path} to {quarantine_path}, but write-through completion could not be established: {source}"
    )]
    RetirementUncertain {
        source_path: PathBuf,
        quarantine_path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Removes obsolete retained history without changing generation authority.
///
/// Unix removes lower marker/staging names, synchronizes the directory, re-verifies authority, then
/// removes lower generation logs and synchronizes again.
///
/// Windows cannot reuse the Unix parent-directory `sync_all` deletion protocol. Instead it moves
/// each obsolete retained name to a deterministic sibling quarantine path with the audited
/// no-overwrite `MOVEFILE_WRITE_THROUGH` primitive. A successful move durably retires that name from
/// the strict generation namespace; physical deletion of the now-non-authoritative quarantine file
/// is best-effort and is reported separately when it cannot be reclaimed immediately.
///
/// Higher staging markers and higher uncommitted generation logs remain retained. Reservation files
/// are never removed, so the allocation frontier cannot move backwards.
pub fn cleanup_obsolete_generations(
    directory: &Path,
) -> Result<GenerationCleanupSummary, GenerationCleanupError> {
    #[cfg(unix)]
    {
        cleanup_obsolete_generations_unix(directory)
    }

    #[cfg(windows)]
    {
        cleanup_obsolete_generations_windows(directory)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = directory;
        Err(GenerationCleanupError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
fn cleanup_obsolete_generations_unix(
    directory: &Path,
) -> Result<GenerationCleanupSummary, GenerationCleanupError> {
    use std::fs::{self, File};

    let lease = acquire_generation_writer_lease(directory)?;
    let before = verify_generation_directory(lease.directory())?;
    let witness = AuthorityWitness::from_summary(before.summary());
    let authoritative_generation = witness.generation;
    let namespace = scan_generation_namespace(lease.directory())?;

    let plan = CleanupPlan::from_namespace(&namespace, authoritative_generation);
    validate_plan_sources(&namespace, &plan)?;
    require_same_authority(lease.directory(), &witness, "pre-delete verification")?;

    for id in &plan.marker_ids {
        let path = namespace
            .marker_files
            .get(id)
            .expect("planned marker id came from namespace");
        fs::remove_file(path).map_err(|source| io_error(path, source))?;
    }
    for id in &plan.staging_ids {
        let path = namespace
            .staging_marker_files
            .get(id)
            .expect("planned staging id came from namespace");
        fs::remove_file(path).map_err(|source| io_error(path, source))?;
    }
    if !plan.marker_ids.is_empty() || !plan.staging_ids.is_empty() {
        sync_directory(lease.directory()).map_err(|source| {
            GenerationCleanupError::DurabilityUncertain {
                phase: "obsolete marker/staging cleanup",
                directory: lease.directory().to_path_buf(),
                source,
            }
        })?;
    }

    require_same_authority(
        lease.directory(),
        &witness,
        "post-marker pre-generation verification",
    )?;

    for id in &plan.generation_ids {
        let path = namespace
            .generation_files
            .get(id)
            .expect("planned generation id came from namespace");
        fs::remove_file(path).map_err(|source| io_error(path, source))?;
    }
    if !plan.generation_ids.is_empty() {
        sync_directory(lease.directory()).map_err(|source| {
            GenerationCleanupError::DurabilityUncertain {
                phase: "obsolete generation cleanup",
                directory: lease.directory().to_path_buf(),
                source,
            }
        })?;
    }

    let final_verified = require_same_authority(
        lease.directory(),
        &witness,
        "final post-cleanup verification",
    )?;
    build_summary(
        GENERATION_CLEANUP_PROTOCOL,
        authoritative_generation,
        plan,
        lease.directory(),
        final_verified,
        Vec::new(),
    )
}

#[cfg(windows)]
fn cleanup_obsolete_generations_windows(
    directory: &Path,
) -> Result<GenerationCleanupSummary, GenerationCleanupError> {
    let lease = acquire_generation_writer_lease(directory)?;
    let before = verify_generation_directory(lease.directory())?;
    let witness = AuthorityWitness::from_summary(before.summary());
    let authoritative_generation = witness.generation;
    let namespace = scan_generation_namespace(lease.directory())?;
    let plan = CleanupPlan::from_namespace(&namespace, authoritative_generation);

    validate_plan_sources(&namespace, &plan)?;
    let marker_moves = planned_quarantine_moves(
        lease.directory(),
        "marker",
        &plan.marker_ids,
        &namespace.marker_files,
    )?;
    let staging_moves = planned_quarantine_moves(
        lease.directory(),
        "staging",
        &plan.staging_ids,
        &namespace.staging_marker_files,
    )?;
    let generation_moves = planned_quarantine_moves(
        lease.directory(),
        "generation",
        &plan.generation_ids,
        &namespace.generation_files,
    )?;

    require_same_authority(lease.directory(), &witness, "pre-retirement verification")?;

    let mut quarantine_paths = Vec::new();
    for (source, quarantine) in marker_moves.iter().chain(staging_moves.iter()) {
        durable_retire_to_quarantine(source, quarantine)?;
        quarantine_paths.push(quarantine.clone());
    }

    require_same_authority(
        lease.directory(),
        &witness,
        "post-marker pre-generation verification",
    )?;

    for (source, quarantine) in &generation_moves {
        durable_retire_to_quarantine(source, quarantine)?;
        quarantine_paths.push(quarantine.clone());
    }

    let final_verified = require_same_authority(
        lease.directory(),
        &witness,
        "final post-cleanup verification",
    )?;

    // Quarantine files are outside the retained generation namespace, so deleting them cannot alter
    // recovery authority. This is physical-space cleanup only and intentionally is not elevated to a
    // durable deletion claim. Any file that cannot be removed is surfaced in the success summary.
    let retained_quarantine_paths = reclaim_quarantine_best_effort(&quarantine_paths);
    build_summary(
        GENERATION_CLEANUP_WINDOWS_PROTOCOL,
        authoritative_generation,
        plan,
        lease.directory(),
        final_verified,
        retained_quarantine_paths,
    )
}

#[derive(Debug)]
struct CleanupPlan {
    marker_ids: Vec<u64>,
    generation_ids: Vec<u64>,
    staging_ids: Vec<u64>,
}

impl CleanupPlan {
    fn from_namespace(
        namespace: &crate::generation_directory::GenerationNamespace,
        authoritative_generation: u64,
    ) -> Self {
        Self {
            marker_ids: namespace
                .marker_files
                .keys()
                .copied()
                .filter(|id| *id < authoritative_generation)
                .collect(),
            generation_ids: namespace
                .generation_files
                .keys()
                .copied()
                .filter(|id| *id < authoritative_generation)
                .collect(),
            staging_ids: namespace
                .staging_marker_files
                .keys()
                .copied()
                .filter(|id| *id <= authoritative_generation)
                .collect(),
        }
    }
}

fn build_summary(
    protocol: &'static str,
    authoritative_generation: u64,
    plan: CleanupPlan,
    directory: &Path,
    final_verified: GenerationVerificationSummary,
    retained_quarantine_paths: Vec<String>,
) -> Result<GenerationCleanupSummary, GenerationCleanupError> {
    let final_namespace = scan_generation_namespace(directory)?;
    let retained_staging_marker_generation_ids = final_namespace
        .staging_marker_files
        .keys()
        .copied()
        .collect();
    let retained_uncommitted_generation_ids = final_namespace
        .generation_files
        .keys()
        .filter(|id| !final_namespace.marker_files.contains_key(*id))
        .copied()
        .collect();

    Ok(GenerationCleanupSummary {
        protocol,
        authoritative_generation,
        removed_marker_generation_ids: plan.marker_ids,
        removed_generation_ids: plan.generation_ids,
        removed_staging_marker_generation_ids: plan.staging_ids,
        retained_staging_marker_generation_ids,
        retained_uncommitted_generation_ids,
        retained_quarantine_paths,
        final_generation: final_verified,
    })
}

fn validate_plan_sources(
    namespace: &crate::generation_directory::GenerationNamespace,
    plan: &CleanupPlan,
) -> Result<(), GenerationCleanupError> {
    for id in &plan.marker_ids {
        let path = namespace
            .marker_files
            .get(id)
            .expect("planned marker id came from namespace");
        require_real_regular_file(path, "obsolete commit marker")?;
    }
    for id in &plan.staging_ids {
        let path = namespace
            .staging_marker_files
            .get(id)
            .expect("planned staging id came from namespace");
        require_real_regular_file(path, "obsolete staging commit marker")?;
    }
    for id in &plan.generation_ids {
        let path = namespace
            .generation_files
            .get(id)
            .expect("planned generation id came from namespace");
        require_real_regular_file(path, "obsolete generation log")?;
    }
    Ok(())
}

#[cfg(windows)]
fn planned_quarantine_moves(
    directory: &Path,
    kind: &str,
    ids: &[u64],
    files: &std::collections::BTreeMap<u64, PathBuf>,
) -> Result<Vec<(PathBuf, PathBuf)>, GenerationCleanupError> {
    let mut moves = Vec::with_capacity(ids.len());
    for id in ids {
        let source = files
            .get(id)
            .expect("planned quarantine id came from namespace")
            .clone();
        let quarantine = quarantine_path(directory, kind, *id)?;
        require_absent(&quarantine, "cleanup quarantine")?;
        moves.push((source, quarantine));
    }
    Ok(moves)
}

#[cfg(windows)]
fn quarantine_path(
    directory: &Path,
    kind: &str,
    generation: u64,
) -> Result<PathBuf, GenerationCleanupError> {
    let name = directory.file_name().ok_or_else(|| {
        GenerationCleanupError::Invalid(format!(
            "generation directory has no quarantineable final component: {}",
            directory.display()
        ))
    })?;
    let mut quarantine_name = OsString::from(".");
    quarantine_name.push(name);
    quarantine_name.push(format!(
        ".append-log-retired-{kind}-{generation:020}.quarantine"
    ));
    Ok(directory.with_file_name(quarantine_name))
}

#[cfg(windows)]
fn durable_retire_to_quarantine(
    source_path: &Path,
    quarantine_path: &Path,
) -> Result<(), GenerationCleanupError> {
    if let Err(source) = move_no_replace_write_through(source_path, quarantine_path) {
        let source_present = path_exists(source_path)?;
        let quarantine_present = path_exists(quarantine_path)?;
        if !source_present && quarantine_present {
            return Err(GenerationCleanupError::RetirementUncertain {
                source_path: source_path.to_path_buf(),
                quarantine_path: quarantine_path.to_path_buf(),
                source,
            });
        }
        if source_present && !quarantine_present {
            return Err(io_error(source_path, source));
        }
        return Err(GenerationCleanupError::RetirementUncertain {
            source_path: source_path.to_path_buf(),
            quarantine_path: quarantine_path.to_path_buf(),
            source,
        });
    }

    if path_exists(source_path)? || !path_exists(quarantine_path)? {
        return Err(GenerationCleanupError::Invalid(format!(
            "successful Windows retirement did not leave exactly one quarantine copy: source={} quarantine={}",
            source_path.display(),
            quarantine_path.display()
        )));
    }
    require_real_regular_file(quarantine_path, "retired quarantine file")
}

#[cfg(windows)]
fn reclaim_quarantine_best_effort(paths: &[PathBuf]) -> Vec<String> {
    let mut retained = Vec::new();
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => retained.push(path.display().to_string()),
        }
    }
    retained
}

#[cfg(windows)]
fn require_absent(path: &Path, label: &str) -> Result<(), GenerationCleanupError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(GenerationCleanupError::Invalid(format!(
            "{label} already exists: {}",
            path.display()
        ))),
        Err(source) => Err(io_error(path, source)),
    }
}

#[cfg(windows)]
fn path_exists(path: &Path) -> Result<bool, GenerationCleanupError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error(path, source)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthorityWitness {
    generation: u64,
    authoritative_log: String,
    committed_prefix: crate::generation_marker::CommittedPrefix,
    log_verification: db_storage_log::VerificationReport,
}

impl AuthorityWitness {
    fn from_summary(summary: &GenerationVerificationSummary) -> Self {
        Self {
            generation: summary.authoritative_generation,
            authoritative_log: summary.authoritative_log.clone(),
            committed_prefix: summary.committed_prefix,
            log_verification: summary.log_verification.clone(),
        }
    }
}

fn require_same_authority(
    directory: &Path,
    expected: &AuthorityWitness,
    stage: &'static str,
) -> Result<GenerationVerificationSummary, GenerationCleanupError> {
    let current = verify_generation_directory(directory)?;
    let observed = AuthorityWitness::from_summary(current.summary());
    if &observed != expected {
        return Err(GenerationCleanupError::AuthorityChanged {
            generation: expected.generation,
            stage,
        });
    }
    Ok(current.summary().clone())
}

fn require_real_regular_file(path: &Path, label: &str) -> Result<(), GenerationCleanupError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return Err(GenerationCleanupError::Invalid(format!(
            "{label} must be a real regular file rather than a symlink or non-file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> GenerationCleanupError {
    GenerationCleanupError::Io {
        path: path.to_path_buf(),
        source,
    }
}
