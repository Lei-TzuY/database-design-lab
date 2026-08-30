use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::generation_directory::{canonical_real_directory, GenerationDirectoryError};

pub const GENERATION_WRITER_LOCK_NAME: &str = "writer.lock";
pub const GENERATION_WRITER_LOCK_PROTOCOL: &str = "append_log_generation_writer_lock_v1";

#[derive(Debug, Error)]
pub enum GenerationWriterLockError {
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error("generation writer lock is already held or stale: {path}")]
    Busy { path: PathBuf },
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Cooperative cross-process writer exclusion for one generation directory.
///
/// Acquisition uses create-new filesystem semantics. A crashed process may leave the canonical
/// lock file behind; that stale lock intentionally fails closed until an operator removes it after
/// independently confirming that no writer is alive. No PID-based or age-based lock stealing is
/// performed by this protocol.
pub struct GenerationWriterLease {
    directory: PathBuf,
    lock_path: PathBuf,
    file: Option<File>,
}

impl std::fmt::Debug for GenerationWriterLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GenerationWriterLease")
            .field("directory", &self.directory)
            .field("lock_path", &self.lock_path)
            .finish_non_exhaustive()
    }
}

impl GenerationWriterLease {
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

impl Drop for GenerationWriterLease {
    fn drop(&mut self) {
        let _ = self.file.take();
        let _ = fs::remove_file(&self.lock_path);
    }
}

pub fn acquire_generation_writer_lease(
    directory: &Path,
) -> Result<GenerationWriterLease, GenerationWriterLockError> {
    let directory = canonical_real_directory(directory)?;
    let lock_path = directory.join(GENERATION_WRITER_LOCK_NAME);
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            return Err(GenerationWriterLockError::Busy { path: lock_path });
        }
        Err(source) => {
            return Err(GenerationWriterLockError::Io {
                path: lock_path,
                source,
            });
        }
    };

    let owner_record = format!(
        "protocol={GENERATION_WRITER_LOCK_PROTOCOL}\npid={}\n",
        std::process::id()
    );
    if let Err(source) = file.write_all(owner_record.as_bytes()) {
        drop(file);
        let _ = fs::remove_file(&lock_path);
        return Err(GenerationWriterLockError::Io {
            path: lock_path,
            source,
        });
    }

    Ok(GenerationWriterLease {
        directory,
        lock_path,
        file: Some(file),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::generation_directory::scan_generation_namespace;

    #[test]
    fn lease_is_exclusive_and_released_on_drop() {
        let root = tempdir().expect("temporary root");
        let directory = root.path().join("generations");
        fs::create_dir(&directory).expect("create generation directory");

        let first = acquire_generation_writer_lease(&directory).expect("acquire first lease");
        assert!(first.lock_path().is_file());
        assert!(matches!(
            acquire_generation_writer_lease(&directory),
            Err(GenerationWriterLockError::Busy { .. })
        ));
        drop(first);
        assert!(!directory.join(GENERATION_WRITER_LOCK_NAME).exists());

        let second = acquire_generation_writer_lease(&directory).expect("acquire after release");
        drop(second);
    }

    #[test]
    fn namespace_scan_accepts_live_writer_lock() {
        let root = tempdir().expect("temporary root");
        let directory = root.path().join("generations");
        fs::create_dir(&directory).expect("create generation directory");

        let lease = acquire_generation_writer_lease(&directory).expect("acquire lease");
        scan_generation_namespace(lease.directory()).expect("scan with live writer lock");
        drop(lease);
    }

    #[test]
    fn stale_lock_fails_closed_without_lock_stealing() {
        let root = tempdir().expect("temporary root");
        let directory = root.path().join("generations");
        fs::create_dir(&directory).expect("create generation directory");
        fs::write(directory.join(GENERATION_WRITER_LOCK_NAME), b"stale")
            .expect("write stale lock");

        assert!(matches!(
            acquire_generation_writer_lease(&directory),
            Err(GenerationWriterLockError::Busy { .. })
        ));
        assert_eq!(
            fs::read(directory.join(GENERATION_WRITER_LOCK_NAME)).expect("read stale lock"),
            b"stale",
            "acquisition must not rewrite or steal stale lock evidence"
        );
    }
}
