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

const FORMAT_VERSION: u16 = 1;
const SESSION_PROTOCOL: &str = "controlled_publication_session_v1";
const PUBLICATION_PROTOCOL: &str = "publication_warm_v1";
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const PREFLIGHT_FILE: &str = "host-preflight.json";
const EVIDENCE_DIR: &str = "evidence";
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
    Create {
        #[arg(long)]
        host_preflight: PathBuf,
        #[arg(long)]
        archive_dir: PathBuf,
        #[arg(long)]
        session_dir: PathBuf,
        #[arg(long)]
        expected_revision: Option<String>,
    },
    Verify {
        #[arg(long)]
        session_dir: PathBuf,
        #[arg(long)]
        expected_revision: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionIndex {
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
struct SessionSummary {
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
    let args = Cli::parse();
    let result = match args.command {
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
                eprintln!("error: failed to encode session summary: {error}");
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
    preflight_path: &Path,
    archive_path: &Path,
    session_path: &Path,
    expected_revision: Option<&str>,
) -> Result<SessionSummary, SessionError> {
    let source_preflight = canonical_regular_file(preflight_path, "host preflight")?;
    let source_archive = canonical_directory(archive_path, "source archive")?;
    let target = fresh_target(session_path)?;
    reject_overlap(&source_archive, &target)?;

    let preflight = load_verified_host_preflight_snapshot(&source_preflight, None, true)?;
    let archive = verify_batch_archive(&source_archive, expected_revision, true)?;
    let host_label = publication_host_label(&source_archive)?;
    match_host(&preflight, &host_label)?;

    fs::create_dir(&target).map_err(|source| io_error(&target, source))?;
    let result = create_session_inner(
        &source_preflight,
        &source_archive,
        &target,
        &preflight,
        &archive,
        &host_label,
    );
    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result
}

fn create_session_inner(
    source_preflight: &Path,
    source_archive: &Path,
    target: &Path,
    preflight: &HostPreflightSnapshot,
    archive: &VerificationSummary,
    host_label: &str,
) -> Result<SessionSummary, SessionError> {
    let copied_preflight = target.join(PREFLIGHT_FILE);
    copy_new(source_preflight, &copied_preflight)?;

    let evidence = target.join(EVIDENCE_DIR);
    fs::create_dir(&evidence).map_err(|source| io_error(&evidence, source))?;
    let source_files = entry_names(source_archive, "source archive")?;
    for name in &source_files {
        copy_new(&source_archive.join(name), &evidence.join(name))?;
    }

    let copied_preflight_value =
        load_verified_host_preflight_snapshot(&copied_preflight, Some(host_label), true)?;
    if &copied_preflight_value != preflight {
        return invalid("host-preflight value changed while it was copied");
    }
    let copied_archive = verify_batch_archive(&evidence, Some(&archive.repository_revision), true)?;
    if &copied_archive != archive {
        return invalid("archive verification summary changed while evidence was copied");
    }
    if publication_host_label(&evidence)? != host_label {
        return invalid("publication host label changed while evidence was copied");
    }

    let source_preflight_after =
        load_verified_host_preflight_snapshot(source_preflight, Some(host_label), true)?;
    if &source_preflight_after != preflight {
        return invalid("source host-preflight changed while the session was being created");
    }
    let source_archive_after =
        verify_batch_archive(source_archive, Some(&archive.repository_revision), true)?;
    if &source_archive_after != archive {
        return invalid("source archive changed while the session was being created");
    }
    if publication_host_label(source_archive)? != host_label {
        return invalid("source publication host label changed while the session was being created");
    }
    compare_files(source_preflight, &copied_preflight, PREFLIGHT_FILE)?;
    compare_directories(source_archive, &evidence)?;

    let index = SessionIndex {
        format_version: FORMAT_VERSION,
        session_protocol: SESSION_PROTOCOL.to_owned(),
        host_preflight_protocol: HOST_PREFLIGHT_PROTOCOL.to_owned(),
        publication_admission_protocol: PUBLICATION_PROTOCOL.to_owned(),
        host_label: host_label.to_owned(),
        preflight_recorded_unix_seconds: preflight.recorded_unix_seconds,
        repository_revision: archive.repository_revision.clone(),
        source_archive_format_version: archive.format_version,
        evidence_files: source_files.into_iter().collect(),
    };
    write_new_json(&target.join(INDEX_FILE), &index)?;
    verify_session(target, Some(&archive.repository_revision))
}

fn verify_session(
    session_path: &Path,
    expected_revision: Option<&str>,
) -> Result<SessionSummary, SessionError> {
    let session = canonical_directory(session_path, "publication session")?;
    require_session_layout(&session)?;

    let index_path = session.join(INDEX_FILE);
    let index: SessionIndex = serde_json::from_value(read_json(&index_path)?).map_err(|source| {
        SessionError::Json {
            path: index_path.clone(),
            source,
        }
    })?;
    validate_index(&index, expected_revision)?;

    let preflight = load_verified_host_preflight_snapshot(
        &session.join(PREFLIGHT_FILE),
        Some(&index.host_label),
        true,
    )?;
    if preflight.recorded_unix_seconds != index.preflight_recorded_unix_seconds {
        return invalid("host-preflight recording time differs from session index");
    }

    let evidence = session.join(EVIDENCE_DIR);
    let actual_files: Vec<String> = entry_names(&evidence, "publication evidence")?
        .into_iter()
        .collect();
    if actual_files != index.evidence_files {
        return invalid(format!(
            "publication evidence file set differs from index: expected {:?}, found {actual_files:?}",
            index.evidence_files
        ));
    }

    let archive = verify_batch_archive(&evidence, Some(&index.repository_revision), true)?;
    if archive.format_version != index.source_archive_format_version {
        return invalid(format!(
            "source archive format differs from index: expected v{}, verified v{}",
            index.source_archive_format_version, archive.format_version
        ));
    }
    let archive_host = publication_host_label(&evidence)?;
    if archive_host != index.host_label {
        return invalid(format!(
            "publication archive host label {archive_host:?} differs from session host label {:?}",
            index.host_label
        ));
    }
    match_host(&preflight, &archive_host)?;

    Ok(SessionSummary {
        valid: true,
        session_format_version: FORMAT_VERSION,
        session_protocol: SESSION_PROTOCOL,
        host_label: index.host_label,
        preflight_recorded_unix_seconds: index.preflight_recorded_unix_seconds,
        repository_revision: archive.repository_revision,
        source_archive_format_version: archive.format_version,
        evidence_files: index.evidence_files.len(),
    })
}

fn validate_index(index: &SessionIndex, expected_revision: Option<&str>) -> Result<(), SessionError> {
    if index.format_version != FORMAT_VERSION {
        return invalid(format!(
            "unsupported publication-session format {}; expected {FORMAT_VERSION}",
            index.format_version
        ));
    }
    if index.session_protocol != SESSION_PROTOCOL {
        return invalid(format!(
            "unsupported publication-session protocol {:?}",
            index.session_protocol
        ));
    }
    if index.host_preflight_protocol != HOST_PREFLIGHT_PROTOCOL {
        return invalid("host-preflight protocol differs from the shared verifier");
    }
    if index.publication_admission_protocol != PUBLICATION_PROTOCOL {
        return invalid("publication admission protocol differs from publication_warm_v1");
    }
    if index.host_label.trim().is_empty() || index.host_label.trim() != index.host_label {
        return invalid("index host_label must be non-empty without surrounding whitespace");
    }
    if index.preflight_recorded_unix_seconds == 0 {
        return invalid("index preflight recording time must be greater than zero");
    }
    if index.repository_revision.trim().is_empty()
        || index.repository_revision.trim() != index.repository_revision
    {
        return invalid("index repository_revision must be non-empty without surrounding whitespace");
    }
    if let Some(expected) = expected_revision {
        if index.repository_revision != expected {
            return invalid(format!(
                "session repository revision {:?} differs from expected revision {expected:?}",
                index.repository_revision
            ));
        }
    }
    if !matches!(index.source_archive_format_version, 7 | 11) {
        return invalid(format!(
            "publication session requires source archive format v7 or v11; found v{}",
            index.source_archive_format_version
        ));
    }
    if index.evidence_files.is_empty() {
        return invalid("index evidence_files must not be empty");
    }
    let unique: BTreeSet<&str> = index.evidence_files.iter().map(String::as_str).collect();
    let sorted: Vec<&str> = unique.iter().copied().collect();
    let recorded: Vec<&str> = index.evidence_files.iter().map(String::as_str).collect();
    if unique.len() != recorded.len() || sorted != recorded {
        return invalid("index evidence_files must be sorted and duplicate-free");
    }
    Ok(())
}

fn publication_host_label(archive: &Path) -> Result<String, SessionError> {
    let environment = read_json(&archive.join("environment.json"))?;
    environment
        .get("publication_admission")
        .and_then(Value::as_object)
        .and_then(|admission| admission.get("host_label"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            SessionError::Invalid(
                "verified publication archive is missing publication_admission.host_label"
                    .to_owned(),
            )
        })
}

fn match_host(preflight: &HostPreflightSnapshot, archive_host: &str) -> Result<(), SessionError> {
    if preflight.host_label != archive_host {
        return invalid(format!(
            "host-preflight label {:?} differs from publication archive host label {archive_host:?}",
            preflight.host_label
        ));
    }
    Ok(())
}

fn canonical_regular_file(path: &Path, label: &str) -> Result<PathBuf, SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return invalid(format!(
            "{label} must be a regular file rather than a symlink or non-file"
        ));
    }
    fs::canonicalize(path).map_err(|source| io_error(path, source))
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_dir() {
        return invalid(format!(
            "{label} must be a real directory rather than a symlink or non-directory"
        ));
    }
    fs::canonicalize(path).map_err(|source| io_error(path, source))
}

fn fresh_target(path: &Path) -> Result<PathBuf, SessionError> {
    match fs::symlink_metadata(path) {
        Ok(_) => return invalid(format!("session destination already exists: {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error(path, source)),
    }
    let name = path.file_name().ok_or_else(|| {
        SessionError::Invalid(format!(
            "session destination has no final path component: {}",
            path.display()
        ))
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|source| io_error(parent, source))?;
    Ok(parent.join(name))
}

fn reject_overlap(source: &Path, target: &Path) -> Result<(), SessionError> {
    if source == target || source.starts_with(target) || target.starts_with(source) {
        return invalid("source archive and publication session must be distinct, non-nested paths");
    }
    Ok(())
}

fn require_session_layout(session: &Path) -> Result<(), SessionError> {
    let actual = entry_names(session, "publication session")?;
    let expected: BTreeSet<String> = [PREFLIGHT_FILE, EVIDENCE_DIR, INDEX_FILE]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if actual != expected {
        return invalid(format!(
            "publication-session entries must be exactly {expected:?}; found {actual:?}"
        ));
    }
    require_regular(&session.join(PREFLIGHT_FILE), PREFLIGHT_FILE)?;
    require_regular(&session.join(INDEX_FILE), INDEX_FILE)?;
    let evidence = session.join(EVIDENCE_DIR);
    let metadata = fs::symlink_metadata(&evidence).map_err(|source| io_error(&evidence, source))?;
    if !metadata.file_type().is_dir() {
        return invalid("publication-session evidence must be a real directory");
    }
    Ok(())
}

fn require_regular(path: &Path, label: &str) -> Result<(), SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return invalid(format!("{label} must be a regular file"));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return invalid(format!(
            "{label} has {} bytes; maximum is {MAX_FILE_BYTES}",
            metadata.len()
        ));
    }
    Ok(())
}

fn entry_names(path: &Path, label: &str) -> Result<BTreeSet<String>, SessionError> {
    let entries = fs::read_dir(path).map_err(|source| io_error(path, source))?;
    let mut names = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|source| io_error(path, source))?;
        let name = entry.file_name().into_string().map_err(|_| {
            SessionError::Invalid(format!("{label} contains a non-UTF-8 entry name"))
        })?;
        names.insert(name);
    }
    Ok(names)
}

fn copy_new(source: &Path, target: &Path) -> Result<(), SessionError> {
    let metadata = fs::symlink_metadata(source).map_err(|error| io_error(source, error))?;
    if !metadata.file_type().is_file() {
        return invalid(format!("source entry {} is not a regular file", source.display()));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return invalid(format!("source entry {} exceeds size limit", source.display()));
    }

    let source_file = File::open(source).map_err(|error| io_error(source, error))?;
    let target_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| io_error(target, error))?;
    let mut reader = BufReader::new(source_file);
    let mut writer = BufWriter::new(target_file);
    let copied = io::copy(
        &mut reader.by_ref().take(MAX_FILE_BYTES + 1),
        &mut writer,
    )
    .map_err(|error| io_error(source, error))?;
    if copied > MAX_FILE_BYTES {
        return invalid(format!("source entry {} grew beyond size limit", source.display()));
    }
    writer.flush().map_err(|error| io_error(target, error))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| io_error(target, error))?;
    Ok(())
}

fn compare_directories(source: &Path, copy: &Path) -> Result<(), SessionError> {
    let source_names = entry_names(source, "source archive")?;
    let copy_names = entry_names(copy, "publication evidence")?;
    if source_names != copy_names {
        return invalid("source archive file set changed while the session was being created");
    }
    for name in source_names {
        compare_files(&source.join(&name), &copy.join(&name), &name)?;
    }
    Ok(())
}

fn compare_files(source: &Path, copy: &Path, label: &str) -> Result<(), SessionError> {
    let source_file = File::open(source).map_err(|error| io_error(source, error))?;
    let copy_file = File::open(copy).map_err(|error| io_error(copy, error))?;
    let source_metadata = source_file.metadata().map_err(|error| io_error(source, error))?;
    let copy_metadata = copy_file.metadata().map_err(|error| io_error(copy, error))?;
    if !source_metadata.is_file()
        || !copy_metadata.is_file()
        || source_metadata.len() != copy_metadata.len()
    {
        return invalid(format!("source/copy metadata differs for {label:?}"));
    }

    let mut left = BufReader::new(source_file);
    let mut right = BufReader::new(copy_file);
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_len = left
            .read(&mut left_buffer)
            .map_err(|error| io_error(source, error))?;
        let right_len = right
            .read(&mut right_buffer)
            .map_err(|error| io_error(copy, error))?;
        if left_len != right_len || left_buffer[..left_len] != right_buffer[..right_len] {
            return invalid(format!("source/copy bytes differ for {label:?}"));
        }
        if left_len == 0 {
            return Ok(());
        }
    }
}

fn read_json(path: &Path) -> Result<Value, SessionError> {
    require_regular(path, &path.display().to_string())?;
    let file = File::open(path).map_err(|error| io_error(path, error))?;
    let mut encoded = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|error| io_error(path, error))?;
    if encoded.len() as u64 > MAX_FILE_BYTES {
        return invalid(format!("JSON file {} exceeds size limit", path.display()));
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
        .map_err(|error| io_error(path, error))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value).map_err(|source| SessionError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    writer.write_all(b"\n").map_err(|error| io_error(path, error))?;
    writer.flush().map_err(|error| io_error(path, error))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| io_error(path, error))?;
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> SessionError {
    SessionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, SessionError> {
    Err(SessionError::Invalid(message.into()))
}
