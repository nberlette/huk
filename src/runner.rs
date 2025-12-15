//! Execution logic for hooks and tasks.
//!
//! This module contains the implementations of the various CLI handlers
//! responsible for listing, running and modifying hooks. It delegates
//! configuration loading to [`crate::config`] and executes commands via
//! [`std::process::Command`].

use std::collections::HashSet;
use std::io;
use std::process::Command;
use std::process::ExitStatus;

use ::derive_more::IsVariant;
use ::serde_json::json;
use moos::CowStr;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::GIT_HOOKS;
use crate::cli::AddOpts;
use crate::cli::ListOpts;
use crate::cli::RemoveOpts;
use crate::cli::RunOpts;
use crate::cli::TaskOpts;
use crate::cli::UpdateOpts;
use crate::config::*;
use crate::task::TaskSpec;
use crate::task::TaskSpecParseError;

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
  /// Failed to parse a task specification provided via CLI/TUI.
  #[error("failed to parse task specification: {0}")]
  InvalidTaskSpec(#[from] TaskSpecParseError),
  /// Invalid JSON was provided for a task specification.
  #[error("invalid JSON for task specification: {0}")]
  InvalidSpecJson(#[from] serde_json::Error),
  /// Failed to serialize output.
  #[error("failed to serialize output: {0}")]
  Serialize(String),
  /// The configuration file is not structured as an object with a hooks map.
  #[error(
    "configuration file '{0}' is not a JSON object; unable to modify hooks"
  )]
  InvalidConfigShape(String),
}

/// Handler for the `list` subcommand.
pub fn handle_list(opts: &ListOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  let default_spec = TaskSpec::Single("<undefined>".into());
  let mut hooks_sorted: Vec<(&str, &TaskSpec)> = if opts.all {
    GIT_HOOKS
      .iter()
      .map(|hook| {
        let hook_str = *hook;
        let spec = cfg.hooks.get(hook_str).unwrap_or(&default_spec);
        (hook_str, spec)
      })
      .collect()
  } else {
    cfg.hooks.iter().map(|(k, v)| (k.as_str(), v)).collect()
  };
  hooks_sorted.sort_by(|a, b| a.0.cmp(b.0));

  let n = hooks_sorted.len();
  let path = cfg.source.as_path_buf().display().to_string();

  // machine-readable output formats (JSON, YAML, TOML)
  if opts.json || opts.toml || opts.yaml {
    #[derive(Serialize, IsVariant)]
    #[serde(untagged)]
    enum HookEntry {
      Name(String),
      Full { name: String, spec: Option<Value> },
    }

    let mut hooks: Vec<HookEntry>;

    if opts.all {
      hooks = GIT_HOOKS
        .iter()
        .map(|hook| HookEntry::Name(hook.to_string()))
        .collect();
      use HookEntry::*;
      hooks.sort_by(|a, b| match (a, b) {
        // we know we are only dealing with Name variants here
        (Name(a), Name(b)) => a.cmp(b),
        // so we can safely discard all other cases
        (_, _) => std::cmp::Ordering::Equal,
      })
    } else {
      hooks = hooks_sorted
        .iter()
        .map(|(hook, spec)| HookEntry::Full {
          name: hook.to_string(),
          spec: if opts.name_only {
            None
          } else {
            Some(spec.to_json())
          },
        })
        .collect();
    }

    let mut payload: serde_json::value::Value = json!({
      "source": path,
      "hooks": hooks,
    });

    if opts.name_only {
      payload = json!(
        hooks
          .into_iter()
          .map(|h| match h {
            HookEntry::Name(name) => name,
            HookEntry::Full { name, .. } => name,
          })
          .collect::<Vec<String>>()
      );
    }

    if opts.json {
      let out = if opts.compact {
        serde_json::to_string(&payload)
      } else {
        serde_json::to_string_pretty(&payload)
      }
      .map_err(RunnerError::InvalidSpecJson)?;
      println!("{out}");
      return Ok(());
    }

    if opts.yaml {
      // serde_yaml does not have a pretty vs compact option unfortunately ;(
      let out = serde_yaml::to_string(&payload)
        .map_err(|e| RunnerError::Serialize(e.to_string()))?;
      println!("{out}");
      return Ok(());
    }

    if opts.toml {
      let out = if opts.compact {
        toml::to_string(&payload)
      } else {
        toml::to_string_pretty(&payload)
      }
      .map_err(|e| RunnerError::Serialize(e.to_string()))?;
      println!("{out}");
      return Ok(());
    }
  }

  // all the human-readable output logic is below
  if n == 0 {
    eprintln!("No hooks found in '{path}'.");
    return Ok(());
  }
  let s = if n == 1 { "" } else { "s" };
  if !opts.all {
    eprintln!("Discovered {n} hook{s} in '{path}':");
    eprintln!();
  }
  let mut i = 0;
  for (hook, spec) in hooks_sorted {
    if i != 0 && !opts.all && !opts.compact && !opts.name_only {
      eprintln!();
    }
    i += 1;
    if opts.name_only || opts.all {
      println!("- {hook}");
      continue;
    }
    if opts.compact {
      println!("- {hook}");
      let spec_str = spec.to_string();
      let _info: String = spec_str.replace('\n', "\n  ");
    } else {
      eprintln!(
        r#"- {cyan}{hook}{reset}"#,
        cyan = "\x1b[1;36m",
        reset = "\x1b[0m"
      );

      let spec_str = spec.to_string();
      let info: String = spec_str.replace('\n', "\n  ");

      // if each line of info starts with a number and colon, e.g. `1: ...\n2:
      // ...\n`, we want to indent accordingly and colorize the numbers to
      // be dimmer than the actual command text.
      if info.lines().all(|line| {
        line
          .trim_start()
          .chars()
          .next()
          .is_some_and(|c| c.is_ascii_digit() || c == '-')
      }) {
        let mut lines: Vec<&str> = info.lines().collect::<Vec<&str>>();
        let tmp = lines.drain(..);
        for line in tmp {
          let trimmed: &str = line.trim_start();
          let (num_part, rest): (&str, &str) =
            trimmed.split_at(trimmed.find(' ').unwrap_or(trimmed.len()));
          let num_part = num_part.trim_end_matches(':').trim_end_matches('.');
          let num_part = format!("{num_part}.");

          // in the rest part, attempt to resolve the task name to its
          // configured command, printing them on the same line as the
          // number, but dimming the number part and emboldening/
          // underlining the task name. for tasks that appear to be a shell
          // script/command themselves, just print them as is.
          let rest = rest.trim().to_string();
          let info = if cfg.node_scripts.contains_key(&rest)
            || cfg.deno_tasks.contains_key(&rest)
          {
            let (_kind, cmd): (CowStr, CowStr) =
              if let Some(script) = cfg.node_scripts.get(&rest) {
                (CowStr::from("script"), CowStr::from(script.as_str()))
              } else if let Some(script) = cfg.deno_tasks.get(&rest) {
                (CowStr::from("task"), CowStr::from(script.as_str()))
              } else {
                (CowStr::from("unknown"), "<unknown>".into())
              };
            let named = rest.clone();
            let cmd = cmd.replace('\n', " ");
            format!(
              r#"{bun}{magenta}{named}{reset}{indent}{cmd}{reset}"#,
              bun = "\x1b[1;4m",
              magenta = "\x1b[35m",
              reset = "\x1b[0m",
              indent = "\n     \x1b[2m↪︎\x1b[0m \x1b[3m"
            )
          } else {
            rest
          };

          eprintln!(
            r#"  {dim}{num_part}{reset} {info}"#,
            dim = "\x1b[2m",
            reset = "\x1b[0m"
          );
        }
        continue;
      }
      eprintln!("  {info}");
      continue;
    }
  }
  Ok(())
}

/// Handler for the `run` subcommand.
pub fn handle_run(opts: &RunOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  if opts.hook.is_empty() {
    eprintln!("Please specify a valid hook name.");
    if opts.verbose && !cfg.hooks.is_empty() {
      crate::print_available_hooks!(&cfg);
    }
    return Err(ConfigError::UnknownHook(opts.hook.clone()).into());
  }
  if opts.hook.is_empty() || !GIT_HOOKS.contains(&&*opts.hook) {
    return Err(ConfigError::UnknownHook(opts.hook.clone()).into());
  }
  if let Some(spec) = cfg.hooks.get(&opts.hook) {
    let mut runner = TaskRunner::new(&cfg);
    runner.run_spec(spec, &opts.hook, &opts.args)?;
  } else {
    let path = cfg.source.as_path_buf().display().to_string();
    eprintln!("Hook '{}' is not defined in {path}.", opts.hook);
    if opts.verbose && !cfg.hooks.is_empty() {
      crate::print_available_hooks!(&cfg);
    }
  }
  Ok(())
}

/// Handler for the `tasks` subcommand.
pub fn handle_task(opts: &TaskOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  // Collect all task names from node_scripts and deno_tasks.
  let mut all_tasks: HashSet<String> = HashSet::new();
  all_tasks.extend(cfg.node_scripts.keys().cloned());
  all_tasks.extend(cfg.deno_tasks.keys().cloned());

  let path = cfg.source.as_path_buf();

  if let Some(ref run_task) = opts.run {
    if all_tasks.contains(run_task) {
      let mut runner = TaskRunner::new(&cfg);
      runner.run_named_task(run_task)?;
    } else {
      eprintln!(
        "There is no '{run_task}' task defined in '{path}'.",
        path = path.display()
      );
    }
  } else {
    #[derive(Serialize)]
    struct TaskEntry {
      name:    String,
      command: String,
      #[serde(rename = "type")]
      kind:    String,
    }

    #[derive(Serialize)]
    struct TaskList {
      source: String,
      tasks:  Vec<TaskEntry>,
    }

    let mut tasks: Vec<TaskEntry> = cfg
      .deno_tasks
      .iter()
      .map(|(name, cmd)| TaskEntry {
        name:    name.clone(),
        command: cmd.clone(),
        kind:    "task".into(),
      })
      .collect();

    tasks.extend(cfg.node_scripts.iter().map(|(name, cmd)| TaskEntry {
      name:    name.clone(),
      command: cmd.clone(),
      kind:    "script".into(),
    }));

    tasks.sort_by(|a, b| {
      a.name
        .cmp(&b.name)
        .then_with(|| a.kind.cmp(&b.kind))
        .then_with(|| a.command.cmp(&b.command))
    });

    if opts.json || opts.yaml || opts.toml {
      let payload = TaskList {
        source: path.display().to_string(),
        tasks,
      };

      if opts.json {
        let out = if opts.compact {
          serde_json::to_string(&payload)
        } else {
          serde_json::to_string_pretty(&payload)
        }
        .map_err(RunnerError::InvalidSpecJson)?;
        println!("{out}");
        return Ok(());
      }

      if opts.yaml {
        let out = serde_yaml::to_string(&payload)
          .map_err(|e| RunnerError::Serialize(e.to_string()))?;
        println!("{out}");
        return Ok(());
      }

      if opts.toml {
        let out = if opts.compact {
          toml::to_string(&payload)
        } else {
          toml::to_string_pretty(&payload)
        }
        .map_err(|e| RunnerError::Serialize(e.to_string()))?;
        println!("{out}");
        return Ok(());
      }
    }

    crate::print_tasks!(&cfg, compact = opts.compact);
  }
  Ok(())
}

pub(crate) fn mutate_hooks<F>(
  cfg: &HookConfig,
  mutator: F,
) -> Result<(), RunnerError>
where
  F: FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), RunnerError>,
{
  let mut value = load_config_value(&cfg.source)?;
  with_hooks_map(&mut value, &cfg.source, mutator)?;
  write_config_value(&cfg.source, &value)?;
  Ok(())
}

/// Handler for the `add` subcommand.
pub fn handle_add(opts: &AddOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  ensure_valid_hook_name(&opts.hook)?;

  let spec = parse_specs_inputs(&opts.spec)?;
  let merged = merge_specs(cfg.hooks.get(&opts.hook), spec, opts.replace);

  mutate_hooks(&cfg, |hooks| {
    hooks.insert(opts.hook.clone(), merged.to_json());
    Ok(())
  })?;

  if cfg.hooks.contains_key(&opts.hook) && !opts.replace {
    eprintln!(
      "Appended to hook '{}' in {}.",
      opts.hook,
      cfg.source.as_str()
    );
  } else {
    eprintln!("Added hook '{}' to {}.", opts.hook, cfg.source.as_str());
  }
  Ok(())
}

/// Handler for the `remove` subcommand.
pub fn handle_remove(opts: &RemoveOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  ensure_valid_hook_name(&opts.hook)?;
  let Some(existing) = cfg.hooks.get(&opts.hook) else {
    if !opts.force {
      eprintln!(
        "Hook '{}' is not currently defined in {}.",
        opts.hook,
        cfg.source.as_str()
      );
    }
    return Ok(());
  };

  // If a spec guard is provided, ensure it matches.
  if let Some(spec_str) = &opts.spec {
    let guard = parse_spec_input(spec_str)?;
    if &guard != existing {
      if !opts.force {
        eprintln!(
          "Hook '{}' does not match the provided spec; skipping removal.",
          opts.hook
        );
      }
      return Ok(());
    }
  }

  let mut removed = false;
  mutate_hooks(&cfg, |hooks| {
    if let Some(task_str) = &opts.task {
      let target = parse_spec_input(task_str)?;
      if let Some(current) = hooks.get(&opts.hook).cloned() {
        let parsed_current = TaskSpec::from_json(&current)
          .map_err(RunnerError::InvalidTaskSpec)?;
        if let Some(next_spec) = remove_task_from_spec(&parsed_current, &target)
        {
          hooks.insert(opts.hook.clone(), next_spec.to_json());
        } else {
          hooks.remove(&opts.hook);
        }
        removed = true;
      }
      Ok(())
    } else {
      hooks.remove(&opts.hook);
      removed = true;
      Ok(())
    }
  })?;

  if removed {
    if opts.task.is_some() {
      eprintln!(
        "Updated hook '{}' in {} (task removed).",
        opts.hook,
        cfg.source.as_str()
      );
    } else {
      eprintln!("Removed hook '{}' from {}.", opts.hook, cfg.source.as_str());
    }
  } else if !opts.force {
    eprintln!("Task not found in hook '{}'; no changes made.", opts.hook);
  }
  Ok(())
}

/// Handler for the `update` subcommand.
pub fn handle_update(opts: &UpdateOpts) -> Result<(), RunnerError> {
  let cfg = HookConfig::discover(&std::env::current_dir()?)?;
  ensure_valid_hook_name(&opts.hook)?;
  if !cfg.hooks.contains_key(&opts.hook) {
    eprintln!(
      "Hook '{}' is not currently defined in {}. Use `huk add` to create it.",
      opts.hook,
      cfg.source.as_str()
    );
    return Ok(());
  }

  let spec = parse_specs_inputs(&opts.spec)?;
  let merged = merge_specs(cfg.hooks.get(&opts.hook), spec, opts.replace);
  mutate_hooks(&cfg, |hooks| {
    hooks.insert(opts.hook.clone(), merged.to_json());
    Ok(())
  })?;

  let verb = if opts.replace { "Replaced" } else { "Updated" };
  eprintln!("{verb} hook '{}' in {}.", opts.hook, cfg.source.as_str());
  Ok(())
}

/// A stateful task runner responsible for executing task specifications.
pub struct TaskRunner<'cfg> {
  pub config: &'cfg HookConfig,
  visiting:   HashSet<String>,
  /// Optional buffer for capturing stdout/stderr when running via the TUI.
  pub output: Option<Vec<OutputChunk>>,
}

impl<'cfg> TaskRunner<'cfg> {
  pub fn new(config: &'cfg HookConfig) -> Self {
    Self {
      config,
      visiting: HashSet::new(),
      output: None,
    }
  }

  pub fn new_with_capture(config: &'cfg HookConfig) -> Self {
    Self {
      config,
      visiting: HashSet::new(),
      output: Some(Vec::new()),
    }
  }

  /// Retrieve captured output if output capture is enabled.
  pub fn take_output(&mut self) -> Vec<OutputChunk> {
    self.output.take().unwrap_or_default()
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
    let result = if self.config.deno_tasks.get(name).is_some() {
      // It's a Deno task.
      self.exec_deno_task(name, extra_args)
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
    &mut self,
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
    let mut command = Command::new("sh");
    command.arg("-c").arg(&full_cmd);
    self.spawn_command(command, full_cmd)
  }

  /// Execute a Deno task using `deno task`.
  pub(crate) fn exec_deno_task(
    &mut self,
    name: &str,
    extra_args: &[String],
  ) -> Result<(), RunnerError> {
    let mut cmd = Command::new("deno");
    cmd.arg("task").arg(name);
    for arg in extra_args {
      cmd.arg(arg);
    }
    self.spawn_command(cmd, format!("deno task {name}"))
  }

  /// Execute a Node script using the configured package manager.
  pub(crate) fn exec_node_script(
    &mut self,
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
    self.spawn_command(cmd, format!("{exe_name} run {name}"))
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

  /// Spawn the command either streaming output directly or capturing
  /// stdout/stderr when an output buffer is present.
  fn spawn_command(
    &mut self,
    mut cmd: Command,
    display: String,
  ) -> Result<(), RunnerError> {
    if let Some(buf) = self.output.as_mut() {
      let output = cmd.output()?;
      if !output.stdout.is_empty() {
        buf.push(OutputChunk::Stdout(
          String::from_utf8_lossy(&output.stdout).to_string(),
        ));
      }
      if !output.stderr.is_empty() {
        buf.push(OutputChunk::Stderr(
          String::from_utf8_lossy(&output.stderr).to_string(),
        ));
      }
      if output.status.success() {
        Ok(())
      } else {
        Err(RunnerError::CommandFailure {
          cmd:    display,
          status: output.status,
        })
      }
    } else {
      let status = cmd.status()?;
      if status.success() {
        Ok(())
      } else {
        Err(RunnerError::CommandFailure {
          cmd: display,
          status,
        })
      }
    }
  }
}

/// Captured output from a task execution, used primarily by the TUI dashboard.
#[derive(Clone, Debug)]
pub enum OutputChunk {
  Stdout(String),
  Stderr(String),
}
