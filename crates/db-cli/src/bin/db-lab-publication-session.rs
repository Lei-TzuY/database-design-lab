use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use db_cli::batch_archive::{verify_batch_archive, VerificationSummary, VerifyError};
use db_cli::host_preflight::{
    load_verified_host_preflight_snapshot, HostPreflightSnapshot, HostPreflightVerifyError,
    HOST_PREFLIGHT_PROTOCOL,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const SESSION_FORMAT_VERSION: u16 = 1;
const SESSION_PROTOCOL: &str = "controlled_publication_session_v1";
const PUBLICATION_ADMISSION_PROTOCOL: &str = "publication_warm_v1";
const MAX_SESSION_FILE_BYTES: u64 = 64 * 1024 * 1024;
const PREFLIGHT_FILE: &str = "host-preflight.json";
const EVIDENCE_DIRECTORY: &str = "evidence";
const INDEX_FILE: &str = "index.json";

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-publication-session",
    version,
    about = "Create or verify a self-contained controlled publication-session artifact"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Bind one passed host preflight to one publication-admitted repeated-batch archive.
    Create {
        /// Existing verified host-preflight snapshot.
        #[arg(long)]
        host_preflight: PathBuf,
        /// Existing immutable publication repeated-batch archive.
        #[arg(long)]
        archive_dir: PathBuf,
        /// Fresh destination directory for the publication session.
        #[arg(long)]
        session_dir: PathBuf,
        /// Optional exact repository revision expected by the caller.
        #[arg(long)]
        expected_revision: Option<String>,
    },
    /// Re-verify all bindings inside one retained publication session.
    Verify {
        /// Existing publication-session directory.
        #[arg(long)]
        session_dir: PathBuf,
        /// Optional exact repository revision expected by the caller.
        #[arg(long)]
        expected_revision: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationSessionIndex {
    format_version: u16,
    session_protocol: String,
    host_preflight_protocol: String,
    publication_admission_protocol: String,
    host_label: String,
    preflight_recorded_unix_seconds: u64,
    repository_revision: String,
    source_archive_format_version: u16,
    evidence_files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PublicationSessionVerificationSummary {
    valid: bool,
    session_format_version: u16,
    session_protocol: &'static str,
    host_label: String,
    preflight_recorded_unix_seconds: u64,
    repository_revision: String,
    source_archive_format_version: u16,
    evidence_files: usize,
}

#[derive(Debug, Error)]
enum SessionError {
    #[error(transparent)]
    Archive(#[from] VerifyError),
    #[error(transparent)]
    Preflight(#[from] HostPreflightVerifyError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid JSON at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid publication session: {0}")]
    Invalid(String),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Create {
            host_preflight,
            archive_dir,
            session_dir,
            expected_revision,
        } => create_session(
            &host_preflight,
            &archive_dir,
            &session_dir,
            expected_revision.as_deref(),
        ),
        Command::Verify {
            session_dir,
            expected_revision,
        } => verify_session(&session_dir, expected_revision.as_deref()),
    };

    match result {
        Ok(summary) => match serde_json::to_string_pretty(&summary) {
            Ok(encoded) => {
                println!("{encoded}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: failed to encode session verification summary: {error}");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn create_session(
    host_preflight: &Path,
    archive_dir: &Path,
    session_dir: &Path,
    expected_revision: Option<&str>,
) -> Result<PublicationSessionVerificationSummary, SessionError> {
    let source_preflight = canonical_existing_regular_file(host_preflight, "host preflight")?;
    let source_archive = canonical_existing_real_directory(archive_dir, "source archive")?;
    let target_dir = canonical_fresh_target(session_dir)?;
    reject_nested_paths(&source_archive, &target_dir)?;

    let source_preflight_value =
        load_verified_host_preflight_snapshot(&source_preflight, None, true)?;
    let source_archive_verification =
        verify_batch_archive(&source_archive, expected_revision, true)?;
    let source_host_label = publication_host_label(&source_archive)?;
    require_matching_host_label(&source_preflight_value, &source_host_label)?;

    fs::create_dir(&target_dir).map_err(|source| SessionError::Io {
        path: target_dir.clone(),
        source,
    })?;

    let result = (|| {
        let copied_preflight = target_dir.join(PREFLIGHT_FILE);
        copy_regular_file(&source_preflight, &copied_preflight)?;

        let evidence_dir = target_dir.join(EVIDENCE_DIRECTORY);
        fs::create_dir(&evidence_dir).map_err(|source| SessionError::Io {
            path: evidence_dir.clone(),
            source,
        })?;
        let source_files = directory_entry_names(&source_archive, "source archive")?;
        for name in &source_files {
            copy_regular_file(&source_archive.join(name), &evidence_dir.join(name))?;
        }

        let copied_preflight_value =
            load_verified_host_preflight_snapshot(&copied_preflight, Some(&source_host_label), true)?;
        if copied_preflight_value != source_preflight_value {
            return Err(SessionError::Invalid(
                "host-preflight value changed while it was copied".to_owned(),
            ));
        }

        let copied_archive_verification = verify_batch_archive(
            &evidence_dir,
            Some(&source_archive_verification.repository_revision),
            true,
        )?;
        if copied_archive_verification != source_archive_verification {
            return Err(SessionError::Invalid(
                "archive verification summary changed while evidence was copied".to_owned(),
            ));
        }
        let copied_host_label = publication_host_label(&evidence_dir)?;
        if copied_host_label != source_host_label {
            return Err(SessionError::Invalid(format!(
                "publication host label changed while evidence was copied: source {source_host_label:?}, copy {copied_host_label:?}"
            )));
        }

        let source_preflight_after =
            load_verified_host_preflight_snapshot(&source_preflight, Some(&source_host_label), true)?;
        if source_preflight_after != source_preflight_value {
            return Err(SessionError::Invalid(
                "source host-preflight changed while the session was being created".to_owned(),
            ));
        }
        let source_archive_after = verify_batch_archive(
            &source_archive,
            Some(&source_archive_verification.repository_revision),
            true,
        )?;
        if source_archive_after != source_archive_verification {
            return Err(SessionError::Invalid(
                "source archive verification summary changed while the session was being created"
                    .to_owned(),
            ));
        }
        if publication_host_label(&source_archive)? != source_host_label {
            return Err(SessionError::Invalid(
                "source publication host label changed while the session was being created".to_owned(),
            ));
        }
        compare_regular_files(&source_preflight, &copied_preflight, PREFLIGHT_FILE)?;
        verify_directories_match(&source_archive, &evidence_dir)?;

        let index = PublicationSessionIndex {
            format_version: SESSION_FORMAT_VERSION,
            session_protocol: SESSION_PROTOCOL.to_owned(),
            host_preflight_protocol: HOST_PREFLIGHT_PROTOCOL.to_owned(),
            publication_admission_protocol: PUBLICATION_ADMISSION_PROTOCOL.to_owned(),
            host_label: source_host_label.clone(),
            preflight_recorded_unix_seconds: source_preflight_value.recorded_unix_seconds,
            repository_revision: source_archive_verification.repository_revision.clone(),
            source_archive_format_version: source_archive_verification.format_version,
            evidence_files: source_files.into_iter().collect(),
        };
        write_new_json(&target_dir.join(INDEX_FILE), &index)?;

        verify_session(
            &target_dir,
            Some(&source_archive_verification.repository_revision),
        )
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&target_dir);
    }
    result
}

fn verify_session(
    session_dir: &Path,
    expected_revision: Option<&str>,
) -> Result<PublicationSessionVerificationSummary, SessionError> {
    let session_dir = canonical_existing_real_directory(session_dir, "publication session")?;
    require_exact_session_entries(&session_dir)?;

    let index_path = session_dir.join(INDEX_FILE);
    let index: PublicationSessionIndex =
        serde_json::from_value(read_json(&index_path)?).map_err(|source| SessionError::Json {
            path: index_path.clone(),
            source,
        })?;
    validate_index(&index, expected_revision)?;

    let preflight = load_verified_host_preflight_snapshot(
        &session_dir.join(PREFLIGHT_FILE),
        Some(&index.host_label),
        true,
    )?;
    if preflight.recorded_unix_seconds != index.preflight_recorded_unix_seconds {
        return Err(SessionError::Invalid(format!(
            "host-preflight recording time differs from index.json: index {}, verified {}",
            index.preflight_recorded_unix_seconds, preflight.recorded_unix_seconds
        )));
    }

    let evidence_dir = session_dir.join(EVIDENCE_DIRECTORY);
    let actual_evidence_files: Vec<String> =
        directory_entry_names(&evidence_dir, "publication evidence")?
            .into_iter()
            .collect();
    if actual_evidence_files != index.evidence_files {
        return Err(SessionError::Invalid(format!(
            "publication evidence file set differs from index.json: index {:?}, actual {:?}",
            index.evidence_files, actual_evidence_files
        )));
    }

    let archive = verify_batch_archive(&evidence_dir, Some(&index.repository_revision), true)?;
    if archive.format_version != index.source_archive_format_version {
        return Err(SessionError::Invalid(format!(
            "source archive format differs from index.json: index {}, verified {}",
            index.source_archive_format_version, archive.format_version
        )));
    }
    let archive_host_label = publication_host_label(&evidence_dir)?;
    if archive_host_label != index.host_label {
        return Err(SessionError::Invalid(format!(
            "publication archive host label {archive_host_label:?} differs from session host label {:?}",
            index.host_label
        )));
    }
    require_matching_host_label(&preflight, &archive_host_label)?;

    Ok(PublicationSessionVerificationSummary {
        valid: true,
        session_format_version: SESSION_FORMAT_VERSION,
        session_protocol: SESSION_PROTOCOL,
        host_label: index.host_label,
        preflight_recorded_unix_seconds: index.preflight_recorded_unix_seconds,
        repository_revision: archive.repository_revision,
        source_archive_format_version: archive.format_version,
        evidence_files: index.evidence_files.len(),
    })
}

fn validate_index(
    index: &PublicationSessionIndex,
    expected_revision: Option<&str>,
) -> Result<(), SessionError> {
    if index.format_version != SESSION_FORMAT_VERSION {
        return Err(SessionError::Invalid(format!(
            "unsupported publication-session format {}; expected {SESSION_FORMAT_VERSION}",
            index.format_version
        )));
    }
    if index.session_protocol != SESSION_PROTOCOL {
        return Err(SessionError::Invalid(format!(
            "unsupported publication-session protocol {:?}",
            index.session_protocol
        )));
    }
    if index.host_preflight_protocol != HOST_PREFLIGHT_PROTOCOL {
        return Err(SessionError::Invalid(format!(
            "host-preflight protocol differs from verifier: index {:?}, expected {HOST_PREFLIGHT_PROTOCOL:?}",
            index.host_preflight_protocol
        )));
    }
    if index.publication_admission_protocol != PUBLICATION_ADMISSION_PROTOCOL {
        return Err(SessionError::Invalid(format!(
            "publication admission protocol differs from required protocol: index {:?}, expected {PUBLICATION_ADMISSION_PROTOCOL:?}",
            index.publication_admission_protocol
        )));
    }
    if index.host_label.trim().is_empty() {
        return Err(SessionError::Invalid(
            "index.json host_label must be non-empty".to_owned(),
        ));
    }
    if index.preflight_recorded_unix_seconds == 0 {
        return Err(SessionError::Invalid(
            "index.json preflight_recorded_unix_seconds must be greater than zero".to_owned(),
        ));
    }
    if index.repository_revision.trim().is_empty() {
        return Err(SessionError::Invalid(
            "index.json repository_revision must be non-empty".to_owned(),
        ));
    }
    if let Some(expected) = expected_revision {
        if index.repository_revision != expected {
            return Err(SessionError::Invalid(format!(
                "session repository revision {:?} differs from expected revision {expected:?}",
                index.repository_revision
            )));
        }
    }
    if !matches!(index.source_archive_format_version, 7 | 11) {
        return Err(SessionError::Invalid(format!(
            "publication session requires source archive format v7 or v11; index records v{}",
            index.source_archive_format_version
        )));
    }
    if index.evidence_files.is_empty() {
        return Err(SessionError::Invalid(
            "index.json evidence_files must not be empty".to_owned(),
        ));
    }
    let sorted: Vec<&str> = index.evidence_files.iter().map(String::as_str).collect();
    let unique: BTreeSet<&str> = sorted.iter().copied().collect();
    if unique.len() != sorted.len() || unique.iter().copied().collect::<Vec<_>>() != sorted {
        return Err(SessionError::Invalid(
            "index.json evidence_files must be sorted and duplicate-free".to_owned(),
        ));
    }
    Ok(())
}

fn publication_host_label(archive_dir: &Path) -> Result<String, SessionError> {
    let environment = read_json(&archive_dir.join("environment.json"))?;
    let admission = environment
        .get("publication_admission")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SessionError::Invalid(
                "verified publication archive is missing publication_admission object".to_owned(),
            )
        })?;
    let host_label = admission
        .get("host_label")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SessionError::Invalid(
                "verified publication archive is missing publication_admission.host_label"
                    .to_owned(),
            )
        })?;
    Ok(host_label.to_owned())
}

fn require_matching_host_label(
    preflight: &HostPreflightSnapshot,
    archive_host_label: &str,
) -> Result<(), SessionError> {
    if preflight.host_label != archive_host_label {
        return Err(SessionError::Invalid(format!(
            "host-preflight label {:?} differs from publication archive host label {archive_host_label:?}",
            preflight.host_label
        )));
    }
    Ok(())
}

fn canonical_existing_regular_file(path: &Path, label: &str) -> Result<PathBuf, SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(SessionError::Invalid(format!(
            "{label} must be a regular file rather than a symlink or non-file"
        )));
    }
    fs::canonicalize(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn canonical_existing_real_directory(path: &Path, label: &str) -> Result<PathBuf, SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_dir() {
        return Err(SessionError::Invalid(format!(
            "{label} must be a real directory rather than a symlink or non-directory"
        )));
    }
    fs::canonicalize(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn canonical_fresh_target(path: &Path) -> Result<PathBuf, SessionError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(SessionError::Invalid(format!(
                "publication-session destination already exists: {}",
                path.display()
            )))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(SessionError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    }
    let name = path.file_name().ok_or_else(|| {
        SessionError::Invalid(format!(
            "publication-session destination has no final path component: {}",
            path.display()
        ))
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|source| SessionError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    Ok(parent.join(name))
}

fn reject_nested_paths(source_dir: &Path, target_dir: &Path) -> Result<(), SessionError> {
    if source_dir == target_dir
        || source_dir.starts_with(target_dir)
        || target_dir.starts_with(source_dir)
    {
        return Err(SessionError::Invalid(
            "source archive and publication session must be distinct, non-nested paths".to_owned(),
        ));
    }
    Ok(())
}

fn require_exact_session_entries(session_dir: &Path) -> Result<(), SessionError> {
    let actual = directory_entry_names(session_dir, "publication session")?;
    let expected: BTreeSet<String> = [PREFLIGHT_FILE, EVIDENCE_DIRECTORY, INDEX_FILE]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if actual != expected {
        return Err(SessionError::Invalid(format!(
            "publication-session entries must be exactly {expected:?}; found {actual:?}"
        )));
    }
    require_regular_file(&session_dir.join(PREFLIGHT_FILE), PREFLIGHT_FILE)?;
    require_regular_file(&session_dir.join(INDEX_FILE), INDEX_FILE)?;
    let evidence_path = session_dir.join(EVIDENCE_DIRECTORY);
    let metadata = fs::symlink_metadata(&evidence_path).map_err(|source| SessionError::Io {
        path: evidence_path,
        source,
    })?;
    if !metadata.file_type().is_dir() {
        return Err(SessionError::Invalid(
            "publication-session evidence must be a real directory".to_owned(),
        ));
    }
    Ok(())
}

fn require_regular_file(path: &Path, label: &str) -> Result<(), SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(SessionError::Invalid(format!(
            "{label} must be a regular file rather than a symlink or non-file"
        )));
    }
    if metadata.len() > MAX_SESSION_FILE_BYTES {
        return Err(SessionError::Invalid(format!(
            "{label} has {} bytes; maximum is {MAX_SESSION_FILE_BYTES}",
            metadata.len()
        )));
    }
    Ok(())
}

fn directory_entry_names(path: &Path, label: &str) -> Result<BTreeSet<String>, SessionError> {
    let read_dir = fs::read_dir(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut names = BTreeSet::new();
    for entry in read_dir {
        let entry = entry.map_err(|source| SessionError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let name = entry.file_name().into_string().map_err(|_| {
            SessionError::Invalid(format!("{label} contains a non-UTF-8 entry name"))
        })?;
        names.insert(name);
    }
    Ok(names)
}

fn copy_regular_file(source_path: &Path, target_path: &Path) -> Result<(), SessionError> {
    let metadata = fs::symlink_metadata(source_path).map_err(|source| SessionError::Io {
        path: source_path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(SessionError::Invalid(format!(
            "source entry {} is not a regular file",
            source_path.display()
        )));
    }
    if metadata.len() > MAX_SESSION_FILE_BYTES {
        return Err(SessionError::Invalid(format!(
            "source entry {} has {} bytes; maximum is {MAX_SESSION_FILE_BYTES}",
            source_path.display(),
            metadata.len()
        )));
    }

    let source_file = File::open(source_path).map_err(|source| SessionError::Io {
        path: source_path.to_path_buf(),
        source,
    })?;
    let target_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target_path)
        .map_err(|source| SessionError::Io {
            path: target_path.to_path_buf(),
            source,
        })?;
    let mut reader = BufReader::new(source_file);
    let mut writer = BufWriter::new(target_file);
    let copied = io::copy(&mut reader.by_ref().take(MAX_SESSION_FILE_BYTES + 1), &mut writer)
        .map_err(|source| SessionError::Io {
            path: source_path.to_path_buf(),
            source,
        })?;
    if copied > MAX_SESSION_FILE_BYTES {
        return Err(SessionError::Invalid(format!(
            "source entry {} grew beyond maximum while being copied",
            source_path.display()
        )));
    }
    writer.flush().map_err(|source| SessionError::Io {
        path: target_path.to_path_buf(),
        source,
    })?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|source| SessionError::Io {
            path: target_path.to_path_buf(),
            source,
        })?;
    Ok(())
}

fn verify_directories_match(source_dir: &Path, copy_dir: &Path) -> Result<(), SessionError> {
    let source_entries = directory_entry_names(source_dir, "source archive")?;
    let copy_entries = directory_entry_names(copy_dir, "publication evidence")?;
    if source_entries != copy_entries {
        return Err(SessionError::Invalid(format!(
            "source archive entries changed during session creation: source {source_entries:?}, copy {copy_entries:?}"
        )));
    }
    for name in source_entries {
        compare_regular_files(&source_dir.join(&name), &copy_dir.join(&name), &name)?;
    }
    Ok(())
}

fn compare_regular_files(source_path: &Path, copy_path: &Path, name: &str) -> Result<(), SessionError> {
    let source_file = File::open(source_path).map_err(|source| SessionError::Io {
        path: source_path.to_path_buf(),
        source,
    })?;
    let copy_file = File::open(copy_path).map_err(|source| SessionError::Io {
        path: copy_path.to_path_buf(),
        source,
    })?;
    let source_metadata = source_file.metadata().map_err(|source| SessionError::Io {
        path: source_path.to_path_buf(),
        source,
    })?;
    let copy_metadata = copy_file.metadata().map_err(|source| SessionError::Io {
        path: copy_path.to_path_buf(),
        source,
    })?;
    if !source_metadata.is_file() || !copy_metadata.is_file() {
        return Err(SessionError::Invalid(format!(
            "session source/copy entry {name:?} ceased to be a regular file"
        )));
    }
    if source_metadata.len() != copy_metadata.len() {
        return Err(SessionError::Invalid(format!(
            "session source/copy entry {name:?} differs in size"
        )));
    }

    let mut source_reader = BufReader::new(source_file);
    let mut copy_reader = BufReader::new(copy_file);
    let mut source_buffer = [0_u8; 64 * 1024];
    let mut copy_buffer = [0_u8; 64 * 1024];
    loop {
        let source_read = source_reader
            .read(&mut source_buffer)
            .map_err(|source| SessionError::Io {
                path: source_path.to_path_buf(),
                source,
            })?;
        let copy_read = copy_reader
            .read(&mut copy_buffer)
            .map_err(|source| SessionError::Io {
                path: copy_path.to_path_buf(),
                source,
            })?;
        if source_read != copy_read || source_buffer[..source_read] != copy_buffer[..copy_read] {
            return Err(SessionError::Invalid(format!(
                "session source/copy entry {name:?} differs byte-for-byte"
            )));
        }
        if source_read == 0 {
            break;
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, SessionError> {
    require_regular_file(path, &path.display().to_string())?;
    let file = File::open(path).map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut encoded = Vec::new();
    file.take(MAX_SESSION_FILE_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|source| SessionError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if encoded.len() as u64 > MAX_SESSION_FILE_BYTES {
        return Err(SessionError::Invalid(format!(
            "JSON file {} exceeds maximum {MAX_SESSION_FILE_BYTES} bytes",
            path.display()
        )));
    }
    serde_json::from_slice(&encoded).map_err(|source| SessionError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), SessionError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| SessionError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value).map_err(|source| SessionError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    writer.write_all(b"\n").map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    writer.flush().map_err(|source| SessionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|source| SessionError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}
