//! Entry point for the `huk` CLI application.
//!
//! The `huk` program provides a set of subcommands for installing and running
//! Git hooks based on tasks defined in either a `deno.json`/`deno.jsonc` or
//! `package.json` file. A more detailed description of the available
//! functionality can be found in the crate documentation and in the
//! README.md accompanying this package.

mod cli;
mod config;
mod constants;
mod install;
mod runner;
mod task;
mod tui;

use clap::Parser;
use cli::Cli;
use cli::Commands;
use std::process;

pub use constants::*;

#[derive(Debug, thiserror::Error)]
pub enum HukError {
  /// Wrapper around runner errors.
  #[error(transparent)]
  Runner(#[from] runner::RunnerError),

  /// Wrapper around installation errors.
  #[error(transparent)]
  Install(#[from] install::InstallError),

  #[error(transparent)]
  Parse(#[from] task::TaskSpecParseError),

  #[error(transparent)]
  Config(#[from] config::ConfigError),
}

fn main() {
  // Parse command line arguments using clap.
  let cli = Cli::parse();

  // Dispatch the requested subcommand.
  let result: Result<(), HukError> = match &cli.command {
    Commands::Install(opts) => {
      install::handle_install(opts).map_err(|e| e.into())
    }
    Commands::List(opts) => runner::handle_list(opts).map_err(|e| e.into()),
    Commands::Run(opts) => runner::handle_run(opts).map_err(|e| e.into()),
    Commands::Tasks(opts) => runner::handle_tasks(opts).map_err(|e| e.into()),
    Commands::Dashboard(_opts) => tui::handle_dashboard().map_err(|e| e.into()),
    Commands::Add(opts) => runner::handle_add(opts).map_err(|e| e.into()),
    Commands::Remove(opts) => runner::handle_remove(opts).map_err(|e| e.into()),
    Commands::Update(opts) => runner::handle_update(opts).map_err(|e| e.into()),
  };

  // Exit with the appropriate code on error.
  if let Err(err) = result {
    eprintln!("error: {err}");
    process::exit(1);
  }
}
