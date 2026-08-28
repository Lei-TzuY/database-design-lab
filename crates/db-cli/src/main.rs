use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use db_core::{
    compare_workload, execute_workload, generate_workload, DbError, DifferentialError,
    GeneratorConfig, KvEngine, Outcome, Workload,
};
use db_storage_log::{InspectionReport, LogEngine, VerificationReport};
use db_storage_lsm::LsmEngine;
use db_storage_memory::MemoryEngine;
use serde::Serialize;
use thiserror::Error;

const MAX_WORKLOAD_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "db-lab",
    version,
    about = "Deterministic correctness laboratory for database architecture experiments"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate a versioned deterministic workload JSON file.
    Generate {
        /// Recorded SplitMix64 seed.
        #[arg(long)]
        seed: u64,
        /// Number of logical KV actions.
        #[arg(long, default_value_t = 1_000)]
        operations: u32,
        /// Number of reusable keys.
        #[arg(long, default_value_t = 128)]
        key_space: u32,
        /// Inclusive maximum generated value size.
        #[arg(long, default_value_t = 256)]
        max_value_bytes: u32,
        /// Insert an engine reopen after this many logical actions.
        #[arg(long)]
        reopen_every: Option<u32>,
        /// New JSON file to create; existing files are never overwritten.
        #[arg(long)]
        output: PathBuf,
    },
    /// Execute a workload against one engine and print all observable outcomes as JSON.
    Run {
        /// Engine implementation.
        #[arg(long, value_enum)]
        engine: EngineKind,
        /// Required for persistent engines; forbidden for `memory`.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Versioned workload JSON file.
        workload: PathBuf,
    },
    /// Compare the reference engine and a fresh persistent candidate step by step.
    Differential {
        /// Persistent candidate engine.
        #[arg(long, value_enum, default_value_t = PersistentEngineKind::Log)]
        engine: PersistentEngineKind,
        /// New candidate file or directory to create; existing paths are rejected.
        #[arg(long)]
        path: PathBuf,
        /// Versioned workload JSON file.
        workload: PathBuf,
    },
    /// Validate an append-log file without modifying it.
    Verify {
        /// Append-log file.
        path: PathBuf,
    },
    /// Replay and show live entries without modifying the append-log file.
    Inspect {
        /// Append-log file.
        path: PathBuf,
        /// Include values as hexadecimal. By default only key and value size are printed.
        #[arg(long)]
        show_values: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EngineKind {
    Memory,
    Log,
    Lsm,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PersistentEngineKind {
    Log,
    Lsm,
}

#[derive(Debug, Serialize)]
struct RunReport {
    engine: &'static str,
    workload_format_version: u16,
    seed: Option<u64>,
    steps_executed: usize,
    outcomes: Vec<Outcome>,
}

#[derive(Debug, Serialize)]
struct DifferentialCliReport {
    reference_engine: &'static str,
    candidate_engine: &'static str,
    workload_format_version: u16,
    seed: Option<u64>,
    steps_checked: usize,
}

#[derive(Debug, Error)]
enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Differential(#[from] DifferentialError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

fn main() -> ExitCode {
    match run_cli(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run_cli(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Generate {
            seed,
            operations,
            key_space,
            max_value_bytes,
            reopen_every,
            output,
        } => {
            let workload = generate_workload(GeneratorConfig {
                seed,
                operations,
                key_space,
                max_value_bytes,
                reopen_every,
            })?;
            write_new_json(&output, &workload)
        }
        Command::Run {
            engine,
            path,
            workload,
        } => {
            let workload = read_workload(&workload)?;
            match (engine, path) {
                (EngineKind::Memory, None) => {
                    let mut engine = MemoryEngine::new();
                    print_run_report(&mut engine, &workload)
                }
                (EngineKind::Memory, Some(_)) => Err(CliError::Usage(
                    "--path is not valid for the in-memory engine".to_owned(),
                )),
                (EngineKind::Log, Some(path)) => {
                    let mut engine = LogEngine::open(path)?;
                    print_run_report(&mut engine, &workload)
                }
                (EngineKind::Log, None) => Err(CliError::Usage(
                    "--path is required for the append-log engine".to_owned(),
                )),
                (EngineKind::Lsm, Some(path)) => {
                    let mut engine = LsmEngine::open(path)?;
                    print_run_report(&mut engine, &workload)
                }
                (EngineKind::Lsm, None) => Err(CliError::Usage(
                    "--path is required for the LSM engine".to_owned(),
                )),
            }
        }
        Command::Differential {
            engine,
            path,
            workload,
        } => {
            let workload = read_workload(&workload)?;
            let mut reference = MemoryEngine::new();
            match engine {
                PersistentEngineKind::Log => {
                    let mut candidate = LogEngine::create_new(path)?;
                    print_differential_report(&mut reference, &mut candidate, &workload)
                }
                PersistentEngineKind::Lsm => {
                    let mut candidate = LsmEngine::create_new(path)?;
                    print_differential_report(&mut reference, &mut candidate, &workload)
                }
            }
        }
        Command::Verify { path } => {
            let report: VerificationReport = LogEngine::verify(path)?;
            write_stdout_json(&report)
        }
        Command::Inspect { path, show_values } => {
            let report: InspectionReport = LogEngine::inspect(path, show_values)?;
            write_stdout_json(&report)
        }
    }
}

fn print_run_report<E: KvEngine>(engine: &mut E, workload: &Workload) -> Result<(), CliError> {
    let engine_name = engine.capabilities().name;
    let outcomes = execute_workload(engine, workload)?;
    write_stdout_json(&RunReport {
        engine: engine_name,
        workload_format_version: workload.format_version,
        seed: workload.seed,
        steps_executed: outcomes.len(),
        outcomes,
    })
}

fn print_differential_report<E: KvEngine>(
    reference: &mut MemoryEngine,
    candidate: &mut E,
    workload: &Workload,
) -> Result<(), CliError> {
    let reference_name = reference.capabilities().name;
    let candidate_name = candidate.capabilities().name;
    let report = compare_workload(reference, candidate, workload)?;
    write_stdout_json(&DifferentialCliReport {
        reference_engine: reference_name,
        candidate_engine: candidate_name,
        workload_format_version: workload.format_version,
        seed: workload.seed,
        steps_checked: report.steps_checked,
    })
}

fn read_workload(path: &Path) -> Result<Workload, CliError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_WORKLOAD_FILE_BYTES {
        return Err(CliError::Usage(format!(
            "workload file has {} bytes; maximum is {MAX_WORKLOAD_FILE_BYTES}",
            metadata.len()
        )));
    }
    let encoded = fs::read(path)?;
    let workload: Workload = serde_json::from_slice(&encoded)?;
    workload.validate()?;
    Ok(workload)
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), CliError> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn write_stdout_json(value: &impl Serialize) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use db_core::Workload;

    use super::{Cli, Command, EngineKind, PersistentEngineKind};

    #[test]
    fn suggested_run_shape_parses() {
        let cli = Cli::try_parse_from([
            "db-lab",
            "run",
            "--engine",
            "log",
            "--path",
            "lab.db",
            "workload.json",
        ])
        .expect("parse run command");
        assert!(matches!(
            cli.command,
            Command::Run {
                engine: EngineKind::Log,
                ..
            }
        ));
    }

    #[test]
    fn committed_semantics_fixture_is_valid() {
        let encoded = include_str!("../../../fixtures/workloads/semantics-v1.json");
        let workload: Workload = serde_json::from_str(encoded).expect("parse semantics fixture");
        workload.validate().expect("validate semantics fixture");
        assert_eq!(workload.steps.len(), 11);
    }

    #[test]
    fn lsm_run_and_differential_shapes_parse() {
        let run = Cli::try_parse_from([
            "db-lab",
            "run",
            "--engine",
            "lsm",
            "--path",
            "engine-dir",
            "workload.json",
        ])
        .expect("parse LSM run");
        assert!(matches!(
            run.command,
            Command::Run {
                engine: EngineKind::Lsm,
                ..
            }
        ));

        let differential = Cli::try_parse_from([
            "db-lab",
            "differential",
            "--engine",
            "lsm",
            "--path",
            "fresh-dir",
            "workload.json",
        ])
        .expect("parse LSM differential");
        assert!(matches!(
            differential.command,
            Command::Differential {
                engine: PersistentEngineKind::Lsm,
                ..
            }
        ));
    }
}
