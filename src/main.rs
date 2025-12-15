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
#[macro_use]
mod macros;

pub use cli::*;
pub use constants::*;

fn main() {
  Cli::run();
}

pub(crate) mod handlers {
  pub use crate::install::*;
  pub use crate::runner::*;
  pub use crate::tui::*;
}
