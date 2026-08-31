use std::io;
use std::path::{Path, PathBuf};

use db_core::DbError;
use serde::Serialize;
use thiserror::Error;

use crate::generation_directory::GenerationDirectoryError;

pub const LEGACY_CUTOVER_RECEIPT_PROTOCOL: &str = "append_log_legacy_cutover_receipt_v1";
pub const LEGACY_CUTOVER_RECEIPT_VERSION: u16 = 1;
pub const MAX_LEGACY_SOURCE_PATH_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LegacySourceFingerprint {
    pub file_bytes: u64,
    pub crc32: u32,
    pub record_count: u64,
    pub next_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyCutoverReceiptSummary {
    pub protocol: &'static str,
    pub receipt_version: u16,
    pub receipt_path: String,
    pub source_path: String,
    pub source: LegacySourceFingerprint,
}

#[derive(Debug, Error)]
pub enum GenerationCutoverError {
    #[error("legacy cutover receipts are unsupported on this platform")]
    UnsupportedPlatform,
    #[error("invalid legacy cutover receipt: {0}")]
    Invalid(String),
    #[error(transparent)]
    Directory(#[from] GenerationDirectoryError),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "legacy cutover receipt {receipt} is visible but parent-directory durability could not be confirmed: {source}"
    )]
    DurabilityUncertain {
        receipt: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "legacy source {source} no longer matches its cutover receipt; generation-aware operations must stop to avoid split-brain"
    )]
    SourceDrift { source: PathBuf },
}

/// Captures the exact clean legacy source fingerprint used by a migration cutover receipt.
pub fn fingerprint_legacy_source(
    source: &Path,
) -> Result<LegacySourceFingerprint, GenerationCutoverError> {
    #[cfg(unix)]
    {
        unix::fingerprint_legacy_source(source)
    }

    #[cfg(not(unix))]
    {
        let _ = source;
        Err(GenerationCutoverError::UnsupportedPlatform)
    }
}

/// Publishes a durable sibling receipt binding a generation directory to the exact retained legacy
/// source that was imported. The expected fingerprint must have been captured before target commit.
pub fn publish_legacy_cutover_receipt(
    directory: &Path,
    source: &Path,
    expected: LegacySourceFingerprint,
) -> Result<LegacyCutoverReceiptSummary, GenerationCutoverError> {
    #[cfg(unix)]
    {
        unix::publish_legacy_cutover_receipt(directory, source, expected)
    }

    #[cfg(not(unix))]
    {
        let _ = (directory, source, expected);
        Err(GenerationCutoverError::UnsupportedPlatform)
    }
}

/// Verifies an optional sibling cutover receipt and the retained legacy source it binds.
///
/// Generation directories without a receipt remain valid. On Unix, any present receipt is strict:
/// malformed bytes, a missing/replaced legacy source, or fingerprint drift fail closed.
pub fn verify_legacy_cutover_receipt(
    directory: &Path,
) -> Result<Option<LegacyCutoverReceiptSummary>, GenerationCutoverError> {
    #[cfg(unix)]
    {
        unix::verify_legacy_cutover_receipt(directory)
    }

    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(None)
    }
}

#[cfg(unix)]
mod unix {
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    use db_storage_log::LogEngine;

    use super::*;
    use crate::generation_directory::canonical_real_directory;
    use crate::generation_marker::Crc32Ieee;

    const RECEIPT_MAGIC: [u8; 8] = *b"DBLCUT01";
    const RECEIPT_HEADER_LEN: usize = 48;
    const RECEIPT_TRAILER_LEN: usize = 4;
    const CRC_BUFFER_BYTES: usize = 64 * 1024;

    struct DecodedReceipt {
        source_path: PathBuf,
        source: LegacySourceFingerprint,
    }

    pub(super) fn fingerprint_legacy_source(
        source: &Path,
    ) -> Result<LegacySourceFingerprint, GenerationCutoverError> {
        let source = canonical_legacy_source(source)?;
        let verification = LogEngine::verify(&source)?;
        if verification.recoverable_tail.is_some()
            || verification.file_bytes != verification.valid_bytes
        {
            return invalid(
                "legacy source must be a complete clean append-log image before cutover receipt publication",
            );
        }
        let crc32 = crc32_file(&source, verification.file_bytes)?;
        Ok(LegacySourceFingerprint {
            file_bytes: verification.file_bytes,
            crc32,
            record_count: verification.record_count,
            next_sequence: verification.next_sequence,
        })
    }

    pub(super) fn publish_legacy_cutover_receipt(
        directory: &Path,
        source: &Path,
        expected: LegacySourceFingerprint,
    ) -> Result<LegacyCutoverReceiptSummary, GenerationCutoverError> {
        let directory = canonical_real_directory(directory)?;
        let source = canonical_legacy_source(source)?;
        if fingerprint_legacy_source(&source)? != expected {
            return Err(GenerationCutoverError::SourceDrift { source });
        }

        let receipt_path = receipt_path(&directory)?;
        let encoded = encode_receipt(&source, expected)?;
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&receipt_path)
        {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                return invalid(format!(
                    "cutover receipt already exists: {}",
                    receipt_path.display()
                ));
            }
            Err(source) => return Err(io_error(&receipt_path, source)),
        };
        if let Err(source) = file.write_all(&encoded).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&receipt_path);
            return Err(io_error(&receipt_path, source));
        }
        drop(file);

        let parent = directory.parent().ok_or_else(|| {
            GenerationCutoverError::Invalid(format!(
                "generation directory has no parent for cutover receipt durability: {}",
                directory.display()
            ))
        })?;
        if let Err(source) = File::open(parent).and_then(|file| file.sync_all()) {
            return Err(GenerationCutoverError::DurabilityUncertain {
                receipt: receipt_path,
                source,
            });
        }

        let summary = verify_legacy_cutover_receipt(&directory)?.ok_or_else(|| {
            GenerationCutoverError::Invalid(
                "published cutover receipt disappeared during verification".to_owned(),
            )
        })?;
        if summary.source != expected || canonical_legacy_source(&source)? != source {
            return Err(GenerationCutoverError::SourceDrift { source });
        }
        Ok(summary)
    }

    pub(super) fn verify_legacy_cutover_receipt(
        directory: &Path,
    ) -> Result<Option<LegacyCutoverReceiptSummary>, GenerationCutoverError> {
        let directory = canonical_real_directory(directory)?;
        let receipt_path = receipt_path(&directory)?;
        let metadata = match fs::symlink_metadata(&receipt_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io_error(&receipt_path, source)),
            Ok(metadata) => metadata,
        };
        if !metadata.file_type().is_file() {
            return invalid(format!(
                "cutover receipt must be a real regular file rather than a symlink or non-file: {}",
                receipt_path.display()
            ));
        }
        let maximum = RECEIPT_HEADER_LEN
            .checked_add(MAX_LEGACY_SOURCE_PATH_BYTES)
            .and_then(|len| len.checked_add(RECEIPT_TRAILER_LEN))
            .expect("receipt size constants fit usize");
        if metadata.len() > maximum as u64 {
            return invalid(format!(
                "cutover receipt has {} bytes, maximum is {maximum}",
                metadata.len()
            ));
        }
        let bytes = fs::read(&receipt_path).map_err(|source| io_error(&receipt_path, source))?;
        let decoded = decode_receipt(&bytes)?;
        let source_path = canonical_legacy_source(&decoded.source_path).map_err(|error| match error {
            GenerationCutoverError::Io { .. } | GenerationCutoverError::Invalid(_) => {
                GenerationCutoverError::SourceDrift {
                    source: decoded.source_path.clone(),
                }
            }
            other => other,
        })?;
        let current = fingerprint_legacy_source(&source_path)?;
        if current != decoded.source {
            return Err(GenerationCutoverError::SourceDrift {
                source: source_path,
            });
        }
        Ok(Some(LegacyCutoverReceiptSummary {
            protocol: LEGACY_CUTOVER_RECEIPT_PROTOCOL,
            receipt_version: LEGACY_CUTOVER_RECEIPT_VERSION,
            receipt_path: receipt_path.to_string_lossy().into_owned(),
            source_path: source_path.to_string_lossy().into_owned(),
            source: current,
        }))
    }

    fn canonical_legacy_source(path: &Path) -> Result<PathBuf, GenerationCutoverError> {
        let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
        if !metadata.file_type().is_file() {
            return invalid(format!(
                "legacy source must be a real regular file rather than a symlink or non-file: {}",
                path.display()
            ));
        }
        fs::canonicalize(path).map_err(|source| io_error(path, source))
    }

    fn receipt_path(directory: &Path) -> Result<PathBuf, GenerationCutoverError> {
        let name = directory.file_name().ok_or_else(|| {
            GenerationCutoverError::Invalid(format!(
                "generation directory has no cutover-receipt final component: {}",
                directory.display()
            ))
        })?;
        let mut receipt_name = OsString::from(".");
        receipt_name.push(name);
        receipt_name.push(".append-log-legacy-cutover.receipt");
        Ok(directory.with_file_name(receipt_name))
    }

    fn encode_receipt(
        source: &Path,
        fingerprint: LegacySourceFingerprint,
    ) -> Result<Vec<u8>, GenerationCutoverError> {
        let path_bytes = source.as_os_str().as_bytes();
        if path_bytes.is_empty() || path_bytes.len() > MAX_LEGACY_SOURCE_PATH_BYTES {
            return invalid(format!(
                "canonical legacy source path must contain 1..={MAX_LEGACY_SOURCE_PATH_BYTES} bytes"
            ));
        }
        let path_len = u32::try_from(path_bytes.len())
            .map_err(|_| GenerationCutoverError::Invalid("source path length does not fit u32".to_owned()))?;
        let capacity = RECEIPT_HEADER_LEN
            .checked_add(path_bytes.len())
            .and_then(|len| len.checked_add(RECEIPT_TRAILER_LEN))
            .ok_or_else(|| GenerationCutoverError::Invalid("cutover receipt length overflowed usize".to_owned()))?;
        let mut bytes = Vec::with_capacity(capacity);
        bytes.extend_from_slice(&RECEIPT_MAGIC);
        bytes.extend_from_slice(&LEGACY_CUTOVER_RECEIPT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(RECEIPT_HEADER_LEN as u16).to_le_bytes());
        bytes.extend_from_slice(&path_len.to_le_bytes());
        bytes.extend_from_slice(&fingerprint.file_bytes.to_le_bytes());
        bytes.extend_from_slice(&fingerprint.crc32.to_le_bytes());
        bytes.extend_from_slice(&fingerprint.record_count.to_le_bytes());
        bytes.extend_from_slice(&fingerprint.next_sequence.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        debug_assert_eq!(bytes.len(), RECEIPT_HEADER_LEN);
        bytes.extend_from_slice(path_bytes);
        let crc = crc32_bytes(&bytes);
        bytes.extend_from_slice(&crc.to_le_bytes());
        Ok(bytes)
    }

    fn decode_receipt(bytes: &[u8]) -> Result<DecodedReceipt, GenerationCutoverError> {
        if bytes.len() < RECEIPT_HEADER_LEN + RECEIPT_TRAILER_LEN {
            return invalid("cutover receipt is truncated");
        }
        if bytes[..8] != RECEIPT_MAGIC {
            return invalid("cutover receipt magic mismatch");
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != LEGACY_CUTOVER_RECEIPT_VERSION {
            return invalid(format!(
                "cutover receipt version {version} is unsupported; expected {LEGACY_CUTOVER_RECEIPT_VERSION}"
            ));
        }
        let header_len = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
        if header_len != RECEIPT_HEADER_LEN {
            return invalid(format!(
                "cutover receipt header length {header_len} is non-canonical"
            ));
        }
        let path_len = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice")) as usize;
        if path_len == 0 || path_len > MAX_LEGACY_SOURCE_PATH_BYTES {
            return invalid(format!(
                "cutover receipt source path length {path_len} is outside 1..={MAX_LEGACY_SOURCE_PATH_BYTES}"
            ));
        }
        let expected_len = RECEIPT_HEADER_LEN
            .checked_add(path_len)
            .and_then(|len| len.checked_add(RECEIPT_TRAILER_LEN))
            .ok_or_else(|| GenerationCutoverError::Invalid("cutover receipt length overflowed usize".to_owned()))?;
        if bytes.len() != expected_len {
            return invalid(format!(
                "cutover receipt has {} bytes, expected {expected_len}",
                bytes.len()
            ));
        }
        if bytes[44..48] != [0_u8; 4] {
            return invalid("cutover receipt reserved field is nonzero");
        }
        let stored_crc = u32::from_le_bytes(
            bytes[expected_len - 4..]
                .try_into()
                .expect("fixed CRC trailer"),
        );
        let computed_crc = crc32_bytes(&bytes[..expected_len - 4]);
        if stored_crc != computed_crc {
            return invalid(format!(
                "cutover receipt checksum mismatch: stored {stored_crc:#010x}, computed {computed_crc:#010x}"
            ));
        }
        let source_path = PathBuf::from(OsString::from_vec(
            bytes[RECEIPT_HEADER_LEN..RECEIPT_HEADER_LEN + path_len].to_vec(),
        ));
        if !source_path.is_absolute() {
            return invalid("cutover receipt legacy source path must be absolute");
        }
        Ok(DecodedReceipt {
            source_path,
            source: LegacySourceFingerprint {
                file_bytes: u64::from_le_bytes(bytes[16..24].try_into().expect("fixed slice")),
                crc32: u32::from_le_bytes(bytes[24..28].try_into().expect("fixed slice")),
                record_count: u64::from_le_bytes(bytes[28..36].try_into().expect("fixed slice")),
                next_sequence: u64::from_le_bytes(bytes[36..44].try_into().expect("fixed slice")),
            },
        })
    }

    fn crc32_file(path: &Path, expected_bytes: u64) -> Result<u32, GenerationCutoverError> {
        let mut file = File::open(path).map_err(|source| io_error(path, source))?;
        let mut remaining = expected_bytes;
        let mut hasher = Crc32Ieee::new();
        let mut buffer = [0_u8; CRC_BUFFER_BYTES];
        while remaining > 0 {
            let wanted = usize::try_from(remaining.min(CRC_BUFFER_BYTES as u64))
                .expect("CRC chunk is bounded by a usize-sized constant");
            let read = file
                .read(&mut buffer[..wanted])
                .map_err(|source| io_error(path, source))?;
            if read == 0 {
                return Err(GenerationCutoverError::SourceDrift {
                    source: path.to_path_buf(),
                });
            }
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }
        let mut extra = [0_u8; 1];
        if file.read(&mut extra).map_err(|source| io_error(path, source))? != 0 {
            return Err(GenerationCutoverError::SourceDrift {
                source: path.to_path_buf(),
            });
        }
        Ok(hasher.finalize())
    }

    fn crc32_bytes(bytes: &[u8]) -> u32 {
        let mut hasher = Crc32Ieee::new();
        hasher.update(bytes);
        hasher.finalize()
    }

    fn io_error(path: &Path, source: io::Error) -> GenerationCutoverError {
        GenerationCutoverError::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    fn invalid<T>(message: impl Into<String>) -> Result<T, GenerationCutoverError> {
        Err(GenerationCutoverError::Invalid(message.into()))
    }

    #[cfg(test)]
    mod tests {
        use db_core::KvEngine;
        use tempfile::tempdir;

        use super::*;

        #[test]
        fn receipt_round_trips_non_utf8_source_path() {
            let root = tempdir().expect("temporary root");
            let source_name = OsStr::from_bytes(b"legacy-\xff.db");
            let source = root.path().join(source_name);
            let directory = root.path().join("generations");
            fs::create_dir(&directory).expect("create generation directory");
            let mut engine = LogEngine::create_new(&source).expect("create source");
            engine.put(b"a", b"one").expect("put value");
            drop(engine);

            let fingerprint = fingerprint_legacy_source(&source).expect("fingerprint source");
            let summary = publish_legacy_cutover_receipt(&directory, &source, fingerprint)
                .expect("publish receipt");
            assert_eq!(summary.source, fingerprint);
            let verified = verify_legacy_cutover_receipt(&directory)
                .expect("verify receipt")
                .expect("receipt present");
            assert_eq!(verified.source, fingerprint);
        }
    }
}
