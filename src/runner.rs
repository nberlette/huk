//! Execution logic for hooks and tasks.
//!
//! This module contains the implementations of the various CLI handlers
//! responsible for listing, running and modifying hooks. It delegates
//! configuration loading to [`crate::config`] and executes commands via
//! [`std::process::Command`].

use crate::cli::AddOpts;
use crate::cli::ListOpts;
use crate::cli::RemoveOpts;
use crate::cli::RunOpts;
use crate::cli::TasksOpts;
use crate::cli::UpdateOpts;
use crate::config::ConfigError;
use crate::config::HookConfig;
use crate::task::TaskSpec;
use std::collections::HashSet;
use std::io;
use std::process::Command;
use std::process::ExitStatus;
use thiserror::Error;

/// Aggregate error type for operations performed by the runner.
#[derive(Error, Debug)]
pub enum RunnerError {
  /// Failure to load the configuration.
  #[error(transparent)]
  Config(#[from] ConfigError),
  /// Failure to execute a command.
  #[error("command '{cmd}' failed with status {status}")]
  CommandFailure { cmd: String, status: ExitStatus },
  /// A referenced task could not be found.
  #[error("task '{0}' not found in configuration")]
  TaskNotFound(String),
  /// A circular dependency was detected while resolving tasks.
  #[error("circular dependency detected for task '{0}'")]
  CircularDependency(String),
  /// An underlying I/O error occurred.
  #[error(transparent)]
  Io(#[from] io::Error),
}

/// Handler for the `list` subcommand.
pub fn handle_list(opts: &ListOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  if cfg.hooks.is_empty() {
    println!("No hooks defined in configuration.");
    return Ok(());
  }
  println!("Configured hooks:");
  for (hook, spec) in &cfg.hooks {
    if opts.verbose {
      println!("- {hook}: {spec:?}");
    } else {
      println!("- {hook}");
    }
  }
  Ok(())
}

/// Handler for the `run` subcommand.
pub fn handle_run(opts: &RunOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  if let Some(spec) = cfg.hooks.get(&opts.hook) {
    let mut runner = TaskRunner::new(&cfg);
    runner.run_spec(spec, &opts.hook, &opts.args)?;
  } else {
    println!("Hook '{}' is not defined in configuration.", opts.hook);
  }
  Ok(())
}

/// Handler for the `tasks` subcommand.
pub fn handle_tasks(opts: &TasksOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  // Collect all task names from node_scripts and deno_tasks.
  let mut all_tasks: HashSet<String> = HashSet::new();
  all_tasks.extend(cfg.node_scripts.keys().cloned());
  all_tasks.extend(cfg.deno_tasks.keys().cloned());
  if let Some(ref run_task) = opts.run {
    if all_tasks.contains(run_task) {
      let mut runner = TaskRunner::new(&cfg);
      runner.run_named_task(run_task)?;
    } else {
      println!("Task '{run_task}' not found.");
    }
  } else {
    println!("Available tasks:");
    for name in all_tasks {
      println!("- {name}");
    }
  }
  Ok(())
}

/// Handler for the `add` subcommand.
pub fn handle_add(_opts: &AddOpts) -> Result<(), RunnerError> {
  // TODO: implement adding hooks via CLI to configuration files.
  println!(
    "Adding hooks via CLI is not yet implemented. Please edit your configuration file manually."
  );
  Ok(())
}

/// Handler for the `remove` subcommand.
pub fn handle_remove(_opts: &RemoveOpts) -> Result<(), RunnerError> {
  // TODO: implement removing hooks via CLI from configuration files.
  println!(
    "Removing hooks via CLI is not yet implemented. Please edit your configuration file manually."
  );
  Ok(())
}

/// Handler for the `update` subcommand.
pub fn handle_update(_opts: &UpdateOpts) -> Result<(), RunnerError> {
  // TODO: implement updating hooks via CLI in configuration files.
  println!(
    "Updating hooks via CLI is not yet implemented. Please edit your configuration file manually."
  );
  Ok(())
}

/// A stateful task runner responsible for executing task specifications.
pub struct TaskRunner<'cfg> {
  pub config: &'cfg HookConfig,
  visiting:   HashSet<String>,
}

impl<'cfg> TaskRunner<'cfg> {
  pub fn new(config: &'cfg HookConfig) -> Self {
    Self {
      config,
      visiting: HashSet::new(),
    }
  }

  /// Execute a task specification. The `hook` name is used to label error
  /// messages and the `extra_args` are arguments forwarded from Git to the
  /// hook script.
  pub(crate) fn run_spec(
    &mut self,
    spec: &TaskSpec,
    hook: &str,
    extra_args: &[String],
  ) -> Result<(), RunnerError> {
    match spec {
      TaskSpec::Single(name) => self.run_single(name, extra_args),
      TaskSpec::Detailed {
        command,
        dependencies,
        ..
      } => {
        // Execute dependencies first.
        for dep in dependencies {
          self.run_named_task(dep)?;
        }
        if let Some(cmd) = command {
          self.exec_raw_command(cmd, extra_args)
        } else {
          // Only dependencies defined; nothing else to do.
          Ok(())
        }
      }
      TaskSpec::Sequence(list) => {
        for item in list {
          self.run_spec(item, hook, extra_args)?;
        }
        Ok(())
      }
    }
  }

  /// Execute a single task by name or treat it as a raw command if unknown.
  pub(crate) fn run_single(
    &mut self,
    name: &str,
    extra_args: &[String],
  ) -> Result<(), RunnerError> {
    // To avoid cycles, track the task names we are resolving.
    if self.visiting.contains(name) {
      return Err(RunnerError::CircularDependency(name.to_string()));
    }
    self.visiting.insert(name.to_string());
    let result = if let Some(cmd) = self.config.deno_tasks.get(name) {
      // It's a Deno task.
      self.exec_deno_task(cmd, extra_args)
    } else if let Some(script) = self.config.node_scripts.get(name) {
      // It's a Node script.
      self.exec_node_script(name, script, extra_args)
    } else if let Some(spec) = self.config.hooks.get(name) {
      // It's another hook; run its spec.
      self.run_spec(spec, name, extra_args)
    } else {
      // Unknown: treat as raw command.
      self.exec_raw_command(name, extra_args)
    };
    self.visiting.remove(name);
    result
  }

  /// Run a named task defined in either node_scripts or deno_tasks.
  pub(crate) fn run_named_task(
    &mut self,
    name: &str,
  ) -> Result<(), RunnerError> {
    self.run_single(name, &[])
  }

  /// Execute a raw shell command. Extra arguments from the hook invocation are
  /// appended.
  pub(crate) fn exec_raw_command(
    &self,
    cmd: &str,
    extra_args: &[String],
  ) -> Result<(), RunnerError> {
    // Compose the final command string. If there are extra args, append them.
    let mut full_cmd = cmd.to_string();
    if !extra_args.is_empty() {
      // Append each argument quoting as necessary (naive quoting: wrap in
      // single quotes if whitespace).
      for arg in extra_args {
        if arg.contains(' ') {
          full_cmd.push(' ');
          full_cmd.push_str(&format!("'{}'", arg.replace('"', "\\\"")));
        } else {
          full_cmd.push(' ');
          full_cmd.push_str(arg);
        }
      }
    }
    // Execute via sh -c.
    let status = Command::new("sh").arg("-c").arg(&full_cmd).status()?;
    if status.success() {
      Ok(())
    } else {
      Err(RunnerError::CommandFailure {
        cmd: full_cmd,
        status,
      })
    }
  }

  /// Execute a Deno task using `deno task`.
  pub(crate) fn exec_deno_task(
    &self,
    name: &str,
    extra_args: &[String],
  ) -> Result<(), RunnerError> {
    let mut cmd = Command::new("deno");
    cmd.arg("task").arg(name);
    for arg in extra_args {
      cmd.arg(arg);
    }
    let status = cmd.status()?;
    if status.success() {
      Ok(())
    } else {
      Err(RunnerError::CommandFailure {
        cmd: format!("deno task {name}"),
        status,
      })
    }
  }

  /// Execute a Node script using the configured package manager.
  pub(crate) fn exec_node_script(
    &self,
    name: &str,
    _script: &str,
    extra_args: &[String],
  ) -> Result<(), RunnerError> {
    // Determine the package manager. Parse something like "pnpm@7.1.2" into
    // "pnpm".
    let manager = self.config.package_manager.as_deref().unwrap_or("npm");
    let exe_name = Self::extract_package_manager_command(manager);
    // Build the command: <pm> run <script> [-- <extra args>]
    let mut cmd = Command::new(&exe_name);
    cmd.arg("run").arg(name);
    // If there are extra arguments, insert -- to forward them to the script.
    if !extra_args.is_empty() {
      cmd.arg("--");
      for arg in extra_args {
        cmd.arg(arg);
      }
    }
    let status = cmd.status()?;
    if status.success() {
      Ok(())
    } else {
      Err(RunnerError::CommandFailure {
        cmd: format!("{exe_name} run {name}"),
        status,
      })
    }
  }

  /// Extract the binary name from a packageManager field value. For example,
  /// `"pnpm@9.1.4"` becomes `"pnpm"`. If the value is simply a bare name
  /// without a version it will be returned unchanged.
  fn extract_package_manager_command(pm: &str) -> String {
    // The packageManager string may look like "npm@10.8.1". Split at '@'.
    let parts: Vec<&str> = pm.split('@').collect();
    let pm = parts[0].to_lowercase();
    let pm = pm.trim();
    match pm {
      "npm" | "yarn" | "pnpm" | "bun" => pm.to_string(),
      _ => "npm".to_string(),
    }
  }
}
