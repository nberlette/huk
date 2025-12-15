#![feature(associated_type_defaults)]
#![feature(trait_alias)]

//! Public library API for the `huk` crate.
//!
//! Although `huk` is primarily intended to be used as a CLI application,
//! exposing its internals as a library makes it possible to write tests and
//! integrate the functionality into other programs. The modules exposed here
//! mirror those used by the CLI: configuration parsing, task definitions and
//! execution logic.

pub mod cli;
pub mod config;
pub mod constants;
pub mod install;
pub mod runner;
pub mod task;
pub mod tui;

#[macro_use]
pub(crate) mod macros;

pub(crate) mod handlers {
  pub use crate::install::*;
  pub use crate::runner::*;
  pub use crate::tui::*;
}

pub use constants::*;

#[cfg(test)]
mod tests;
