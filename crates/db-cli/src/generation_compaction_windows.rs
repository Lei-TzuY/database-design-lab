use std::path::Path;

use db_storage_log::{InspectionReport, LogEngine};

use crate::generation_compaction::{
    OfflineGenerationCompactSwitchError, OfflineGenerationCompactSwitchSummary,
};
use crate::generation_directory::{
    canonical_generation_name, verify_generation_directory, GenerationVerificationSummary,
};
use crate::generation_lock::acquire_generation_writer_lease;
use crate::generation_publication_windows::publish_windows_compact_generation_marker;
use crate::generation_reservation::reserve_next_generation;
use crate::log_compaction::compact_log_to_fresh_file_with_windows_witness;

pub const OFFLINE_GENERATION_COMPACT_SWITCH_WINDOWS_PROTOCOL: &str =
    "append_log_offline_generation_compact_switch_windows_v1";

/// Windows authoritative compact switch using durable reservation, write-through candidate-name
/// publication, the shared writer lease, and witness-bound write-through marker publication.
///
/// The initial retained directory must already verify under the common generation-directory recovery
/// contract. This function does not provide Windows legacy migration/bootstrap; it advances an
/// existing retained authority. Raw-path `LogEngine` writers remain outside the cooperative lease
/// contract and must be quiesced by the caller.
pub fn compact_switch_generation_offline_windows(
    directory: &Path,
) -> Result<OfflineGenerationCompactSwitchSummary, OfflineGenerationCompactSwitchError> {
    let reservation = reserve_next_generation(directory)?;
    let before = verify_generation_directory(directory)?;
    let old_generation = before.summary().authoritative_generation;
    if reservation.generation <= old_generation {
        return Err(
            OfflineGenerationCompactSwitchError::ReservedGenerationObsolete {
                reserved_generation: reservation.generation,
                authoritative_generation: old_generation,
            },
        );
    }

    let old_generation_log = before.summary().authoritative_log.clone();
    let source_path = before.authoritative_log_path();
    let source_state = LogEngine::inspect(&source_path, true)?;
    let source_authority = SourceAuthorityWitness::from_summary(before.summary());
    let new_generation = reservation.generation;
    let new_generation_log = canonical_generation_name(new_generation);
    let new_path = before.directory().join(&new_generation_log);

    let (compaction, candidate) =
        compact_log_to_fresh_file_with_windows_witness(&source_path, &new_path)?;

    // Candidate construction is intentionally outside the short writer lease. Routed writers may
    // proceed while the expensive compact copy is built. The authority-changing section below
    // reacquires the common cross-process lease and rejects any old-source drift before the marker
    // can become authoritative.
    let lease = acquire_generation_writer_lease(before.directory())?;
    let before_publication = verify_generation_directory(lease.directory())?;
    let current_authority = SourceAuthorityWitness::from_summary(before_publication.summary());
    let current_source_state = LogEngine::inspect(&source_path, true)?;
    if current_authority != source_authority || current_source_state != source_state {
        return Err(OfflineGenerationCompactSwitchError::SourceChanged {
            orphan_generation: new_generation,
        });
    }

    let compact_state = LogEngine::inspect(&new_path, true)?;
    validate_compact_state(new_generation, &source_state, &compact_state)?;
    if candidate.inspection() != &compact_state || candidate.path() != new_path {
        return Err(OfflineGenerationCompactSwitchError::TargetChanged {
            generation: new_generation,
        });
    }

    let publication = publish_windows_compact_generation_marker(
        lease.directory(),
        new_generation,
        &candidate,
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

    Ok(OfflineGenerationCompactSwitchSummary {
        protocol: OFFLINE_GENERATION_COMPACT_SWITCH_WINDOWS_PROTOCOL,
        old_generation,
        new_generation,
        old_generation_log,
        new_generation_log,
        reservation,
        compaction,
        publication,
        final_generation: final_verified.summary().clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceAuthorityWitness {
    authoritative_generation: u64,
    committed_prefix: crate::generation_marker::CommittedPrefix,
    log_verification: db_storage_log::VerificationReport,
}

impl SourceAuthorityWitness {
    fn from_summary(summary: &GenerationVerificationSummary) -> Self {
        Self {
            authoritative_generation: summary.authoritative_generation,
            committed_prefix: summary.committed_prefix,
            log_verification: summary.log_verification.clone(),
        }
    }
}

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
