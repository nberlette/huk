//! Public library API for the `huk` crate.
//!
//! Although `huk` is primarily intended to be used as a CLI application,
//! exposing its internals as a library makes it possible to write tests and
//! integrate the functionality into other programs. The modules exposed here
//! mirror those used by the CLI: configuration parsing, task definitions and
//! execution logic.

mod cli;
pub mod config;
pub mod constants;
pub mod install;
pub mod runner;
pub mod task;
mod tui;

pub use constants::*;
