//! This module exposes constants that are used throughout the `huk` crate.

/// The full semantic version of the `huk` crate at the time of compilation.
/// This is used internally for displaying version information in the CLI and
/// TUI dashboard interface, as well as for logging and diagnostics.
///
/// For convenience, this module also provides the individual components of the
/// version (major, minor, patch, pre-release) as separate constants:
///
///  - [`VERSION_MAJOR`]
///  - [`VERSION_MINOR`]
///  - [`VERSION_PATCH`]
///  - [`VERSION_PRE`]
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The major version component (e.g., `1` in `1.2.3-rc.2`) of the `huk` crate
/// at the time of compilation.
pub const VERSION_MAJOR: &str = env!("CARGO_PKG_VERSION_MAJOR");
/// The minor version component (e.g., `2` in `1.2.3-rc.2`) of the `huk` crate
/// at the time of compilation.
pub const VERSION_MINOR: &str = env!("CARGO_PKG_VERSION_MINOR");
/// The patch version component (e.g., `3` in `1.2.3-rc.2`) of the `huk` crate
/// at the time of compilation.
pub const VERSION_PATCH: &str = env!("CARGO_PKG_VERSION_PATCH");
/// The pre-release version component (e.g., `rc.2` in `1.2.3-rc.2`) of the
/// `huk` crate at the time of compilation.
pub const VERSION_PRE: &str = env!("CARGO_PKG_VERSION_PRE");

/// Git hook names as defined by [Git documentation]. These are the standard
/// hooks that Git recognizes and invokes at various points in its workflow.
/// We use this list to validate user input and ensure the files we install
/// in the git hooks directory will actually be recognized by Git.
///
/// [Git documentation]: https://git-scm.com/docs/githooks
pub const GIT_HOOKS: [&str; 22] = [
  "pre-applypatch",
  "pre-auto-gc",
  "pre-checkout",
  "pre-commit",
  "pre-merge-commit",
  "pre-push",
  "pre-rebase",
  "pre-receive",
  "prepare-commit-msg",
  "applypatch-msg",
  "commit-msg",
  "post-applypatch",
  "post-checkout",
  "post-commit",
  "post-merge",
  "post-receive",
  "post-rewrite",
  "post-update",
  "push-to-checkout",
  "fsmonitor-watchman",
  "sendemail-validate",
  "update",
];
