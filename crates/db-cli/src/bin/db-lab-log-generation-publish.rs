use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use db_cli::generation_publication::publish_generation_marker;

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-log-generation-publish",
    version,
    about = "Durably publish an append-log generation commit marker on supported hosts"
)]
struct Cli {
    /// Existing generation directory.
    #[arg(long)]
    directory: PathBuf,

    /// Generation id whose clean append-log image should become committed.
    #[arg(long)]
    generation: u64,
}

fn main() -> ExitCode {
    let args = Cli::parse();
    match publish_generation_marker(&args.directory, args.generation) {
        Ok(summary) => match serde_json::to_string_pretty(&summary) {
            Ok(encoded) => {
                println!("{encoded}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: failed to encode publication summary: {error}");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
