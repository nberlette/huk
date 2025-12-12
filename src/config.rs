//! Configuration discovery and parsing.
//!
//! This module contains logic for locating and parsing configuration files
//! that define hooks and tasks. The utility searches for a `deno.json` or
//! `deno.jsonc` file first; if none is found it will fall back to a
//! `package.json` file. The chosen file is inspected for a top-level
//! `hooks` object mapping Git hook names to task specifications. In
//! addition, the Node `scripts` field and Deno `tasks` field are captured
//! so that tasks can reference them.

use crate::constants::GIT_HOOKS;
use crate::task::TaskSpec;
use crate::task::TaskSpecParseError;
use ::derive_more::IsVariant;
use serde_json::Value;
use serde_json::{self};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;

/// A resolved configuration containing hook definitions and tasks.
#[derive(Debug, Clone)]
pub struct HookConfig {
  /// Path to the configuration file used (either deno.json/deno.jsonc or
  /// package.json).
  #[allow(dead_code)]
  pub source:          ConfigSource,
  /// Mapping of hook names (e.g. "pre-commit") to their task specification.
  pub hooks:           HashMap<String, TaskSpec>,
  /// Mapping of task names to raw commands coming from the Node `scripts`
  /// field.
  pub node_scripts:    HashMap<String, String>,
  /// Mapping of task names to raw commands coming from the Deno `tasks` field.
  pub deno_tasks:      HashMap<String, String>,
  /// The preferred package manager to use when executing Node scripts (npm,
  /// pnpm, yarn, etc.).
  pub package_manager: Option<String>,
}

/// Enum describing where the configuration was loaded from.
#[derive(Debug, Clone, IsVariant)]
pub enum ConfigSource {
  DenoJson(PathBuf),
  PackageJson(PathBuf),
}

impl ConfigSource {
  /// Get a [`PathBuf`] reference to the configuration file.
  pub const fn as_path_buf(&self) -> &PathBuf {
    match self {
      ConfigSource::DenoJson(p) => p,
      ConfigSource::PackageJson(p) => p,
    }
  }

  /// Get a [`Path`] reference to the configuration file.
  pub fn as_path(&self) -> &Path {
    match self {
      ConfigSource::DenoJson(p) => p.as_path(),
      ConfigSource::PackageJson(p) => p.as_path(),
    }
  }

  /// Get the file name of the configuration file.
  #[allow(dead_code)]
  pub fn file_name(&self) -> &str {
    match self {
      ConfigSource::DenoJson(p) => p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deno.json"),
      ConfigSource::PackageJson(p) => p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("package.json"),
    }
  }

  /// Get a string representation of the configuration source.
  pub fn as_str(&self) -> &str {
    self.as_path().to_str().unwrap_or("")
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
  /// An unknown or unsupported Git hook name was specified.
  #[error("unknown Git hook name '{0}'. Supported hooks are: {supported_hooks}", supported_hooks = GIT_HOOKS.join(", "))]
  UnknownHook(String),
}

impl HookConfig {
  /// Discover and load a configuration from the specified directory. The search
  /// order is `deno.json`, `deno.jsonc`, then `package.json`. If none of
  /// these exist, returns [`ConfigError::NotFound`].
  pub fn discover(dir: &Path) -> Result<Self, ConfigError> {
    let deno_json = dir.join("deno.json");
    let deno_jsonc = dir.join("deno.jsonc");
    let package_json = dir.join("package.json");

    if deno_json.exists() {
      Self::load_deno_json(&deno_json)
    } else if deno_jsonc.exists() {
      Self::load_deno_json(&deno_jsonc)
    } else if package_json.exists() {
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
    // Extract deno tasks (these are simple command strings in Deno).
    let mut deno_tasks = HashMap::new();
    if let Some(Value::Object(tasks)) = value.get("tasks") {
      for (name, val) in tasks {
        match val {
          Value::String(cmd) => {
            deno_tasks.insert(name.clone(), cmd.clone());
          }
          // Deno tasks may also be objects with command/description etc.
          Value::Object(obj) => {
            let mut cmd_parts = Vec::new();
            if let Some(Value::Array(deps)) = obj.get("dependencies") {
              // If only dependencies are defined, we can join them with "&&".
              for dep in deps {
                if let Value::String(task) = dep {
                  cmd_parts.push(format!("deno task {task}"));
                }
              }
            }
            if let Some(Value::String(cmd)) = obj.get("command") {
              cmd_parts.push(cmd.clone());
            }
            let joined = cmd_parts.join(" && ");
            deno_tasks.insert(name.clone(), joined);
          }
          _ => {}
        }
      }
    }
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
    let value: Value = serde_json::from_str(&content.trim())
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
          node_scripts.insert(name.clone(), cmd.clone());
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
}

/// Remove JavaScript-style comments from a JSON string.
///
/// This naive implementation removes `// ...` single-line comments and
/// `/* ... */` block comments. It does not handle edge cases like strings
/// containing comment markers. The intent is simply to allow JSONC files
/// commonly used for Deno configuration to parse as JSON. If comment markers
/// appear inside string literals this function may remove valid content.
fn strip_json_comments(input: &str) -> String {
  let mut output = String::with_capacity(input.len());
  let mut chars = input.chars().peekable();
  while let Some(c) = chars.next() {
    if c == '/' {
      match chars.peek() {
        Some('/') => {
          // Skip until newline
          chars.next();
          while let Some(next) = chars.next() {
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
            if next == '*' {
              if let Some('/') = chars.peek() {
                chars.next();
                break;
              }
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
