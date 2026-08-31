use std::io;
use std::path::{Path, PathBuf};

use db_core::DbError;
#[cfg(unix)]
use db_storage_log::{InspectionReport, LogEngine, VerificationReport};
#[cfg(not(unix))]
use db_storage_log::VerificationReport;
use serde::Serialize;
use thiserror::Error;

use crate::generation_directory::{GenerationDirectoryError, GenerationVerificationSummary};
#[cfg(unix)]
use crate::generation_lock::acquire_generation_writer_lease;
use crate::generation_lock::GenerationWriterLockError;
use crate::generation_publication::{GenerationPublicationError, GenerationPublicationSummary};
use crate::log_compaction::{LogCompactionError, LogCompactionReport};

pub const LEGACY_LOG_BOOTSTRAP_PROTOCOL: &str = "append_log_legacy_bootstrap_unix_v1";
pub const BOOTSTRAP_GENERATION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyLogBootstrapSummary {
    pub protocol: &'static str,
    pub source: String,
    pub target_directory: String,
    pub source_verification: VerificationReport,
    pub generation: u64,
    pub reservation: String,
    pub compaction: LogCompactionReport,
    pub publication: GenerationPublicationSummary,
    pub final_generation: GenerationVerificationSummary,
}

#[derive(Debug, Error)]
pub enum LegacyLogBootstrapError {
    #[error("legacy append-log bootstrap is unsupported on this platform; no filesystem path was accessed")]
    UnsupportedPlatform,
    #[error("invalid legacy append-log bootstrap: {0}")]
    Invalid(String),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Lock(#[from] GenerationWriterLockError),
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error(transparent)]
    Compaction(#[from] LogCompactionError),
    #[error(transparent)]
    Publication(#[from] GenerationPublicationError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "new generation directory {directory} is visible but its parent-directory entry durability could not be confirmed: {source}; preserve the legacy source and inspect the target before retrying"
    )]
    TargetDirectoryDurabilityUncertain {
        directory: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "bootstrap reservation for generation 1 is visible but generation-directory durability could not be confirmed at {directory}: {source}; preserve the legacy source and inspect the target before retrying"
    )]
    ReservationDurabilityUncertain {
        directory: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("legacy source changed before generation 1 could be committed; target remains non-authoritative bootstrap evidence")]
    SourceChangedBeforePublication,
    #[error(
        "legacy source changed after generation 1 became committed; preserve both source and target and reconcile explicitly before selecting one for further writes"
    )]
    SourceChangedAfterPublication,
    #[error(
        "post-bootstrap verification selected generation {found}, expected bootstrap generation 1"
    )]
    FinalAuthority { found: u64 },
    #[error("committed bootstrap generation 1 does not reproduce the verified legacy live state")]
    FinalState,
}

/// Bootstraps a fresh generation directory from one clean legacy append-log v1 file.
///
/// The legacy source is always read-only and is never deleted or renamed. Callers MUST quiesce every
/// raw-path writer to the source for the full call. The generation writer lease protects only the new
/// target directory; it cannot serialize an independent legacy `LogEngine` handle.
pub fn bootstrap_legacy_log(
    source: &Path,
    target_directory: &Path,
) -> Result<LegacyLogBootstrapSummary, LegacyLogBootstrapError> {
    #[cfg(unix)]
    {
        bootstrap_legacy_log_impl(source, target_directory, |_| Ok(()), |_| Ok(()))
    }

    #[cfg(not(unix))]
    {
        let _ = (source, target_directory);
        Err(LegacyLogBootstrapError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
fn bootstrap_legacy_log_impl<BeforePublication, AfterPublication>(
    source: &Path,
    target_directory: &Path,
    before_publication: BeforePublication,
    after_publication: AfterPublication,
) -> Result<LegacyLogBootstrapSummary, LegacyLogBootstrapError>
where
    BeforePublication: FnOnce(&Path) -> Result<(), LegacyLogBootstrapError>,
    AfterPublication: FnOnce(&Path) -> Result<(), LegacyLogBootstrapError>,
{
    use std::fs::{self, File, OpenOptions};

    use crate::generation_directory::{
        canonical_generation_name, canonical_reservation_name, require_real_regular_file,
        scan_generation_namespace, verify_generation_directory,
    };
    use crate::generation_publication::publish_generation_marker;
    use crate::log_compaction::compact_log_to_fresh_file;

    require_real_regular_file(source, "legacy append-log source")?;
    let source = fs::canonicalize(source).map_err(|source_error| io_error(source, source_error))?;
    let baseline = require_clean_source(&source)?;

    let target_parent_input = target_directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let target_name = target_directory.file_name().ok_or_else(|| {
        LegacyLogBootstrapError::Invalid(
            "target generation directory must name one fresh directory entry".to_owned(),
        )
    })?;
    let target_parent_metadata = fs::symlink_metadata(target_parent_input)
        .map_err(|source_error| io_error(target_parent_input, source_error))?;
    if !target_parent_metadata.file_type().is_dir() {
        return invalid(format!(
            "target parent must be a real directory rather than a symlink or non-directory: {}",
            target_parent_input.display()
        ));
    }
    let target_parent = fs::canonicalize(target_parent_input)
        .map_err(|source_error| io_error(target_parent_input, source_error))?;
    let target_directory = target_parent.join(target_name);
    match fs::symlink_metadata(&target_directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return invalid(format!(
                "target generation directory already exists: {}",
                target_directory.display()
            ));
        }
        Err(source_error) => return Err(io_error(&target_directory, source_error)),
    }

    fs::create_dir(&target_directory).map_err(|source_error| io_error(&target_directory, source_error))?;
    if let Err(source_error) = File::open(&target_parent).and_then(|parent| parent.sync_all()) {
        return Err(LegacyLogBootstrapError::TargetDirectoryDurabilityUncertain {
            directory: target_directory,
            source: source_error,
        });
    }

    let lease = acquire_generation_writer_lease(&target_directory)?;
    let initial_namespace = scan_generation_namespace(lease.directory())?;
    if !initial_namespace.generation_files.is_empty()
        || !initial_namespace.marker_files.is_empty()
        || !initial_namespace.staging_marker_files.is_empty()
        || !initial_namespace.reservation_files.is_empty()
    {
        return invalid("fresh bootstrap target unexpectedly contains generation evidence");
    }

    let reservation = canonical_reservation_name(BOOTSTRAP_GENERATION);
    let reservation_path = lease.directory().join(&reservation);
    let reservation_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&reservation_path)
        .map_err(|source_error| io_error(&reservation_path, source_error))?;
    reservation_file
        .sync_all()
        .map_err(|source_error| io_error(&reservation_path, source_error))?;
    if let Err(source_error) = File::open(lease.directory()).and_then(|directory| directory.sync_all()) {
        return Err(LegacyLogBootstrapError::ReservationDurabilityUncertain {
            directory: lease.directory().to_path_buf(),
            source: source_error,
        });
    }
    let reserved_namespace = scan_generation_namespace(lease.directory())?;
    if reserved_namespace.reservation_files.len() != 1
        || !reserved_namespace
            .reservation_files
            .contains_key(&BOOTSTRAP_GENERATION)
        || !reserved_namespace.generation_files.is_empty()
        || !reserved_namespace.marker_files.is_empty()
        || !reserved_namespace.staging_marker_files.is_empty()
    {
        return invalid("bootstrap generation 1 reservation was not retained exactly");
    }

    let generation_name = canonical_generation_name(BOOTSTRAP_GENERATION);
    let generation_path = lease.directory().join(&generation_name);
    let compaction = compact_log_to_fresh_file(&source, &generation_path)?;
    before_publication(&source)?;

    if require_clean_source(&source)? != baseline {
        return Err(LegacyLogBootstrapError::SourceChangedBeforePublication);
    }
    let compacted = LogEngine::inspect(&generation_path, true)?;
    if compacted.entries != baseline.entries
        || compacted.verification.recoverable_tail.is_some()
        || compacted.verification.live_keys != baseline.verification.live_keys
    {
        return invalid("bootstrap generation 1 differs from the verified legacy source before publication");
    }

    let publication = publish_generation_marker(lease.directory(), BOOTSTRAP_GENERATION)?;
    after_publication(&source)?;
    if require_clean_source(&source)? != baseline {
        return Err(LegacyLogBootstrapError::SourceChangedAfterPublication);
    }

    let final_verified = verify_generation_directory(lease.directory())?;
    if final_verified.summary().authoritative_generation != BOOTSTRAP_GENERATION {
        return Err(LegacyLogBootstrapError::FinalAuthority {
            found: final_verified.summary().authoritative_generation,
        });
    }
    let final_state = LogEngine::inspect(&final_verified.authoritative_log_path(), true)?;
    if final_state.entries != baseline.entries {
        return Err(LegacyLogBootstrapError::FinalState);
    }
    if !final_verified
        .summary()
        .reservation_generation_ids
        .contains(&BOOTSTRAP_GENERATION)
    {
        return invalid("committed bootstrap generation lost its durable generation 1 reservation");
    }

    Ok(LegacyLogBootstrapSummary {
        protocol: LEGACY_LOG_BOOTSTRAP_PROTOCOL,
        source: source.display().to_string(),
        target_directory: lease.directory().display().to_string(),
        source_verification: baseline.verification,
        generation: BOOTSTRAP_GENERATION,
        reservation,
        compaction,
        publication,
        final_generation: final_verified.summary().clone(),
    })
}

#[cfg(unix)]
fn require_clean_source(path: &Path) -> Result<InspectionReport, LegacyLogBootstrapError> {
    let inspection = LogEngine::inspect(path, true)?;
    if inspection.verification.recoverable_tail.is_some()
        || inspection.verification.file_bytes != inspection.verification.valid_bytes
    {
        return invalid(
            "legacy source must be a complete clean append-log image; recoverable tails require explicit legacy recovery before migration",
        );
    }
    Ok(inspection)
}

fn io_error(path: &Path, source: io::Error) -> LegacyLogBootstrapError {
    LegacyLogBootstrapError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, LegacyLogBootstrapError> {
    Err(LegacyLogBootstrapError::Invalid(message.into()))
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;

    use db_core::KvEngine;
    use tempfile::tempdir;

    use super::*;
    use crate::generation_directory::{canonical_marker_name, verify_generation_directory};

    #[test]
    fn source_drift_before_publication_leaves_target_uncommitted() {
        let root = tempdir().expect("temporary root");
        let source = root.path().join("legacy.db");
        let target = root.path().join("generations");
        {
            let mut engine = LogEngine::create_new(&source).expect("create legacy source");
            engine.put(b"key", b"before").expect("put legacy value");
        }

        let error = bootstrap_legacy_log_impl(
            &source,
            &target,
            |source_path| {
                let mut engine = LogEngine::open(source_path).expect("open source for injected drift");
                engine.put(b"late", b"write").expect("inject late write");
                Ok(())
            },
            |_| Ok(()),
        )
        .expect_err("source drift must stop bootstrap before marker publication");

        assert!(matches!(
            error,
            LegacyLogBootstrapError::SourceChangedBeforePublication
        ));
        assert!(target.join(canonical_generation_name(1)).is_file());
        assert!(!target.join(canonical_marker_name(1)).exists());
        assert!(target.join(crate::generation_directory::canonical_reservation_name(1)).is_file());
        assert!(verify_generation_directory(&target).is_err());
    }

    #[test]
    fn source_drift_after_publication_is_reported_without_rolling_back_target() {
        let root = tempdir().expect("temporary root");
        let source = root.path().join("legacy.db");
        let target = root.path().join("generations");
        {
            let mut engine = LogEngine::create_new(&source).expect("create legacy source");
            engine.put(b"key", b"before").expect("put legacy value");
        }

        let error = bootstrap_legacy_log_impl(
            &source,
            &target,
            |_| Ok(()),
            |source_path| {
                let mut engine = LogEngine::open(source_path).expect("open source for injected drift");
                engine.put(b"late", b"write").expect("inject late write");
                Ok(())
            },
        )
        .expect_err("post-publication drift must be surfaced explicitly");

        assert!(matches!(
            error,
            LegacyLogBootstrapError::SourceChangedAfterPublication
        ));
        let verified = verify_generation_directory(&target).expect("target remains committed");
        assert_eq!(verified.summary().authoritative_generation, 1);
    }
}
