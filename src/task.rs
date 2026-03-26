//! Task specification structures.
//!
//! This module defines the [`TaskSpec`] type which represents the parsed
//! definition of a task or set of tasks as found in the `hooks` section of
//! either `deno.json`/`deno.jsonc` or `package.json`. A task specification may
//! be a single string referencing a task name or shell command, an object
//! describing the command, description and dependencies, or an array of either
//! of those two forms.

use core::any::type_name_of_val;
use core::str::FromStr;

use derive_more::with_trait::Debug;
use derive_more::with_trait::Display;
use derive_more::with_trait::From;
use derive_more::with_trait::IsVariant;
use derive_more::with_trait::TryFrom;
use moos::CowStr;
use serde_json::Value;
use thiserror::Error;

use crate::runner::RunnerError;

/// Parsed representation of a task specification.
#[derive(Display, Clone, PartialEq, Eq, IsVariant, From, TryFrom)]
#[try_from(repr)]
pub enum TaskSpec {
  /// A bare string representing a script or command to execute.
  #[display("{_0}")]
  Single(CowStr<'static>),

  /// A detailed object specification containing a command and/or dependencies.
  #[display("{command}{info}{deps}", command = if let Some(cmd) = command { format!("{cmd}\n") } else { "".to_string() }, info = if let Some(desc) = description { format!("   // {desc}\n") } else { "".to_string() }, deps = if !dependencies.is_empty() { format!("   depends on: {}", dependencies.iter().map(|dep| dep.as_ref()).collect::<Vec<_>>().join(", ")) } else { "".to_string() })]
  Detailed {
    /// Optional shell command to run. If absent, at least one dependency must
    /// be provided.
    command:      Option<CowStr<'static>>,
    /// Optional description for display purposes.
    description:  Option<CowStr<'static>>,
    /// Names of tasks that this task depends on. These will be executed prior
    /// to this task.
    dependencies: Vec<CowStr<'static>>,
  },

  /// A sequence of tasks. Each element may itself be either a single string or
  /// a detailed object.
  #[display("{tasks}", tasks = {
    let mut i = 0;
    _0.iter().map(|t| {
      i += 1;
      format!("{i}. {t}")
    }).collect::<Vec<_>>().join("\n")
  })]
  Sequence(Vec<TaskSpec>),
}

impl std::fmt::Debug for TaskSpec {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{self}")
  }
}

impl TaskSpec {
  /// Parse a task specification from arbitrary JSON.
  pub fn from_json<T: Into<Value> + Clone>(
    value: &T,
  ) -> Result<TaskSpec, TaskSpecParseError> {
    let value = value.clone();
    match value.into() {
      Value::String(s) => Ok(TaskSpec::Single(CowStr::from(s))),
      Value::Object(map) => {
        let command = map
          .get("command")
          .or_else(|| map.get("cmd"))
          .and_then(|v| v.as_str().map(|s| CowStr::from(s.to_string())));
        let description = map
          .get("description")
          .and_then(|v| v.as_str().map(|s| CowStr::from(s.to_string())));
        let deps_value = map.get("dependencies").or_else(|| map.get("depends"));
        let mut dependencies = Vec::new();
        if let Some(Value::Array(dep_array)) = deps_value {
          for dep in dep_array {
            if let Value::String(name) = dep {
              dependencies.push(CowStr::from(name.clone()));
            } else {
              return Err(TaskSpecParseError::InvalidDependencyType);
            }
          }
        } else if let Some(Value::String(name)) = deps_value {
          dependencies.push(CowStr::from(name.clone()));
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
          seq.push(TaskSpec::from_json(&item)?);
        }
        Ok(TaskSpec::Sequence(seq))
      }
      other => Err(TaskSpecParseError::InvalidType(
        type_name_of_val(&other).to_string(),
      )),
    }
  }

  /// Convert the task specification back into a JSON value. This is used when
  /// writing updates back to the configuration file.
  pub fn to_json_value(&self) -> Value {
    match self {
      TaskSpec::Single(s) => Value::String(s.to_string()),
      TaskSpec::Detailed {
        command,
        description,
        dependencies,
      } => {
        let mut map = serde_json::Map::new();
        if let Some(cmd) = command {
          map.insert("command".into(), Value::String(cmd.to_string()));
        }
        if !dependencies.is_empty() {
          let deps = dependencies
            .iter()
            .map(|dep| Value::String(dep.to_string()))
            .collect();
          map.insert("dependencies".into(), Value::Array(deps));
        }
        if let Some(desc) = description {
          map.insert("description".into(), Value::String(desc.to_string()));
        }
        Value::Object(map)
      }
      TaskSpec::Sequence(list) => {
        let seq = list.iter().map(TaskSpec::to_json_value).collect();
        Value::Array(seq)
      }
    }
  }

  pub fn to_json(&self) -> String {
    serde_json::to_string(&self.to_json_value()).unwrap_or_default()
  }

  pub fn to_json_pretty(&self) -> String {
    serde_json::to_string_pretty(&self.to_json_value()).unwrap_or_default()
  }

  /// Return a command-like string for display in task listings.
  pub fn command_string(&self) -> String {
    match self {
      TaskSpec::Single(s) => s.to_string(),
      TaskSpec::Detailed {
        command,
        dependencies,
        ..
      } => {
        let mut cmd = String::new();
        if !dependencies.is_empty() {
          cmd.push_str(
            &dependencies
              .iter()
              .map(|dep| dep.as_ref())
              .collect::<Vec<_>>()
              .join(" && "),
          );
        }
        if let Some(run) = command {
          if !cmd.is_empty() {
            cmd.push_str(" && ");
          }
          cmd.push_str(run.as_ref());
        }
        cmd
      }
      TaskSpec::Sequence(list) => list
        .iter()
        .map(|task| task.command_string())
        .collect::<Vec<_>>()
        .join(" && "),
    }
  }
}

/// Errors that may occur while parsing a task specification from JSON.
#[derive(Error, Debug, Clone, PartialEq, Eq, IsVariant)]
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

impl From<&TaskSpec> for Value {
  #[inline(always)]
  fn from(spec: &TaskSpec) -> Self {
    spec.to_json_value()
  }
}

impl From<TaskSpec> for Value {
  #[inline(always)]
  fn from(spec: TaskSpec) -> Self {
    spec.to_json_value()
  }
}

impl From<&Value> for TaskSpec {
  #[inline(always)]
  fn from(value: &Value) -> Self {
    TaskSpec::from_json(value).expect("invalid task spec")
  }
}

impl From<Value> for TaskSpec {
  #[inline(always)]
  fn from(value: Value) -> Self {
    TaskSpec::from_json(&value).expect("invalid task spec")
  }
}

impl FromStr for TaskSpec {
  type Err = RunnerError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let value: Value = serde_json::from_str(s).map_err(|_| {
      TaskSpecParseError::InvalidType(
        "could not parse string as JSON".to_string(),
      )
    })?;

    TaskSpec::from_json(&value).map_err(Into::into)
  }
}

impl TryFrom<&str> for TaskSpec {
  type Error = RunnerError;

  fn try_from(s: &str) -> Result<Self, Self::Error> {
    s.parse()
  }
}
