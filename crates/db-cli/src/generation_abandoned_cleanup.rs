use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use db_storage_log::VerificationReport;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generation_directory::{
    scan_generation_namespace, verify_generation_directory, GenerationDirectoryError,
    GenerationVerificationSummary,
};
use crate::generation_lock::{acquire_generation_writer_lease, GenerationWriterLockError};
use crate::generation_marker::{CommittedPrefix, Crc32Ieee};

pub const ABANDONED_CLEANUP_PLAN_PROTOCOL: &str = "append_log_abandoned_generation_cleanup_plan_v1";
pub const ABANDONED_CLEANUP_PROTOCOL: &str = "append_log_abandoned_generation_cleanup_unix_v1";
pub const ABANDONED_CLEANUP_PLAN_FORMAT_VERSION: u32 = 1;
pub const MAX_ABANDONED_CLEANUP_PLAN_BYTES: u64 = 4 * 1024 * 1024;
const CRC_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationEvidence {
    pub file_format_version: u16,
    pub file_bytes: u64,
    pub valid_bytes: u64,
    pub record_count: u64,
    pub live_keys: usize,
    pub next_sequence: u64,
    pub recoverable_tail: Option<TruncatedTailEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TruncatedTailEvidence {
    pub record_offset: u64,
    pub available_bytes: u64,
    pub required_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedPrefixEvidence {
    pub bytes: u64,
    pub crc32: u32,
    pub record_count: u64,
    pub next_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationDirectoryEvidence {
    pub directory_protocol: String,
    pub marker_format_version: u16,
    pub authoritative_generation: u64,
    pub authoritative_log: String,
    pub highest_observed_generation: u64,
    pub marker_generation_ids: Vec<u64>,
    pub staging_marker_generation_ids: Vec<u64>,
    pub reservation_generation_ids: Vec<u64>,
    pub uncommitted_generation_ids: Vec<u64>,
    pub committed_prefix: CommittedPrefixEvidence,
    pub committed_prefix_verification: VerificationEvidence,
    pub log_verification: VerificationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEvidence {
    pub name: String,
    pub bytes: u64,
    pub crc32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbandonedArtifactEvidence {
    pub generation: u64,
    pub reservation: String,
    pub generation_log: Option<FileEvidence>,
    pub staging_marker: Option<FileEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbandonedGenerationCleanupPlan {
    pub format_version: u32,
    pub protocol: String,
    pub directory: GenerationDirectoryEvidence,
    pub eligible_artifacts: Vec<AbandonedArtifactEvidence>,
    pub blocked_unreserved_generation_ids: Vec<u64>,
    pub blocked_unreserved_staging_marker_generation_ids: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AbandonedGenerationCleanupSummary {
    pub protocol: &'static str,
    pub authoritative_generation: u64,
    pub removed_generation_ids: Vec<u64>,
    pub removed_staging_marker_generation_ids: Vec<u64>,
    pub retained_reservation_generation_ids: Vec<u64>,
    pub final_generation: GenerationVerificationSummary,
}

#[derive(Debug, Error)]
pub enum AbandonedGenerationCleanupError {
    #[error("abandoned append-log generation cleanup is unsupported on this platform; no generation-directory artifact was removed")]
    UnsupportedPlatform,
    #[error(transparent)]
    Lock(#[from] GenerationWriterLockError),
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error("invalid abandoned-generation cleanup plan: {0}")]
    InvalidPlan(String),
    #[error("explicit --confirm-abandoned approval is required before destructive cleanup")]
    ConfirmationRequired,
    #[error("abandoned-generation cleanup plan no longer matches retained generation-directory evidence; regenerate the plan before retrying")]
    PlanChanged,
    #[error("planned artifact changed before deletion: {path}")]
    ArtifactChanged { path: PathBuf },
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "abandoned cleanup removed {phase} names but parent-directory durability could not be confirmed at {directory}: {source}"
    )]
    DurabilityUncertain {
        phase: &'static str,
        directory: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("authoritative generation changed during abandoned cleanup; stopped fail-closed after {stage}")]
    AuthorityChanged { stage: &'static str },
    #[error("reservation for cleaned generation {generation} was not retained after cleanup")]
    ReservationLost { generation: u64 },
}

/// Builds an exact, non-destructive cleanup plan while holding the cooperative writer lease.
///
/// Only higher uncommitted generation/staging artifacts with a matching durable reservation are
/// eligible. A reservation retires an id but does not prove abandonment; destructive application
/// therefore requires a separate explicit operator confirmation and exact plan replay.
pub fn plan_abandoned_generation_cleanup(
    directory: &Path,
) -> Result<AbandonedGenerationCleanupPlan, AbandonedGenerationCleanupError> {
    let lease = acquire_generation_writer_lease(directory)?;
    build_plan_locked(lease.directory())
}

/// Loads a bounded strict cleanup plan from a real regular file.
pub fn load_abandoned_generation_cleanup_plan(
    path: &Path,
) -> Result<AbandonedGenerationCleanupPlan, AbandonedGenerationCleanupError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return Err(AbandonedGenerationCleanupError::InvalidPlan(format!(
            "plan must be a real regular file rather than a symlink or non-file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_ABANDONED_CLEANUP_PLAN_BYTES {
        return Err(AbandonedGenerationCleanupError::InvalidPlan(format!(
            "plan has {} bytes, limit is {MAX_ABANDONED_CLEANUP_PLAN_BYTES}",
            metadata.len()
        )));
    }
    let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
    let plan: AbandonedGenerationCleanupPlan = serde_json::from_slice(&bytes).map_err(|error| {
        AbandonedGenerationCleanupError::InvalidPlan(format!("invalid JSON: {error}"))
    })?;
    validate_plan_header(&plan)?;
    Ok(plan)
}

/// Applies only the exact previously planned artifact set after explicit abandonment confirmation.
///
/// Unix reacquires the shared writer lease, recomputes the complete plan, and requires exact equality
/// before deleting anything. Reservations are never removed. Non-Unix targets fail before touching
/// the generation directory because this protocol depends on parent-directory deletion durability.
pub fn apply_abandoned_generation_cleanup(
    directory: &Path,
    expected: &AbandonedGenerationCleanupPlan,
    confirm_abandoned: bool,
) -> Result<AbandonedGenerationCleanupSummary, AbandonedGenerationCleanupError> {
    if !confirm_abandoned {
        return Err(AbandonedGenerationCleanupError::ConfirmationRequired);
    }

    #[cfg(unix)]
    {
        apply_abandoned_generation_cleanup_unix(directory, expected)
    }

    #[cfg(not(unix))]
    {
        let _ = (directory, expected);
        Err(AbandonedGenerationCleanupError::UnsupportedPlatform)
    }
}

fn build_plan_locked(
    directory: &Path,
) -> Result<AbandonedGenerationCleanupPlan, AbandonedGenerationCleanupError> {
    let verified = verify_generation_directory(directory)?;
    let summary = verified.summary();
    let namespace = scan_generation_namespace(verified.directory())?;
    let authority = summary.authoritative_generation;

    let mut eligible_artifacts = Vec::new();
    let mut blocked_unreserved_generation_ids = Vec::new();
    let mut blocked_unreserved_staging_marker_generation_ids = Vec::new();

    let candidate_ids: std::collections::BTreeSet<u64> = namespace
        .generation_files
        .keys()
        .chain(namespace.staging_marker_files.keys())
        .copied()
        .filter(|id| *id > authority && !namespace.marker_files.contains_key(id))
        .collect();

    for generation in candidate_ids {
        let has_reservation = namespace.reservation_files.contains_key(&generation);
        let generation_path = namespace.generation_files.get(&generation);
        let staging_path = namespace.staging_marker_files.get(&generation);
        if !has_reservation {
            if generation_path.is_some() {
                blocked_unreserved_generation_ids.push(generation);
            }
            if staging_path.is_some() {
                blocked_unreserved_staging_marker_generation_ids.push(generation);
            }
            continue;
        }

        let generation_log = generation_path
            .map(|path| fingerprint_file(path, "abandoned generation candidate"))
            .transpose()?;
        let staging_marker = staging_path
            .map(|path| fingerprint_file(path, "abandoned staging marker"))
            .transpose()?;
        let reservation = namespace
            .reservation_files
            .get(&generation)
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                AbandonedGenerationCleanupError::InvalidPlan(format!(
                    "reservation filename for generation {generation} is not UTF-8"
                ))
            })?
            .to_owned();
        eligible_artifacts.push(AbandonedArtifactEvidence {
            generation,
            reservation,
            generation_log,
            staging_marker,
        });
    }

    Ok(AbandonedGenerationCleanupPlan {
        format_version: ABANDONED_CLEANUP_PLAN_FORMAT_VERSION,
        protocol: ABANDONED_CLEANUP_PLAN_PROTOCOL.to_owned(),
        directory: GenerationDirectoryEvidence::from_summary(summary),
        eligible_artifacts,
        blocked_unreserved_generation_ids,
        blocked_unreserved_staging_marker_generation_ids,
    })
}

#[cfg(unix)]
fn apply_abandoned_generation_cleanup_unix(
    directory: &Path,
    expected: &AbandonedGenerationCleanupPlan,
) -> Result<AbandonedGenerationCleanupSummary, AbandonedGenerationCleanupError> {
    validate_plan_header(expected)?;
    let lease = acquire_generation_writer_lease(directory)?;
    let current = build_plan_locked(lease.directory())?;
    if &current != expected {
        return Err(AbandonedGenerationCleanupError::PlanChanged);
    }
    let authority = AuthorityWitness::from_evidence(&current.directory);

    let mut removed_staging_marker_generation_ids = Vec::new();
    for artifact in &current.eligible_artifacts {
        if let Some(expected_file) = &artifact.staging_marker {
            let path = lease.directory().join(&expected_file.name);
            require_matching_file(&path, expected_file, "abandoned staging marker")?;
            fs::remove_file(&path).map_err(|source| io_error(&path, source))?;
            removed_staging_marker_generation_ids.push(artifact.generation);
        }
    }
    if !removed_staging_marker_generation_ids.is_empty() {
        sync_directory(lease.directory()).map_err(|source| {
            AbandonedGenerationCleanupError::DurabilityUncertain {
                phase: "staging-marker cleanup",
                directory: lease.directory().to_path_buf(),
                source,
            }
        })?;
    }
    require_same_authority(lease.directory(), &authority, "staging-marker cleanup")?;

    let mut removed_generation_ids = Vec::new();
    for artifact in &current.eligible_artifacts {
        if let Some(expected_file) = &artifact.generation_log {
            let path = lease.directory().join(&expected_file.name);
            require_matching_file(&path, expected_file, "abandoned generation candidate")?;
            fs::remove_file(&path).map_err(|source| io_error(&path, source))?;
            removed_generation_ids.push(artifact.generation);
        }
    }
    if !removed_generation_ids.is_empty() {
        sync_directory(lease.directory()).map_err(|source| {
            AbandonedGenerationCleanupError::DurabilityUncertain {
                phase: "generation-candidate cleanup",
                directory: lease.directory().to_path_buf(),
                source,
            }
        })?;
    }

    let final_verified = require_same_authority(
        lease.directory(),
        &authority,
        "generation-candidate cleanup",
    )?;
    for artifact in &current.eligible_artifacts {
        if !final_verified
            .reservation_generation_ids
            .contains(&artifact.generation)
        {
            return Err(AbandonedGenerationCleanupError::ReservationLost {
                generation: artifact.generation,
            });
        }
    }

    Ok(AbandonedGenerationCleanupSummary {
        protocol: ABANDONED_CLEANUP_PROTOCOL,
        authoritative_generation: authority.generation,
        removed_generation_ids,
        removed_staging_marker_generation_ids,
        retained_reservation_generation_ids: final_verified.reservation_generation_ids.clone(),
        final_generation: final_verified,
    })
}

fn validate_plan_header(
    plan: &AbandonedGenerationCleanupPlan,
) -> Result<(), AbandonedGenerationCleanupError> {
    if plan.format_version != ABANDONED_CLEANUP_PLAN_FORMAT_VERSION {
        return Err(AbandonedGenerationCleanupError::InvalidPlan(format!(
            "format version {} is unsupported; expected {ABANDONED_CLEANUP_PLAN_FORMAT_VERSION}",
            plan.format_version
        )));
    }
    if plan.protocol != ABANDONED_CLEANUP_PLAN_PROTOCOL {
        return Err(AbandonedGenerationCleanupError::InvalidPlan(format!(
            "protocol {:?} is unsupported; expected {ABANDONED_CLEANUP_PLAN_PROTOCOL:?}",
            plan.protocol
        )));
    }
    Ok(())
}

fn fingerprint_file(
    path: &Path,
    label: &str,
) -> Result<FileEvidence, AbandonedGenerationCleanupError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return Err(AbandonedGenerationCleanupError::InvalidPlan(format!(
            "{label} must be a real regular file rather than a symlink or non-file: {}",
            path.display()
        )));
    }
    let expected_bytes = metadata.len();
    let mut file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut remaining = expected_bytes;
    let mut crc = Crc32Ieee::new();
    let mut buffer = [0_u8; CRC_BUFFER_BYTES];
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(CRC_BUFFER_BYTES as u64))
            .expect("CRC chunk is bounded by a usize-sized constant");
        let read = file
            .read(&mut buffer[..wanted])
            .map_err(|source| io_error(path, source))?;
        if read == 0 {
            return Err(AbandonedGenerationCleanupError::ArtifactChanged {
                path: path.to_path_buf(),
            });
        }
        crc.update(&buffer[..read]);
        remaining -= read as u64;
    }
    let mut extra = [0_u8; 1];
    if file
        .read(&mut extra)
        .map_err(|source| io_error(path, source))?
        != 0
    {
        return Err(AbandonedGenerationCleanupError::ArtifactChanged {
            path: path.to_path_buf(),
        });
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AbandonedGenerationCleanupError::InvalidPlan(format!(
                "artifact filename is not UTF-8: {}",
                path.display()
            ))
        })?
        .to_owned();
    Ok(FileEvidence {
        name,
        bytes: expected_bytes,
        crc32: crc.finalize(),
    })
}

#[cfg(unix)]
fn require_matching_file(
    path: &Path,
    expected: &FileEvidence,
    label: &str,
) -> Result<(), AbandonedGenerationCleanupError> {
    let current = fingerprint_file(path, label)?;
    if &current != expected {
        return Err(AbandonedGenerationCleanupError::ArtifactChanged {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthorityWitness {
    generation: u64,
    authoritative_log: String,
    committed_prefix: CommittedPrefixEvidence,
    log_verification: VerificationEvidence,
}

#[cfg(unix)]
impl AuthorityWitness {
    fn from_evidence(evidence: &GenerationDirectoryEvidence) -> Self {
        Self {
            generation: evidence.authoritative_generation,
            authoritative_log: evidence.authoritative_log.clone(),
            committed_prefix: evidence.committed_prefix.clone(),
            log_verification: evidence.log_verification.clone(),
        }
    }
}

#[cfg(unix)]
fn require_same_authority(
    directory: &Path,
    expected: &AuthorityWitness,
    stage: &'static str,
) -> Result<GenerationVerificationSummary, AbandonedGenerationCleanupError> {
    let verified = verify_generation_directory(directory)?;
    let observed = AuthorityWitness::from_evidence(&GenerationDirectoryEvidence::from_summary(
        verified.summary(),
    ));
    if &observed != expected {
        return Err(AbandonedGenerationCleanupError::AuthorityChanged { stage });
    }
    Ok(verified.summary().clone())
}

impl GenerationDirectoryEvidence {
    fn from_summary(summary: &GenerationVerificationSummary) -> Self {
        Self {
            directory_protocol: summary.protocol.to_owned(),
            marker_format_version: summary.marker_format_version,
            authoritative_generation: summary.authoritative_generation,
            authoritative_log: summary.authoritative_log.clone(),
            highest_observed_generation: summary.highest_observed_generation,
            marker_generation_ids: summary.marker_generation_ids.clone(),
            staging_marker_generation_ids: summary.staging_marker_generation_ids.clone(),
            reservation_generation_ids: summary.reservation_generation_ids.clone(),
            uncommitted_generation_ids: summary.uncommitted_generation_ids.clone(),
            committed_prefix: CommittedPrefixEvidence::from_prefix(summary.committed_prefix),
            committed_prefix_verification: VerificationEvidence::from_report(
                &summary.committed_prefix_verification,
            ),
            log_verification: VerificationEvidence::from_report(&summary.log_verification),
        }
    }
}

impl CommittedPrefixEvidence {
    fn from_prefix(prefix: CommittedPrefix) -> Self {
        Self {
            bytes: prefix.bytes,
            crc32: prefix.crc32,
            record_count: prefix.record_count,
            next_sequence: prefix.next_sequence,
        }
    }
}

impl VerificationEvidence {
    fn from_report(report: &VerificationReport) -> Self {
        Self {
            file_format_version: report.file_format_version,
            file_bytes: report.file_bytes,
            valid_bytes: report.valid_bytes,
            record_count: report.record_count,
            live_keys: report.live_keys,
            next_sequence: report.next_sequence,
            recoverable_tail: report.recoverable_tail.as_ref().map(|tail| TruncatedTailEvidence {
                record_offset: tail.record_offset,
                available_bytes: tail.available_bytes,
                required_bytes: tail.required_bytes,
            }),
        }
    }
}

fn io_error(path: &Path, source: io::Error) -> AbandonedGenerationCleanupError {
    AbandonedGenerationCleanupError::Io {
        path: path.to_path_buf(),
        source,
    }
}
