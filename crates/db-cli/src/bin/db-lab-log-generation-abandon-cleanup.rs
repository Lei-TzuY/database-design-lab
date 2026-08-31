use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use db_cli::generation_abandoned_cleanup::{
    apply_abandoned_generation_cleanup, load_abandoned_generation_cleanup_plan,
    plan_abandoned_generation_cleanup,
};

#[derive(Debug, Parser)]
#[command(
    name = "db-lab-log-generation-abandon-cleanup",
    version,
    about = "Plan or explicitly confirm cleanup of reserved abandoned append-log generation artifacts"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print an exact non-destructive cleanup plan as JSON.
    Plan {
        /// Existing generation directory to inspect.
        #[arg(long)]
        directory: PathBuf,
    },
    /// Apply an unchanged saved plan after explicit operator abandonment confirmation.
    Apply {
        /// Existing generation directory whose retained evidence must exactly match the plan.
        #[arg(long)]
        directory: PathBuf,
        /// Saved JSON emitted by the plan command.
        #[arg(long)]
        plan: PathBuf,
        /// Explicit statement that the eligible candidate/staging artifacts are abandoned.
        #[arg(long)]
        confirm_abandoned: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
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

fn run(args: Cli) -> Result<String, Box<dyn std::error::Error>> {
    match args.command {
        Command::Plan { directory } => {
            let plan = plan_abandoned_generation_cleanup(&directory)?;
            Ok(serde_json::to_string_pretty(&plan)?)
        }
        Command::Apply {
            directory,
            plan,
            confirm_abandoned,
        } => {
            let plan = load_abandoned_generation_cleanup_plan(&plan)?;
            let summary =
                apply_abandoned_generation_cleanup(&directory, &plan, confirm_abandoned)?;
            Ok(serde_json::to_string_pretty(&summary)?)
        }
    }
}
