//! CLI argument definitions.
//!
//! This module defines the various command line flags and subcommands that
//! the `huk` executable exposes. It uses the [`clap`](https://crates.io/crates/clap)
//! crate for ergonomic argument parsing.

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use derive_more::with_trait::IsVariant;
use derive_more::with_trait::TryInto;
use paste::paste;
use thiserror::Error;

use crate::config;
use crate::install;
use crate::runner;
use crate::task;

#[derive(Debug, Error, IsVariant, TryInto)]
pub enum HukError {
  /// Wrapper around [runner errors][runner::RunnerError].
  #[error(transparent)]
  Runner(#[from] runner::RunnerError),

  /// Wrapper around [installation errors][install::InstallError].
  #[error(transparent)]
  Install(#[from] install::InstallError),

  /// Wrapper around task [specification parsing
  /// errors][task::TaskSpecParseError].
  #[error(transparent)]
  Parse(#[from] task::TaskSpecParseError),

  /// Wrapper around [configuration errors][config::ConfigError]
  #[error(transparent)]
  Config(#[from] config::ConfigError),
}

/// Top-level options for the `huk` binary.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
  /// Subcommand to execute.
  #[command(subcommand)]
  pub command: Commands,
}

macro_rules! cli {
  (
    $(
      $(#[doc = $doc:expr])+
      $(#[cfg($cfg:expr)])*
      $(#[$meta:meta])*
      $name:ident $(($($derives:meta),*))? $(=>)? $({
        $(
          $(#[$field_meta:meta])*
          $field_name:ident $(($($arg_opts:meta),*))? : $field_type:ty
        ),+ $(,)?
      })?
    ),+
    $(,)?
  ) => {
    paste! {
      /// All supported subcommands for the CLI.
      ///
      $(#[doc = r" - `" $name:snake r"` " $($doc)*])+
      #[derive(Subcommand, Debug, IsVariant)]
      pub enum Commands {
        $(
          $(#[doc = $doc])*
          $($(#[cfg($cfg)])+)?
          $name([<$name:camel Opts>])
        ),+
      }
    }
    $(
      paste! {
        #[doc = r" Options for the `" $name:snake "` subcommand."]
        $(#[doc = $doc])*
        $(#[cfg($cfg)])*
        #[derive(Args, Debug $(, $($derives),* )?)]
        $(#[$meta])*
        pub struct [<$name:camel Opts>] {
          $(
            $(
              #[arg($( $($arg_opts),* )?)]
              $(#[$field_meta])*
              pub $field_name: $field_type,
            )+
          )?
        }
      }
    )+
    paste! {
      $(use $crate::handlers::[<handle_$name:snake>];)+
      impl Cli {
        #[allow(dead_code)]
        pub(crate) fn run() {
          let cli = Self::parse();
          let result: Result<(), HukError> = match &cli.command {
            $(
              Commands::$name(opts) => [<handle_$name:snake>](opts).map_err(|e| <_ as Into<HukError>>::into(e)),
            )+
          };
          if let Err(err) = result {
            eprintln!("error: {err}");
            std::process::exit(1);
          }
        }
      }
    }
  };
}

cli! {
  /// Launch an interactive dashboard for managing hooks and tasks.
  #[command(
    aliases = ["dash", "db"],
    long_flag_aliases = ["interactive"],
    long_about = "Launch an \
      interactive dashboard for managing hooks and tasks.\n\n\
      The dashboard provides a TUI interface to view and modify the \
      configuration, run tasks, and monitor hook executions.\n\
      Within the dashboard, you can use the the following keybindings:\n\n  \
        A  (add)\n    \
           Add a new hook/task definition to the config file.\n  \
        E  (edit)\n    \
           Edit the selected hook/task definition, updating the config file.\n  \
        D  (delete)\n    \
           Remove the selected hook/task definition from the config file.\n  \
        R  (reload)\n    \
           Reload the configuration from disk, discarding unsaved changes.\n  \
        Q  (quit)\n    \
           Exit the dashboard and return to the terminal.\n  \
        ⏎  (enter)\n    \
           Run the selected hook/task or confirm input.\n \
       ↑|↓ (up / down)\n    \
           Navigate the list of hooks/tasks.\n \
       ←|→ (left / right)\n    \
           Reposition the cursor in text fields.\n")]
  #[cfg(feature = "tui")]
  Dashboard(Default),
  /// List configured Git hooks and associated tasks.
  #[command(
    aliases = ["ls", "l", "hooks"],
    long_flag_aliases = ["list", "hooks"],
    short_flag_aliases = ['l']
  )]
  List {
    /// Disable pretty-printing and color output.
    compact(
      long,
      short = 'c',
      long_help = "Disable pretty-printing and color formatting.\n\n\
        Combine with --json or --toml for compact machine-readable output."
    ): bool,
    /// Only output hook names without associated tasks.
    name_only(long, short = 'n'): bool,
    /// Format the results as standard JSON (JavaScript Object Notation).
    json(long, short = 'j'): bool,
    /// Format the results as YAML (YAML Ain't Markup Language).
    yaml(long, short = 'y', long_help = "Format the results as YAML (YAML \
    Ain't Markup Language).\n\nNote: this currently ignores the --compact flag."): bool,
    /// Format the results as TOML (Tom's Obvious, Minimal Language).
    toml(long, short = 't'): bool,
    /// Outputs a static list of names of all Git hooks that `huk` supports.
    all(
      long,
      short = 'a',
      long_help = "Output a list of names of all the Git hooks supported by \
        `huk`.\n\nUnlike other list options, this is unrelated to configuration.\n\
        It returns an immutable list of Git hook names (like 'pre-commit'),\n\
        indicating all of the hooks supported and understood by `huk`."
    ): bool,
  },
  /// Run the tasks for the specified hook name.
  #[command(
    aliases = ["r"],
    long_flag_aliases = ["run", "exec"],
    short_flag_aliases = ['r', 'x']
  )]
  Run(Default) {
    /// Name of the Git hook to execute. See `git help hooks` for a list of
    /// standard hook names recognized by Git and supported by `huk`.
    hook(): String,
    /// Additional arguments to forward to the hook runner.
    args(
      last = true,
      long_help = "Additional arguments to forward to the hook runner.\n\n\
        Depending on the hook being executed, Git may provide additional \
        arguments, such as the commit message file for `commit-msg` hook. \
        These will be passed along in order."
    ): Vec<String>,
    /// Enable verbose output during task execution.
    verbose(long, short = 'v'): bool,
  },
  /// List tasks available in the configuration and optionally run them.
  #[command(aliases = ["t", "tasks"])]
  Task(Default) {
    /// Run the specified task instead of just listing tasks.
    run(
      long_help = "Run the specified task instead of just listing tasks.\n\
        Note: cannot be used with --json, --yaml, --toml, or --compact."
    ): Option<String>,
    /// Additional arguments to forward to the task being run.
    args(
      last = true,
      long_help = "Additional arguments to forward to the task being run.\n\
        Note: cannot be used with --json, --yaml, --toml, or --compact."
    ): Vec<String>,
    /// Format the list of available tasks as JSON.
    json(long, short = 'j'): bool,
    /// Format the list of available tasks as YAML.
    yaml(long, short = 'y'): bool,
    /// Format the list of available tasks as TOML.
    toml(long, short = 't'): bool,
    /// Disable pretty-printing and color output.
    compact(
      long, short = 'c', long_help = "Disable pretty-printing and \
      color output. Can be combined with --json, --yaml, or --toml for \
      compact machine-readable output."
    ): bool,
    /// Enable verbose output during task execution.
    verbose(long, short = 'v'): bool,
  },
  /// Add a hook definition to the configuration file.
  #[command(aliases = ["a", "new"])]
  Add {
    /// Name of the Git hook to add.
    hook(): String,
    /// Task specification to associate with the hook.
    spec(
      required = true,
      last = true,
      long_help = "Task specification to associate with the hook.\n\n\
        Task specifications can take on several different forms:\n \
        1. a raw shell command string (e.g. `\"git add -A\"`)\n \
        2. a task name from the configuration file, which must either be:\n   \
        - defined in the `tasks` section of a deno.json file, or ...\n   \
        - defined in the `scripts` section of a package.json file\n \
        3. an object with `command`, `dependencies`, and/or `description` fields, where:\n   \
        - `command` is a shell command string to execute,\n   \
        - `dependencies` is an array of tasks to run before the command,\n     \
        Note: this field is required if `command` is not provided.\n   \
        - `description` is a human-readable summary of the task (optional)\n \
        4. a sequence where value satisfies either type 1, 2, or 3 above.\n \
        Multiple specifications can be provided to build a sequence."
    ): Vec<String>,
    /// Replace any existing hook definition instead of appending to it.
    replace(long, short = 'r'): bool,
  },
  /// Remove a hook definition from the configuration file.
  #[command(aliases = ["rm", "delete", "d"], short_flag_aliases = ['d'])]
  Remove {
    /// Name of the Git hook to remove.
    hook(): String,
    /// Only remove the specified task from the hook's definition.
    task(long, short = 't'): Option<String>,
    /// Only remove the hook if it matches the provided task specification.
    spec(long, short = 's'): Option<String>,
    /// Suppress errors if the specified hook does not exist.
    force(long, short = 'f'): bool,
  },
  /// Update an existing hook definition in the configuration file.
  #[command(
    aliases = ["up", "edit"],
    long_flag_aliases = ["update", "edit"],
    short_flag_aliases = ['u', 'e']
  )]
  Update {
    /// Name of the Git hook to update.
    hook(): String,
    /// New task specification to associate with the hook.
    spec(
      required = true,
      last = true,
      long_help = "New task specification to associate with the hook.\n\n\
        Task specifications can take on several different forms:\n \
        1. a raw shell command string (e.g. `\"git add -A\"`)\n \
        2. a task name from the configuration file, which must either be:\n   \
        - defined in the `tasks` section of a deno.json file, or ...\n   \
        - defined in the `scripts` section of a package.json file\n \
        3. an object with `command`, `dependencies`, and/or `description` fields, where:\n   \
        - `command` is a shell command string to execute,\n   \
        - `dependencies` is an array of tasks to run before the command,\n     \
        Note: this field is required if `command` is not provided.\n   \
        - `description` is a human-readable summary of the task (optional)\n \
        4. a sequence where value satisfies either type 1, 2, or 3 above.\n \
        Multiple specifications can be provided to build a sequence."
    ): Vec<String>,
    /// Replace the existing hook definition instead of appending to it.
    replace(long, short = 'r'): bool,
  },
  /// Install wrapper scripts into the Git hooks directory.
  #[command(
    aliases = ["link", "i"]
  )]
  Install {
    /// Override the default `core.hooksPath` (usually `.git/hooks`).
    #[arg(long, short = 'd', value_name = "PATH", long_help = "\
      Override the default `core.hooksPath` (usually `.git/hooks`).\n\n\
      This allows installing the hook scripts into a custom directory, which can\n\
      be useful for monorepos or non-standard Git setups.\n\n\
      IMPORTANT: if this value does not match the `core.hooksPath` value in your\n\
      git config, Git will likely fail to find and execute the installed hooks!")]
    hooks_dir: Option<String>,
    /// Overwrite existing hook scripts if they already exist.
    force(long, short = 'f', alias = "y"): bool,
  },
  /// Uninstall wrapper scripts from the Git hooks directory.
  #[command(alias = "un", alias = "u", alias = "unlink")]
  Uninstall {
    /// Override the default `core.hooksPath` (usually `.git/hooks`) uninstall path.
    #[arg(long, short = 'd', value_name = "PATH")]
    hooks_dir: Option<String>,
    /// Suppress errors if hook scripts do not exist.
    force(long, short = 'f', alias = "y"): bool,
    /// Specific hook names to uninstall. If not provided, `huk` will attempt to
    /// uninstall all hooks it previously installed. If you are using additional
    /// hook integrations in your repo, such as Git LFS, it is recommended to
    /// specify explicit names here to avoid removing those hooks.
    hooks(last = true): Vec<String>,
  },
}
