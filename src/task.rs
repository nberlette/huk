//! Task specification structures.
//!
//! This module defines the [`TaskSpec`] type which represents the parsed
//! definition of a task or set of tasks as found in the `hooks` section of
//! either `deno.json`/`deno.jsonc` or `package.json`. A task specification may
//! be a single string referencing a task name or shell command, an object
//! describing the command, description and dependencies, or an array of either
//! of those two forms.

use ::core::any::type_name_of_val;

use serde_json::Value;
use thiserror::Error;

/// Parsed representation of a task specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskSpec {
  /// A bare string representing a script or command to execute.
  Single(String),
  /// A detailed object specification containing a command and/or dependencies.
  Detailed {
    /// Optional shell command to run. If absent, at least one dependency must
    /// be provided.
    command:      Option<String>,
    /// Optional description for display purposes.
    description:  Option<String>,
    /// Names of tasks that this task depends on. These will be executed prior
    /// to this task.
    dependencies: Vec<String>,
  },
  /// A sequence of tasks. Each element may itself be either a single string or
  /// a detailed object.
  Sequence(Vec<TaskSpec>),
}

/// Errors that may occur while parsing a task specification from JSON.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TaskSpecParseError {
  /// The JSON value was of a type not supported for tasks.
  #[error("expected string, object or array but found {0}")]
  InvalidType(String),
  /// A task object did not specify a `command` or any `dependencies`.
  #[error(
    "object must specify either a 'command' or at least one 'dependencies' entry"
  )]
  MissingCommandAndDeps,
  /// A dependency entry was not a string.
  #[error("dependencies must be strings")]
  InvalidDependencyType,
}

impl TaskSpec {
  /// Parse a task specification from arbitrary JSON.
  pub fn from_json(value: &Value) -> Result<TaskSpec, TaskSpecParseError> {
    match value {
      Value::String(s) => Ok(TaskSpec::Single(s.clone())),
      Value::Object(map) => {
        let command = map
          .get("command")
          .or_else(|| map.get("cmd"))
          .and_then(|v| v.as_str().map(|s| s.to_string()));
        let description = map
          .get("description")
          .and_then(|v| v.as_str().map(|s| s.to_string()));
        let deps_value = map.get("dependencies");
        let mut dependencies = Vec::new();
        if let Some(Value::Array(dep_array)) = deps_value {
          for dep in dep_array {
            if let Value::String(name) = dep {
              dependencies.push(name.clone());
            } else {
              return Err(TaskSpecParseError::InvalidDependencyType);
            }
          }
        }
        if command.is_none() && dependencies.is_empty() {
          return Err(TaskSpecParseError::MissingCommandAndDeps);
        }
        Ok(TaskSpec::Detailed {
          command,
          description,
          dependencies,
        })
      }
      Value::Array(arr) => {
        let mut seq = Vec::with_capacity(arr.len());
        for item in arr {
          seq.push(TaskSpec::from_json(item)?);
        }
        Ok(TaskSpec::Sequence(seq))
      }
      other => {
        Err(TaskSpecParseError::InvalidType(type_name_of_val(&other).to_string()))
      }
    }
  }
}
