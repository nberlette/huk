//! Configuration discovery and parsing.
//!
//! This module contains logic for locating and parsing configuration files
//! that define hooks and tasks. The utility prefers a `deno.json` or
//! `deno.jsonc` file, but will fall back to `package.json` when no hooks are
//! present in the Deno config. The chosen file is inspected for a top-level
//! `hooks` object mapping Git hook names to task specifications. In
//! addition, the Node `scripts` field and Deno `tasks` field are captured
//! so that tasks can reference them.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use derive_more::IsVariant;
use moos::CowStr;
use serde_json::Value;
use serde_json::{self};
use thiserror::Error;

use crate::constants::GIT_HOOKS;
use crate::handlers::RunnerError;
use crate::task::TaskSpec;
use crate::task::TaskSpecParseError;

/// A resolved configuration containing hook definitions and tasks.
#[derive(Debug, Clone)]
pub struct HookConfig {
  /// Path to the configuration file used (either deno.json/deno.jsonc or
  /// package.json).
  #[allow(dead_code)]
  pub source:          ConfigSource,
  /// Mapping of hook names (e.g. "pre-commit") to their task specification.
  pub hooks:           HashMap<String, TaskSpec>,
  /// Mapping of task names to script commands coming from the Node `scripts`
  /// field.
  pub node_scripts:    HashMap<String, CowStr<'static>>,
  /// Mapping of task names to Deno task specifications from the `tasks` field.
  pub deno_tasks:      HashMap<String, TaskSpec>,
  /// The preferred package manager to use when executing Node scripts (npm,
  /// pnpm, yarn, etc.).
  pub package_manager: Option<String>,
}

impl<'a> HookConfig
where
  Self: 'a,
{
  pub(crate) fn tasks(&'a self) -> Vec<(CowStr<'a>, TaskSpec)> {
    let mut specs: Vec<(CowStr<'a>, TaskSpec)> = vec![];

    for (s, t) in &self.deno_tasks {
      specs.push((s.as_str().into(), t.clone()));
    }

    for (s, t) in &self.node_scripts {
      let spec = TaskSpec::Single(t.clone());
      specs.push((s.as_str().into(), spec))
    }

    specs.sort_by_key(|(k, _)| (*k).to_ascii_lowercase());
    specs
  }
}

/// Enum describing where the configuration was loaded from.
#[derive(Debug, Clone, IsVariant)]
pub enum ConfigSource {
  DenoJson(PathBuf),
  PackageJson(PathBuf),
  #[cfg(feature = "cargo_toml_config")]
  CargoToml(PathBuf),
  #[cfg(feature = "custom_config")]
  Custom(PathBuf),
}

impl ConfigSource {
  /// Get a [`PathBuf`] reference to the configuration file.
  pub const fn as_path_buf(&self) -> &PathBuf {
    match self {
      ConfigSource::DenoJson(p) => p,
      ConfigSource::PackageJson(p) => p,
      #[cfg(feature = "cargo_toml_config")]
      ConfigSource::CargoToml(p) => p,
      #[cfg(feature = "custom_config")]
      ConfigSource::Custom(p) => p,
    }
  }

  /// Get a [`Path`] reference to the configuration file.
  pub fn as_path(&self) -> &Path {
    match self {
      ConfigSource::DenoJson(p) => p.as_path(),
      ConfigSource::PackageJson(p) => p.as_path(),
      #[cfg(feature = "cargo_toml_config")]
      ConfigSource::CargoToml(p) => p.as_path(),
      #[cfg(feature = "custom_config")]
      ConfigSource::Custom(p) => p.as_path(),
    }
  }

  /// Get the default file name for the configuration file based on its type.
  fn default_file_name(&self) -> &str {
    match self {
      ConfigSource::DenoJson(_) => "deno.json",
      ConfigSource::PackageJson(_) => "package.json",
      #[cfg(feature = "cargo_toml_config")]
      ConfigSource::CargoToml(_) => "Cargo.toml",
      #[cfg(feature = "custom_config")]
      ConfigSource::Custom(_) => ".hukrc",
    }
  }

  /// Get the file name of the configuration file.
  #[allow(dead_code)]
  pub fn file_name(&self) -> &str {
    self
      .as_path_buf()
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or_else(|| self.default_file_name())
  }

  /// Get a string representation of the configuration source.
  pub fn as_str(&self) -> &str {
    self
      .as_path()
      .to_str()
      .unwrap_or_else(|| self.default_file_name())
  }
}

/// Errors that may occur while loading configuration.
#[derive(Error, Debug)]
pub enum ConfigError {
  /// No supported configuration file could be found.
  #[error(
    "no supported configuration file (deno.json, deno.jsonc, package.json) found in {0}"
  )]
  NotFound(PathBuf),
  /// Failed to read the configuration file.
  #[error("failed to read config file {0}: {1}")]
  Io(PathBuf, #[source] std::io::Error),
  /// Failed to parse JSON from the configuration file.
  #[error("failed to parse JSON from {0}: {1}")]
  Json(PathBuf, #[source] serde_json::Error),
  /// The hooks field exists but could not be parsed into a task specification.
  #[error("invalid hook definition for '{0}': {1}")]
  InvalidHook(String, #[source] TaskSpecParseError),
  /// A Deno task definition could not be parsed or validated.
  #[error("invalid task definition for '{0}': {1}")]
  InvalidTask(String, String),
  /// An unknown or unsupported Git hook name was specified.
  #[error("unknown Git hook name '{0}'. Supported hooks are: {supported_hooks}", supported_hooks = GIT_HOOKS.join(", "))]
  UnknownHook(String),
}

impl HookConfig {
  /// Discover and load a configuration from the specified directory. The search
  /// order is `deno.json`, `deno.jsonc`, then `package.json`, but if a Deno
  /// config exists without hooks and a package.json with hooks is present, the
  /// package.json is preferred. If none of these exist, returns
  /// [`ConfigError::NotFound`].
  pub fn discover(dir: &Path) -> Result<Self, ConfigError> {
    let deno_json = dir.join("deno.json");
    let deno_jsonc = dir.join("deno.jsonc");
    let package_json = dir.join("package.json");

    let deno_path = if deno_json.exists() {
      Some(deno_json)
    } else if deno_jsonc.exists() {
      Some(deno_jsonc)
    } else {
      None
    };

    if let Some(deno_path) = deno_path {
      let deno_has_hooks = Self::config_has_hooks(&deno_path, true)?;
      if deno_has_hooks {
        return Self::load_deno_json(&deno_path);
      }
      if package_json.exists() && Self::config_has_hooks(&package_json, false)?
      {
        return Self::load_package_json(&package_json);
      }
      return Self::load_deno_json(&deno_path);
    }

    if package_json.exists() {
      Self::load_package_json(&package_json)
    } else {
      Err(ConfigError::NotFound(dir.to_path_buf()))
    }
  }

  /// Load configuration from a Deno JSON or JSONC file.
  fn load_deno_json(path: &Path) -> Result<Self, ConfigError> {
    let content = fs::read_to_string(path)
      .map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
    // Remove comments if it's JSONC. We'll remove both line and block comments.
    let clean = strip_json_comments(&content);
    let value: Value = serde_json::from_str(&clean)
      .map_err(|e| ConfigError::Json(path.to_path_buf(), e))?;
    // Extract hooks mapping.
    let hooks_value = value.get("hooks").cloned().unwrap_or(Value::Null);
    let mut hooks = HashMap::new();
    if let Value::Object(map) = hooks_value {
      for (hook_name, spec_value) in map {
        if !GIT_HOOKS.contains(&&*hook_name) {
          return Err(ConfigError::UnknownHook(hook_name));
        }
        match TaskSpec::from_json(&spec_value) {
          Ok(spec) => {
            hooks.insert(hook_name, spec);
          }
          Err(err) => {
            return Err(ConfigError::InvalidHook(hook_name, err));
          }
        }
      }
    }
    // Extract deno tasks (strings or structured TaskSpec objects).
    let mut deno_tasks = HashMap::new();
    if let Some(Value::Object(tasks)) = value.get("tasks") {
      for (name, val) in tasks {
        let spec = Self::parse_deno_task(name, val)?;
        deno_tasks.insert(name.clone(), spec);
      }
    }
    Self::validate_deno_task_dependencies(&deno_tasks)?;
    Ok(HookConfig {
      source: ConfigSource::DenoJson(path.to_path_buf()),
      hooks,
      node_scripts: HashMap::new(),
      deno_tasks,
      package_manager: None,
    })
  }

  /// Load configuration from a Node package.json file.
  fn load_package_json(path: &Path) -> Result<Self, ConfigError> {
    let content = fs::read_to_string(path)
      .map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
    let value: Value = serde_json::from_str(content.trim())
      .map_err(|e| ConfigError::Json(path.to_path_buf(), e))?;
    // Extract hooks mapping.
    let hooks_value = value.get("hooks").cloned().unwrap_or(Value::Null);
    let mut hooks = HashMap::new();
    if let Value::Object(map) = hooks_value {
      for (hook_name, spec_value) in map {
        if !GIT_HOOKS.contains(&&*hook_name) {
          return Err(ConfigError::UnknownHook(hook_name));
        }
        match TaskSpec::from_json(&spec_value) {
          Ok(spec) => {
            hooks.insert(hook_name, spec);
          }
          Err(err) => {
            return Err(ConfigError::InvalidHook(hook_name, err));
          }
        }
      }
    }
    // Extract Node scripts.
    let mut node_scripts = HashMap::new();
    if let Some(Value::Object(scripts)) = value.get("scripts") {
      for (name, val) in scripts {
        if let Value::String(cmd) = val {
          node_scripts.insert(name.clone(), CowStr::from(cmd.clone()));
        }
      }
    }

    // Determine preferred package manager.
    let package_manager = value
      .get("packageManager")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    Ok(HookConfig {
      source: ConfigSource::PackageJson(path.to_path_buf()),
      hooks,
      node_scripts,
      deno_tasks: HashMap::new(),
      package_manager,
    })
  }

  fn config_has_hooks(path: &Path, is_deno: bool) -> Result<bool, ConfigError> {
    let content = fs::read_to_string(path)
      .map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
    let content = if is_deno {
      strip_json_comments(&content)
    } else {
      content
    };
    let value: Value = serde_json::from_str(&content)
      .map_err(|e| ConfigError::Json(path.to_path_buf(), e))?;
    Ok(matches!(value.get("hooks"), Some(Value::Object(_))))
  }

  fn parse_deno_task(
    name: &str,
    value: &Value,
  ) -> Result<TaskSpec, ConfigError> {
    if let Value::Object(map) = value
      && let Some(deps_value) =
        map.get("dependencies").or_else(|| map.get("depends"))
      && !matches!(deps_value, Value::Array(_))
    {
      return Err(ConfigError::InvalidTask(
        name.to_string(),
        "dependencies must be an array of strings".into(),
      ));
    }
    TaskSpec::from_json(value).map_err(|err| {
      ConfigError::InvalidTask(name.to_string(), err.to_string())
    })
  }

  fn validate_deno_task_dependencies(
    tasks: &HashMap<String, TaskSpec>,
  ) -> Result<(), ConfigError> {
    for (name, spec) in tasks {
      if let TaskSpec::Detailed { dependencies, .. } = spec {
        for dep in dependencies {
          if !tasks.contains_key(dep.as_ref()) {
            return Err(ConfigError::InvalidTask(
              name.clone(),
              format!("dependency '{dep}' is not defined in tasks"),
            ));
          }
        }
      }
    }
    Ok(())
  }
}

/// Remove JavaScript-style comments from a JSON string.
///
/// This naive implementation removes `// ...` single-line comments and
/// `/* ... */` block comments. It does not handle edge cases like strings
/// containing comment markers. The intent is simply to allow JSONC files
/// commonly used for Deno configuration to parse as JSON. If comment markers
/// appear inside string literals this function may remove valid content.
pub(crate) fn strip_json_comments(input: &str) -> String {
  let mut output = String::with_capacity(input.len());
  let mut chars = input.chars().peekable();
  let mut in_string = false;
  let mut escaped = false;
  while let Some(c) = chars.next() {
    if in_string {
      output.push(c);
      if escaped {
        escaped = false;
      } else if c == '\\' {
        escaped = true;
      } else if c == '"' {
        in_string = false;
      }
      continue;
    }
    if c == '"' {
      in_string = true;
      output.push(c);
      continue;
    }
    if c == '/' {
      match chars.peek() {
        Some('/') => {
          // Skip until newline
          chars.next();
          for next in chars.by_ref() {
            if next == '\n' {
              break;
            }
          }
          output.push('\n');
        }
        Some('*') => {
          // Skip block comment
          chars.next();
          while let Some(next) = chars.next() {
            if next == '*'
              && let Some('/') = chars.peek()
            {
              chars.next();
              break;
            } else if next == '\n' {
              output.push('\n');
            }
          }
        }
        _ => output.push(c),
      }
    } else {
      output.push(c);
    }
  }
  output
}

/// Parse a task specification provided from the CLI or TUI. If the string looks
/// like JSON (starts with `{` or `[`), attempt to parse it accordingly;
/// otherwise treat it as a simple string command or task name.
pub(crate) fn parse_spec_input(spec: &str) -> Result<TaskSpec, RunnerError> {
  let trimmed = spec.trim();
  if trimmed.starts_with('{') || trimmed.starts_with('[') {
    let value: Value = serde_json::from_str(trimmed)?;
    TaskSpec::from_json(&value).map_err(RunnerError::InvalidTaskSpec)
  } else {
    Ok(TaskSpec::Single(CowStr::from(trimmed.to_string())))
  }
}

/// Parse one or more task specifications. If multiple values are supplied they
/// are combined into a sequence. Individual values may themselves be JSON
/// arrays, objects, or simple strings.
pub(crate) fn parse_specs_inputs(
  specs: &[String],
) -> Result<TaskSpec, RunnerError> {
  if specs.is_empty() {
    return Err(RunnerError::InvalidTaskSpec(
      TaskSpecParseError::InvalidType("empty spec".into()),
    ));
  }
  if specs.len() == 1 {
    return parse_spec_input(&specs[0]);
  }

  let mut sequence = Vec::new();
  for s in specs {
    let parsed = parse_spec_input(s)?;
    match parsed {
      TaskSpec::Sequence(list) => {
        // Flatten nested sequences for convenience.
        sequence.extend(list);
      }
      other => sequence.push(other),
    }
  }
  Ok(TaskSpec::Sequence(sequence))
}

/// Merge a new specification into an existing one. When `replace` is false,
/// the new spec is appended (flattening sequences) instead of overwriting.
pub(crate) fn merge_specs(
  existing: Option<&TaskSpec>,
  incoming: TaskSpec,
  replace: bool,
) -> TaskSpec {
  if replace {
    return incoming;
  }
  let Some(current) = existing else {
    return incoming;
  };

  let mut seq = match current {
    TaskSpec::Sequence(list) => list.clone(),
    other => vec![other.clone()],
  };

  match incoming {
    TaskSpec::Sequence(list) => seq.extend(list),
    other => seq.push(other),
  }

  TaskSpec::Sequence(seq)
}

pub(crate) fn remove_task_from_spec(
  current: &TaskSpec,
  target: &TaskSpec,
) -> Option<TaskSpec> {
  match current {
    TaskSpec::Single(_) | TaskSpec::Detailed { .. } => {
      if current == target {
        None
      } else {
        Some(current.clone())
      }
    }
    TaskSpec::Sequence(list) => {
      let mut next: Vec<TaskSpec> = list
        .iter()
        .filter_map(|item| remove_task_from_spec(item, target))
        .collect();
      if next.is_empty() {
        None
      } else if next.len() == 1 {
        Some(next.remove(0))
      } else {
        Some(TaskSpec::Sequence(next))
      }
    }
  }
}

pub(crate) fn ensure_valid_hook_name(hook: &str) -> Result<(), RunnerError> {
  if GIT_HOOKS.contains(&hook) {
    Ok(())
  } else {
    Err(ConfigError::UnknownHook(hook.to_string()).into())
  }
}

pub(crate) fn load_config_value(
  source: &ConfigSource,
) -> Result<Value, RunnerError> {
  let path = source.as_path();
  let content = fs::read_to_string(path)?;
  let content = match source {
    ConfigSource::DenoJson(_) => strip_json_comments(&content),
    ConfigSource::PackageJson(_) => content,
  };
  let value: Value = serde_json::from_str(&content)?;
  Ok(value)
}

// TODO(nberlette): implement support for arbitrary config files in
// different formats (JSON/JSONC, TOML, and YAML). right now we only
// allow deno.json{,c} or package.json files, but in the near duture
// we should allow the user to specify a custom config file path/type
// and (for advanced users) even specify a custom path within the file
// to the hooks and tasks maps. this would allow us to support Cargo.toml
// files out of the box and also our own custom .hukrc.{json,toml,yml}
// files too, if desired.

// pub(crate) fn load_json_config_value(
//   source: &ConfigSource,
// ) -> Result<Value, RunnerError> {
//   let path = source.as_path();
//   let content = fs::read_to_string(path)?;
//   let content = match source {
//     ConfigSource::DenoJson(_) => strip_json_comments(&content),
//     ConfigSource::PackageJson(_) => content,
//   };
//   let value: Value = serde_json::from_str(&content)?;
//   Ok(value)
// }

// pub(crate) fn load_toml_config_value(
//   source: &ConfigSource,
// ) -> Result<toml::Value, RunnerError> {
//   let path = source.as_path();
//   let content = fs::read_to_string(path)?;
//   let value: toml::Value = toml::from_str(&content)
//     .map_err(|e| RunnerError::Serialize(e.to_string()))?;
//   Ok(value)
// }

// pub(crate) fn load_yaml_config_value(
//   source: &ConfigSource,
// ) -> Result<serde_yaml::Value, RunnerError> {
//   let path = source.as_path();
//   let content = fs::read_to_string(path)?;
//   let value: serde_yaml::Value = serde_yaml::from_str(&content)
//     .map_err(|e| RunnerError::Serialize(e.to_string()))?;
//   Ok(value)
// }

pub(crate) fn write_config_value(
  source: &ConfigSource,
  value: &Value,
) -> Result<(), RunnerError> {
  let mut content = serde_json::to_string_pretty(value)?;
  content.push('\n');
  fs::write(source.as_path(), content)?;
  Ok(())
}

pub(crate) fn with_hooks_map<F>(
  value: &mut Value,
  source: &ConfigSource,
  mutator: F,
) -> Result<(), RunnerError>
where
  F: FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), RunnerError>,
{
  let obj = value.as_object_mut().ok_or_else(|| {
    RunnerError::InvalidConfigShape(source.as_str().to_string())
  })?;
  let hooks_value = obj
    .entry("hooks")
    .or_insert_with(|| Value::Object(serde_json::Map::new()));
  if !hooks_value.is_object() {
    *hooks_value = Value::Object(serde_json::Map::new());
  }
  if let Some(map) = hooks_value.as_object_mut() {
    mutator(map)?;
    sort_hooks(map);
    Ok(())
  } else {
    Err(RunnerError::InvalidConfigShape(source.as_str().to_string()))
  }
}

pub(crate) fn sort_hooks(map: &mut serde_json::Map<String, Value>) {
  let mut entries: Vec<(String, Value)> =
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
  entries.sort_by(|a, b| a.0.cmp(&b.0));
  map.clear();
  for (k, v) in entries {
    map.insert(k, v);
  }
}
