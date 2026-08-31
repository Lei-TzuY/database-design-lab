use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

#[cfg(unix)]
use crate::generation_directory::{canonical_reservation_name, verify_generation_directory};
use crate::generation_directory::GenerationDirectoryError;
#[cfg(unix)]
use crate::generation_lock::acquire_generation_writer_lease;
use crate::generation_lock::GenerationWriterLockError;

pub const GENERATION_RESERVATION_PROTOCOL: &str = "append_log_generation_reservation_unix_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerationReservationSummary {
    pub protocol: &'static str,
    pub generation: u64,
    pub reservation: String,
    pub authoritative_generation: u64,
    pub highest_observed_generation: u64,
}

#[derive(Debug, Error)]
pub enum GenerationReservationError {
    #[error("append-log generation reservation is unsupported on this platform; no reservation was written")]
    UnsupportedPlatform,
    #[error(transparent)]
    Lock(#[from] GenerationWriterLockError),
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "generation reservation {generation} is visible but parent-directory durability could not be confirmed at {directory}: {source}"
    )]
    DurabilityUncertain {
        generation: u64,
        directory: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("generation reservation {generation} was not retained by post-write verification")]
    NotRetained { generation: u64 },
}

/// Durably reserves the next generation id before a candidate file is constructed.
///
/// Unix uses the shared generation writer lease so concurrent cooperative allocators cannot choose
/// the same id. The zero-byte reservation is synchronized before the generation-directory entry is
/// synchronized. Once this function reports success, later cleanup may remove a candidate/staging
/// artifact for the same id without allowing that id to be allocated again.
///
/// Non-Unix targets fail before filesystem access because this repository does not yet claim an
/// equivalent parent-directory durability barrier there.
pub fn reserve_next_generation(
    directory: &Path,
) -> Result<GenerationReservationSummary, GenerationReservationError> {
    #[cfg(unix)]
    {
        reserve_next_generation_unix(directory)
    }

    #[cfg(not(unix))]
    {
        let _ = directory;
        Err(GenerationReservationError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
fn reserve_next_generation_unix(
    directory: &Path,
) -> Result<GenerationReservationSummary, GenerationReservationError> {
    use std::fs::{File, OpenOptions};

    let lease = acquire_generation_writer_lease(directory)?;
    let before = verify_generation_directory(lease.directory())?;
    let generation = before.next_generation_id()?;
    let reservation = canonical_reservation_name(generation);
    let reservation_path = lease.directory().join(&reservation);

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&reservation_path)
        .map_err(|source| io_error(&reservation_path, source))?;
    file.sync_all()
        .map_err(|source| io_error(&reservation_path, source))?;

    if let Err(source) = File::open(lease.directory()).and_then(|directory| directory.sync_all()) {
        return Err(GenerationReservationError::DurabilityUncertain {
            generation,
            directory: lease.directory().to_path_buf(),
            source,
        });
    }

    let after = verify_generation_directory(lease.directory())?;
    if !after.summary().reservation_generation_ids.contains(&generation)
        || after.summary().highest_observed_generation < generation
    {
        return Err(GenerationReservationError::NotRetained { generation });
    }

    Ok(GenerationReservationSummary {
        protocol: GENERATION_RESERVATION_PROTOCOL,
        generation,
        reservation,
        authoritative_generation: after.summary().authoritative_generation,
        highest_observed_generation: after.summary().highest_observed_generation,
    })
}

#[cfg(unix)]
fn io_error(path: &Path, source: io::Error) -> GenerationReservationError {
    GenerationReservationError::Io {
        path: path.to_path_buf(),
        source,
    }
}
