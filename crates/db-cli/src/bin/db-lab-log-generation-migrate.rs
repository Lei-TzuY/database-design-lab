use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use db_cli::generation_migration::migrate_legacy_append_log;

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-log-generation-migrate",
    version,
    about = "Offline migrate one legacy append-log file into a fresh generation directory"
)]
struct Cli {
    /// Existing clean legacy one-file append log. Raw-path writers must remain quiesced.
    #[arg(long)]
    source: PathBuf,

    /// Fresh generation-directory path to create. It must not already exist.
    #[arg(long)]
    target_directory: PathBuf,
}

fn main() -> ExitCode {
    let args = Cli::parse();
    match migrate_legacy_append_log(&args.source, &args.target_directory) {
        Ok(summary) => match serde_json::to_string_pretty(&summary) {
            Ok(encoded) => {
                println!("{encoded}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: failed to encode migration summary: {error}");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
