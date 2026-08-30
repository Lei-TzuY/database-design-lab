use std::io;
use std::path::{Path, PathBuf};

use db_core::{DbError, EngineCapabilities, KvEngine, Result};
use db_storage_log::LogEngine;

use crate::generation_directory::{
    scan_generation_namespace, verify_generation_directory, GenerationDirectoryError,
};

/// Generation-directory-aware append-log engine.
///
/// The wrapper keeps ordinary mutations on the currently highest committed generation and adopts a
/// newly published higher generation before the next operation. It deliberately retains the common
/// caller-serialized concurrency contract: namespace checks prevent stale handles after a serialized
/// generation switch, but they are not a cross-process lock and cannot close a scan-to-append race
/// against a writer that violates caller serialization.
pub struct GenerationLogEngine {
    directory: PathBuf,
    authoritative_generation: u64,
    inner: LogEngine,
    poisoned: bool,
}

impl std::fmt::Debug for GenerationLogEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GenerationLogEngine")
            .field("directory", &self.directory)
            .field("authoritative_generation", &self.authoritative_generation)
            .field("authoritative_log", &self.inner.path())
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

impl GenerationLogEngine {
    /// Opens only the generation selected by the shared no-rollback recovery contract.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self> {
        let resolved = resolve_authority(directory.as_ref(), 0)?;
        Ok(Self {
            directory: resolved.directory,
            authoritative_generation: resolved.generation,
            inner: resolved.inner,
            poisoned: false,
        })
    }

    /// Canonical generation directory owned by this routing handle.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Generation id to which the current inner append-log handle is routed.
    #[must_use]
    pub const fn authoritative_generation(&self) -> u64 {
        self.authoritative_generation
    }

    /// Canonical path of the currently routed generation log.
    #[must_use]
    pub fn authoritative_log_path(&self) -> &Path {
        self.inner.path()
    }

    fn ensure_routable(&self) -> Result<()> {
        if self.poisoned {
            Err(DbError::Poisoned)
        } else {
            Ok(())
        }
    }

    fn refresh_authority(&mut self) -> Result<()> {
        self.ensure_routable()?;
        let namespace = match scan_generation_namespace(&self.directory) {
            Ok(namespace) => namespace,
            Err(error) => return self.poison(map_directory_error(error)),
        };
        let Some(highest_marker) = namespace.marker_files.keys().next_back().copied() else {
            return self.poison(corruption(
                "generation directory no longer contains a committed generation marker",
            ));
        };

        if highest_marker < self.authoritative_generation {
            return self.poison(corruption(format!(
                "generation authority regressed from {} to marker frontier {highest_marker}; rollback is forbidden",
                self.authoritative_generation
            )));
        }
        if highest_marker == self.authoritative_generation {
            return Ok(());
        }

        let resolved = match resolve_authority(&self.directory, highest_marker) {
            Ok(resolved) => resolved,
            Err(error) => return self.poison(error),
        };
        self.authoritative_generation = resolved.generation;
        self.inner = resolved.inner;
        Ok(())
    }

    fn poison<T>(&mut self, error: DbError) -> Result<T> {
        self.poisoned = true;
        Err(error)
    }

    fn reopen_routed(&mut self) -> Result<()> {
        let minimum_generation = self.authoritative_generation;
        match resolve_authority(&self.directory, minimum_generation) {
            Ok(resolved) => {
                self.authoritative_generation = resolved.generation;
                self.inner = resolved.inner;
                self.poisoned = false;
                Ok(())
            }
            Err(error) => self.poison(error),
        }
    }
}

impl KvEngine for GenerationLogEngine {
    fn capabilities(&self) -> EngineCapabilities {
        let mut capabilities = self.inner.capabilities();
        capabilities.name = "append-log-generation-v2";
        capabilities
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<Option<Vec<u8>>> {
        self.refresh_authority()?;
        self.inner.put(key, value)
    }

    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.refresh_authority()?;
        self.inner.get(key)
    }

    fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.refresh_authority()?;
        self.inner.delete(key)
    }

    fn reopen(&mut self) -> Result<()> {
        self.reopen_routed()
    }
}

struct ResolvedAuthority {
    directory: PathBuf,
    generation: u64,
    inner: LogEngine,
}

fn resolve_authority(directory: &Path, minimum_generation: u64) -> Result<ResolvedAuthority> {
    let verified = verify_generation_directory(directory).map_err(map_directory_error)?;
    let generation = verified.summary().authoritative_generation;
    if generation < minimum_generation {
        return Err(corruption(format!(
            "generation authority regressed from required minimum {minimum_generation} to {generation}; rollback is forbidden"
        )));
    }
    let canonical_directory = verified.directory().to_path_buf();
    let authoritative_log = verified.authoritative_log_path();

    // Mutable open is allowed only after the shared verifier proved the retained commit marker and
    // committed prefix. It may repair the one canonical post-prefix incomplete append.
    let inner = LogEngine::open(&authoritative_log)?;

    let after_open =
        verify_generation_directory(&canonical_directory).map_err(map_directory_error)?;
    let after_generation = after_open.summary().authoritative_generation;
    if after_generation < minimum_generation || after_generation != generation {
        return Err(corruption(format!(
            "generation authority changed while opening generation {generation}; post-open authority is {after_generation}"
        )));
    }
    if after_open.authoritative_log_path() != inner.path() {
        return Err(corruption(
            "post-open authoritative generation path differs from the opened append log",
        ));
    }

    Ok(ResolvedAuthority {
        directory: canonical_directory,
        generation,
        inner,
    })
}

fn map_directory_error(error: GenerationDirectoryError) -> DbError {
    match error {
        GenerationDirectoryError::Invalid(message) => corruption(format!(
            "generation directory verification failed: {message}"
        )),
        GenerationDirectoryError::Database(error) => error,
        GenerationDirectoryError::Io { path, source } => DbError::Io(io::Error::new(
            source.kind(),
            format!("{}: {source}", path.display()),
        )),
    }
}

fn corruption(reason: impl Into<String>) -> DbError {
    DbError::Corruption {
        offset: 0,
        reason: reason.into(),
    }
}
