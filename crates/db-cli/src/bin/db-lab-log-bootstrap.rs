use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use db_cli::legacy_bootstrap::bootstrap_legacy_log;

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-log-bootstrap",
    version,
    about = "Bootstrap a fresh generation directory from a quiesced legacy append-log file"
)]
struct Cli {
    /// Existing clean legacy append-log v1 file. It is never modified or removed.
    #[arg(long)]
    source: PathBuf,

    /// Fresh generation-directory path to create. The target must not already exist.
    #[arg(long)]
    target_directory: PathBuf,
}

fn main() -> ExitCode {
    let args = Cli::parse();
    match bootstrap_legacy_log(&args.source, &args.target_directory) {
        Ok(summary) => match serde_json::to_string_pretty(&summary) {
            Ok(encoded) => {
                println!("{encoded}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: failed to encode bootstrap summary: {error}");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
