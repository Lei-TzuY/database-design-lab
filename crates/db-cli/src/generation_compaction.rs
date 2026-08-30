use std::path::Path;

use db_core::DbError;
#[cfg(unix)]
use db_storage_log::{InspectionReport, LogEngine};
use serde::Serialize;
use thiserror::Error;

#[cfg(unix)]
use crate::generation_directory::{canonical_generation_name, verify_generation_directory};
use crate::generation_directory::{GenerationDirectoryError, GenerationVerificationSummary};
#[cfg(unix)]
use crate::generation_lock::acquire_generation_writer_lease;
use crate::generation_lock::GenerationWriterLockError;
#[cfg(unix)]
use crate::generation_publication::{
    publish_generation_marker_with_hook, GenerationPublicationStage,
};
use crate::generation_publication::{GenerationPublicationError, GenerationPublicationSummary};
#[cfg(unix)]
use crate::log_compaction::compact_log_to_fresh_file;
use crate::log_compaction::{LogCompactionError, LogCompactionReport};

pub const OFFLINE_GENERATION_COMPACT_SWITCH_PROTOCOL: &str =
    "append_log_offline_generation_compact_switch_unix_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfflineGenerationCompactSwitchSummary {
    pub protocol: &'static str,
    pub old_generation: u64,
    pub new_generation: u64,
    pub old_generation_log: String,
    pub new_generation_log: String,
    pub compaction: LogCompactionReport,
    pub publication: GenerationPublicationSummary,
    pub final_generation: GenerationVerificationSummary,
}

#[derive(Debug, Error)]
pub enum OfflineGenerationCompactSwitchError {
    #[error("offline generation compact switch is unsupported on this platform; no artifact was written")]
    UnsupportedPlatform,
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error(transparent)]
    WriterLock(#[from] GenerationWriterLockError),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Compaction(#[from] LogCompactionError),
    #[error(transparent)]
    Publication(#[from] GenerationPublicationError),
    #[error(
        "authoritative source changed while offline compaction was in progress; generation {orphan_generation} remains uncommitted and must not be treated as authoritative"
    )]
    SourceChanged { orphan_generation: u64 },
    #[error("new compact generation {generation} changed before marker publication")]
    TargetChanged { generation: u64 },
    #[error(
        "post-publication verification selected generation {found}, expected newly committed generation {expected}"
    )]
    FinalAuthority { found: u64, expected: u64 },
    #[error("new authoritative generation {generation} changed after marker publication")]
    FinalState { generation: u64 },
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenerationCompactSwitchStage {
    AfterCandidateBuild,
    AfterLockedSourceRecheck,
    AfterTargetVerification,
    AfterGenerationSync,
    AfterGenerationDirectorySync,
    AfterStagingMarkerSync,
    AfterFinalMarkerLink,
    AfterFinalDirectorySync,
    AfterPublishedEvidenceVerification,
    AfterFinalSwitchVerification,
}

#[cfg(unix)]
impl From<GenerationPublicationStage> for GenerationCompactSwitchStage {
    fn from(stage: GenerationPublicationStage) -> Self {
        match stage {
            GenerationPublicationStage::AfterGenerationSync => Self::AfterGenerationSync,
            GenerationPublicationStage::AfterGenerationDirectorySync => {
                Self::AfterGenerationDirectorySync
            }
            GenerationPublicationStage::AfterStagingMarkerSync => Self::AfterStagingMarkerSync,
            GenerationPublicationStage::AfterFinalMarkerLink => Self::AfterFinalMarkerLink,
            GenerationPublicationStage::AfterFinalDirectorySync => Self::AfterFinalDirectorySync,
            GenerationPublicationStage::AfterPublishedEvidenceVerification => {
                Self::AfterPublishedEvidenceVerification
            }
        }
    }
}

/// Offline authoritative compact switch for a generation directory.
///
/// The expensive compact-copy build runs without the writer lease. Immediately before authority can
/// change, the function acquires the cooperative cross-process writer lease, re-verifies the old
/// authority and complete source state, verifies the compact candidate, publishes the marker, and
/// verifies the new authority while still holding the lease. `GenerationLogEngine` operations use
/// the same lease, closing their final check-to-publication race. Raw-path `LogEngine` users are
/// outside this coordination contract and still must be quiesced by the caller.
pub fn compact_switch_generation_offline(
    directory: &Path,
) -> Result<OfflineGenerationCompactSwitchSummary, OfflineGenerationCompactSwitchError> {
    #[cfg(unix)]
    {
        compact_switch_generation_offline_impl(directory, |_| Ok(()))
    }

    #[cfg(not(unix))]
    {
        let _ = directory;
        Err(OfflineGenerationCompactSwitchError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
fn compact_switch_generation_offline_impl<F>(
    directory: &Path,
    after_compaction: F,
) -> Result<OfflineGenerationCompactSwitchSummary, OfflineGenerationCompactSwitchError>
where
    F: FnOnce(&Path) -> Result<(), OfflineGenerationCompactSwitchError>,
{
    compact_switch_generation_offline_with_fault_hook(directory, after_compaction, |_| Ok(()))
}

#[cfg(unix)]
fn compact_switch_generation_offline_with_fault_hook<F, H>(
    directory: &Path,
    after_compaction: F,
    mut fault_hook: H,
) -> Result<OfflineGenerationCompactSwitchSummary, OfflineGenerationCompactSwitchError>
where
    F: FnOnce(&Path) -> Result<(), OfflineGenerationCompactSwitchError>,
    H: FnMut(GenerationCompactSwitchStage) -> Result<(), String>,
{
    let before = verify_generation_directory(directory)?;
    let old_generation = before.summary().authoritative_generation;
    let old_generation_log = before.summary().authoritative_log.clone();
    let source_path = before.authoritative_log_path();
    let source_state = LogEngine::inspect(&source_path, true)?;
    let source_authority = SourceAuthorityWitness::from_summary(before.summary());
    let new_generation = before.next_generation_id()?;
    let new_generation_log = canonical_generation_name(new_generation);
    let new_path = before.directory().join(&new_generation_log);

    let compaction = compact_log_to_fresh_file(&source_path, &new_path)?;
    fault_hook(GenerationCompactSwitchStage::AfterCandidateBuild)
        .map_err(injected_switch_fault)?;
    after_compaction(&source_path)?;

    // Only the authority-changing critical section is exclusive. A compliant routed writer may run
    // during compact-copy construction; if it did, this locked recheck detects the drift and leaves
    // the candidate as a harmless uncommitted orphan.
    let lease = acquire_generation_writer_lease(before.directory())?;
    let before_publication = verify_generation_directory(lease.directory())?;
    let current_authority = SourceAuthorityWitness::from_summary(before_publication.summary());
    let current_source_state = LogEngine::inspect(&source_path, true)?;
    if current_authority != source_authority || current_source_state != source_state {
        return Err(OfflineGenerationCompactSwitchError::SourceChanged {
            orphan_generation: new_generation,
        });
    }
    fault_hook(GenerationCompactSwitchStage::AfterLockedSourceRecheck)
        .map_err(injected_switch_fault)?;

    let compact_state = LogEngine::inspect(&new_path, true)?;
    validate_compact_state(new_generation, &source_state, &compact_state)?;
    fault_hook(GenerationCompactSwitchStage::AfterTargetVerification)
        .map_err(injected_switch_fault)?;

    let publication = publish_generation_marker_with_hook(
        lease.directory(),
        new_generation,
        |publication_stage| {
            fault_hook(publication_stage.into()).map_err(|message| {
                GenerationPublicationError::Invalid(format!(
                    "injected composed compact-switch fault: {message}"
                ))
            })
        },
    )?;
    let final_verified = verify_generation_directory(lease.directory())?;
    if final_verified.summary().authoritative_generation != new_generation {
        return Err(OfflineGenerationCompactSwitchError::FinalAuthority {
            found: final_verified.summary().authoritative_generation,
            expected: new_generation,
        });
    }

    let final_state = LogEngine::inspect(&new_path, true)?;
    if final_state != compact_state {
        return Err(OfflineGenerationCompactSwitchError::FinalState {
            generation: new_generation,
        });
    }
    fault_hook(GenerationCompactSwitchStage::AfterFinalSwitchVerification)
        .map_err(injected_switch_fault)?;

    Ok(OfflineGenerationCompactSwitchSummary {
        protocol: OFFLINE_GENERATION_COMPACT_SWITCH_PROTOCOL,
        old_generation,
        new_generation,
        old_generation_log,
        new_generation_log,
        compaction,
        publication,
        final_generation: final_verified.summary().clone(),
    })
}

#[cfg(unix)]
fn injected_switch_fault(message: String) -> OfflineGenerationCompactSwitchError {
    OfflineGenerationCompactSwitchError::Directory(GenerationDirectoryError::Invalid(format!(
        "injected composed compact-switch fault: {message}"
    )))
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceAuthorityWitness {
    authoritative_generation: u64,
    committed_prefix: crate::generation_marker::CommittedPrefix,
    log_verification: db_storage_log::VerificationReport,
}

#[cfg(unix)]
impl SourceAuthorityWitness {
    fn from_summary(summary: &GenerationVerificationSummary) -> Self {
        Self {
            authoritative_generation: summary.authoritative_generation,
            committed_prefix: summary.committed_prefix,
            log_verification: summary.log_verification.clone(),
        }
    }
}

#[cfg(unix)]
fn validate_compact_state(
    generation: u64,
    source: &InspectionReport,
    compacted: &InspectionReport,
) -> Result<(), OfflineGenerationCompactSwitchError> {
    if compacted.verification.recoverable_tail.is_some()
        || compacted.verification.live_keys != source.verification.live_keys
        || compacted.entries != source.entries
    {
        return Err(OfflineGenerationCompactSwitchError::TargetChanged { generation });
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;

    use db_core::KvEngine;
    use tempfile::tempdir;

    use super::*;
    use crate::generation_directory::{
        canonical_marker_name, canonical_staging_marker_name, verify_generation_directory,
    };
    use crate::generation_lock::generation_writer_lock_path;
    use crate::generation_publication::publish_generation_marker;

    const FAULT_STAGES: [GenerationCompactSwitchStage; 10] = [
        GenerationCompactSwitchStage::AfterCandidateBuild,
        GenerationCompactSwitchStage::AfterLockedSourceRecheck,
        GenerationCompactSwitchStage::AfterTargetVerification,
        GenerationCompactSwitchStage::AfterGenerationSync,
        GenerationCompactSwitchStage::AfterGenerationDirectorySync,
        GenerationCompactSwitchStage::AfterStagingMarkerSync,
        GenerationCompactSwitchStage::AfterFinalMarkerLink,
        GenerationCompactSwitchStage::AfterFinalDirectorySync,
        GenerationCompactSwitchStage::AfterPublishedEvidenceVerification,
        GenerationCompactSwitchStage::AfterFinalSwitchVerification,
    ];

    #[test]
    fn source_drift_after_compaction_never_publishes_stale_generation() {
        let root = tempdir().expect("temporary root");
        let directory = root.path().join("generations");
        fs::create_dir(&directory).expect("create generation directory");
        let source = directory.join(canonical_generation_name(1));
        {
            let mut engine = LogEngine::create_new(&source).expect("create source generation");
            engine.put(b"a", b"one").expect("put initial value");
        }
        publish_generation_marker(&directory, 1).expect("publish source generation");

        let error = compact_switch_generation_offline_impl(&directory, |source_path| {
            let mut engine = LogEngine::open(source_path).expect("open source for injected drift");
            engine.put(b"late", b"write").expect("inject late write");
            Ok(())
        })
        .expect_err("source drift must fail the switch");

        assert!(matches!(
            error,
            OfflineGenerationCompactSwitchError::SourceChanged {
                orphan_generation: 2
            }
        ));
        assert!(directory.join(canonical_generation_name(2)).is_file());
        assert!(!directory.join(canonical_marker_name(2)).exists());
        let verified = verify_generation_directory(&directory).expect("verify old authority");
        assert_eq!(verified.summary().authoritative_generation, 1);
    }

    #[test]
    fn every_composed_fault_boundary_recovers_to_a_complete_old_or_new_authority() {
        for fault_stage in FAULT_STAGES {
            let root = tempdir().expect("temporary root");
            let directory = root.path().join("generations");
            fs::create_dir(&directory).expect("create generation directory");
            let source = directory.join(canonical_generation_name(1));
            {
                let mut engine = LogEngine::create_new(&source).expect("create source generation");
                engine.put(b"a", b"one").expect("put a one");
                engine.put(b"a", b"two").expect("overwrite a");
                engine.put(b"b", b"three").expect("put b");
                engine.delete(b"b").expect("delete b");
                engine.put(b"c", b"").expect("put empty c");
            }
            publish_generation_marker(&directory, 1).expect("publish source generation");
            let source_before = LogEngine::inspect(&source, true).expect("inspect source before fault");

            let error = compact_switch_generation_offline_with_fault_hook(
                &directory,
                |_| Ok(()),
                |observed| {
                    if observed == fault_stage {
                        Err(format!("{fault_stage:?}"))
                    } else {
                        Ok(())
                    }
                },
            )
            .expect_err("injected boundary must report failure");
            assert!(
                error.to_string().contains("injected composed compact-switch fault"),
                "unexpected fault error at {fault_stage:?}: {error}"
            );

            let lock_path = generation_writer_lock_path(&directory).expect("derive writer lock path");
            assert!(
                !lock_path.exists(),
                "writer lease must be released after fault at {fault_stage:?}"
            );
            let source_after = LogEngine::inspect(&source, true).expect("inspect retained source");
            assert_eq!(
                source_after, source_before,
                "old generation changed at fault {fault_stage:?}"
            );

            let new_path = directory.join(canonical_generation_name(2));
            assert!(
                new_path.is_file(),
                "compact candidate missing at fault {fault_stage:?}"
            );
            let new_state = LogEngine::inspect(&new_path, true).expect("inspect compact candidate");
            assert_eq!(
                new_state.entries, source_before.entries,
                "compact candidate lost live state at fault {fault_stage:?}"
            );
            assert_eq!(
                new_state.verification.record_count,
                u64::try_from(source_before.entries.len()).expect("live entry count fits u64"),
                "candidate is not compact at fault {fault_stage:?}"
            );

            let verified = verify_generation_directory(&directory).expect("verify recovery authority");
            let new_authority_visible = matches!(
                fault_stage,
                GenerationCompactSwitchStage::AfterFinalMarkerLink
                    | GenerationCompactSwitchStage::AfterFinalDirectorySync
                    | GenerationCompactSwitchStage::AfterPublishedEvidenceVerification
                    | GenerationCompactSwitchStage::AfterFinalSwitchVerification
            );
            let expected_generation = if new_authority_visible { 2 } else { 1 };
            assert_eq!(
                verified.summary().authoritative_generation,
                expected_generation,
                "wrong recovery authority at fault {fault_stage:?}"
            );
            assert_eq!(
                directory.join(canonical_marker_name(2)).exists(),
                new_authority_visible,
                "final marker visibility disagrees at fault {fault_stage:?}"
            );

            let staging_visible = matches!(
                fault_stage,
                GenerationCompactSwitchStage::AfterStagingMarkerSync
                    | GenerationCompactSwitchStage::AfterFinalMarkerLink
                    | GenerationCompactSwitchStage::AfterFinalDirectorySync
                    | GenerationCompactSwitchStage::AfterPublishedEvidenceVerification
            );
            assert_eq!(
                directory
                    .join(canonical_staging_marker_name(2))
                    .exists(),
                staging_visible,
                "staging-marker visibility disagrees at fault {fault_stage:?}"
            );
        }
    }
}
