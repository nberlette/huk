//! CLI argument definitions.
//!
//! This module defines the various command line flags and subcommands that
//! the `huk` executable exposes. It uses the [`clap`](https://crates.io/crates/clap)
//! crate for ergonomic argument parsing.

use clap::Args;
use clap::Parser;
use clap::Subcommand;

/// Top-level options for the `huk` binary.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
  /// Subcommand to execute.
  #[command(subcommand)]
  pub command: Commands,
}

/// All supported subcommands for the CLI.
#[derive(Subcommand, Debug)]
pub enum Commands {
  /// Install wrapper scripts into the Git hooks directory.
  Install(InstallOpts),
  /// List configured Git hooks and associated tasks.
  List(ListOpts),
  /// Run the tasks for the specified hook name.
  Run(RunOpts),
  /// List tasks available in the configuration and optionally run them.
  Tasks(TasksOpts),
  /// Launch an interactive dashboard for managing hooks and tasks.
  Dashboard(DashboardOpts),
  /// Add a hook definition to the configuration file.
  Add(AddOpts),
  /// Remove a hook definition from the configuration file.
  Remove(RemoveOpts),
  /// Update an existing hook definition in the configuration file.
  Update(UpdateOpts),
}

/// Options for the `install` subcommand.
#[derive(Args, Debug)]
pub struct InstallOpts {
  /// Overwrite existing hook scripts if they already exist.
  #[arg(long, short)]
  pub force: bool,
}

/// Options for the `list` subcommand.
#[derive(Args, Debug, Default)]
pub struct ListOpts {
  /// Show verbose output, including the raw configuration for each hook.
  #[arg(long, short)]
  pub verbose: bool,
}

/// Options for the `run` subcommand.
#[derive(Args, Debug)]
pub struct RunOpts {
  /// Name of the Git hook to execute.
  #[arg()]
  pub hook: String,
  /// Additional arguments to forward to the hook runner. Git passes hook
  /// parameters depending on the hook type; these are forwarded unmodified.
  #[arg(last = true)]
  pub args: Vec<String>,
}

/// Options for the `tasks` subcommand.
#[derive(Args, Debug)]
pub struct TasksOpts {
  /// If provided, run the specified task instead of just listing tasks.
  #[arg(long, short)]
  pub run: Option<String>,
}

/// Options for the `dashboard` subcommand. Currently unused.
#[derive(Args, Debug, Default)]
pub struct DashboardOpts {}

/// Options for the `add` subcommand.
#[derive(Args, Debug)]
pub struct AddOpts {
  /// Name of the Git hook to add.
  #[arg()]
  pub hook: String,
  /// Task specification to associate with the hook. This can be a raw command
  /// string, a task name defined in your configuration, or a JSON object.
  #[arg()]
  pub spec: String,
}

/// Options for the `remove` subcommand.
#[derive(Args, Debug)]
pub struct RemoveOpts {
  /// Name of the Git hook to remove.
  #[arg()]
  pub hook: String,
}

/// Options for the `update` subcommand.
#[derive(Args, Debug)]
pub struct UpdateOpts {
  /// Name of the Git hook to update.
  #[arg()]
  pub hook: String,
  /// New task specification. See `add` for supported formats.
  #[arg()]
  pub spec: String,
}
