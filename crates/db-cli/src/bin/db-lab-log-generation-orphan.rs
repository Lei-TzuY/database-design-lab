use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use db_cli::generation_orphan::{
    inspect_generation_orphan, retire_generation_orphan, GenerationFileFingerprint,
    GenerationOrphanError,
};

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-log-generation-orphan",
    version,
    about = "Inspect or explicitly retire abandoned append-log generation candidates"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Read one higher uncommitted generation and report the exact retirement fingerprint.
    Inspect {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        generation: u64,
    },
    /// Replace one confirmed-abandoned generation log with durable allocation-frontier evidence.
    Retire {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        generation: u64,
        /// Exact authoritative generation returned by `inspect`.
        #[arg(long)]
        expected_authority: u64,
        /// Exact byte length returned by `inspect`.
        #[arg(long)]
        expected_bytes: u64,
        /// Exact decimal CRC-32/IEEE returned by `inspect`.
        #[arg(long)]
        expected_crc32: u32,
        /// Required operator attestation; the tool never infers candidate-builder liveness.
        #[arg(long)]
        confirm_generation_builder_stopped: bool,
    },
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Inspect {
            directory,
            generation,
        } => inspect_generation_orphan(&directory, generation).and_then(|summary| encode_json(&summary)),
        Command::Retire {
            directory,
            generation,
            expected_authority,
            expected_bytes,
            expected_crc32,
            confirm_generation_builder_stopped,
        } => retire_generation_orphan(
            &directory,
            generation,
            expected_authority,
            GenerationFileFingerprint {
                bytes: expected_bytes,
                crc32: expected_crc32,
            },
            confirm_generation_builder_stopped,
        )
        .and_then(|summary| encode_json(&summary)),
    };

    match result {
        Ok(encoded) => {
            println!("{encoded}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn encode_json<T: serde::Serialize>(value: &T) -> Result<String, GenerationOrphanError> {
    serde_json::to_string_pretty(value).map_err(|error| {
        GenerationOrphanError::Invalid(format!(
            "failed to encode generation orphan summary: {error}"
        ))
    })
}
