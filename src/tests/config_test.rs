use crate::config::ConfigError;
use crate::config::ConfigSource;
use crate::config::HookConfig;
use crate::task::TaskSpec;
use crate::task::TaskSpecParseError;
use moos::CowStr;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

#[test]
fn parse_task_spec_string() {
  let v = json!("npm test");
  let spec = TaskSpec::from_json(&v).unwrap();
  assert_eq!(spec, TaskSpec::Single(CowStr::from("npm test")));
}

#[test]
fn parse_task_spec_object_with_command() {
  let v = json!({"command": "deno fmt", "description": "Format code"});
  let spec = TaskSpec::from_json(&v).unwrap();
  match spec {
    TaskSpec::Detailed {
      command,
      description,
      dependencies,
    } => {
      assert_eq!(command, Some(CowStr::from("deno fmt")));
      assert_eq!(description, Some(CowStr::from("Format code")));
      assert!(dependencies.is_empty());
    }
    _ => panic!("unexpected variant"),
  }
}

#[test]
fn parse_task_spec_object_without_command_or_dependencies_fails() {
  let v = json!({"description": "No command"});
  let err = TaskSpec::from_json(&v).unwrap_err();
  assert_eq!(err, TaskSpecParseError::MissingCommandAndDeps);
}

#[test]
fn parse_task_spec_array() {
  let v = json!(["build", {"command": "npm test"}]);
  let spec = TaskSpec::from_json(&v).unwrap();
  match spec {
    TaskSpec::Sequence(seq) => {
      assert_eq!(seq.len(), 2);
    }
    _ => panic!("expected sequence"),
  }
}

#[test]
fn discover_deno_json() {
  let dir = tempdir().unwrap();
  let deno_path = dir.path().join("deno.json");
  fs::write(
    &deno_path,
    r#"{
        "hooks": {"pre-commit": "fmt"},
        "tasks": {"fmt": "deno fmt"}
    }"#,
  )
  .unwrap();
  let cfg = HookConfig::discover(dir.path()).unwrap();
  match cfg.source {
    ConfigSource::DenoJson(ref path) => {
      assert_eq!(path.as_path(), deno_path.as_path())
    }
    _ => panic!("expected DenoJson"),
  }
  assert!(cfg.hooks.contains_key("pre-commit"));
  assert!(cfg.deno_tasks.contains_key("fmt"));
}

#[test]
fn discover_deno_json_task_objects() {
  let dir = tempdir().unwrap();
  let deno_path = dir.path().join("deno.json");
  fs::write(
    &deno_path,
    r#"{
        "hooks": {"pre-commit": "fmt"},
        "tasks": {
          "check": "deno check **/*.ts",
          "fmt": {"command": "deno fmt", "description": "Format", "dependencies": ["check"]}
        }
    }"#,
  )
  .unwrap();
  let cfg = HookConfig::discover(dir.path()).unwrap();
  match cfg.deno_tasks.get("fmt") {
    Some(TaskSpec::Detailed {
      command,
      description,
      dependencies,
    }) => {
      assert_eq!(*command, Some(CowStr::from("deno fmt")));
      assert_eq!(*description, Some(CowStr::from("Format")));
      assert_eq!(dependencies, &vec![CowStr::from("check")]);
    }
    _ => panic!("expected detailed task spec"),
  }
}

#[test]
fn deno_task_dependencies_must_exist() {
  let dir = tempdir().unwrap();
  let deno_path = dir.path().join("deno.json");
  fs::write(
    &deno_path,
    r#"{
        "hooks": {"pre-commit": "lint"},
        "tasks": {
          "lint": {"dependencies": ["missing"]}
        }
    }"#,
  )
  .unwrap();
  let err = HookConfig::discover(dir.path()).unwrap_err();
  match err {
    ConfigError::InvalidTask(name, _) => {
      assert_eq!(name, "lint");
    }
    _ => panic!("expected invalid task error"),
  }
}

#[test]
fn discover_package_json() {
  let dir = tempdir().unwrap();
  let pkg_path = dir.path().join("package.json");
  fs::write(
    &pkg_path,
    r#"{
        "hooks": {"pre-commit": "lint"},
        "scripts": {"lint": "eslint ."},
        "packageManager": "pnpm@9.1.4"
    }"#,
  )
  .unwrap();
  let cfg = HookConfig::discover(dir.path()).unwrap();
  match cfg.source {
    ConfigSource::PackageJson(ref path) => {
      assert_eq!(path.as_path(), pkg_path.as_path())
    }
    _ => panic!("expected PackageJson"),
  }
  assert_eq!(cfg.package_manager.as_deref(), Some("pnpm@9.1.4"));
  assert!(cfg.node_scripts.contains_key("lint"));
}

#[test]
fn discover_prefers_package_json_when_deno_has_no_hooks() {
  let dir = tempdir().unwrap();
  let deno_path = dir.path().join("deno.json");
  let pkg_path = dir.path().join("package.json");
  fs::write(
    &deno_path,
    r#"{
        "tasks": {"fmt": "deno fmt"}
    }"#,
  )
  .unwrap();
  fs::write(
    &pkg_path,
    r#"{
        "hooks": {"pre-commit": "lint"},
        "scripts": {"lint": "eslint ."}
    }"#,
  )
  .unwrap();
  let cfg = HookConfig::discover(dir.path()).unwrap();
  match cfg.source {
    ConfigSource::PackageJson(ref path) => {
      assert_eq!(path.as_path(), pkg_path.as_path())
    }
    _ => panic!("expected PackageJson"),
  }
}

#[test]
fn discover_prefers_deno_json_when_hooks_present() {
  let dir = tempdir().unwrap();
  let deno_path = dir.path().join("deno.json");
  let pkg_path = dir.path().join("package.json");
  fs::write(
    &deno_path,
    r#"{
        "hooks": {"pre-commit": "fmt"},
        "tasks": {"fmt": "deno fmt"}
    }"#,
  )
  .unwrap();
  fs::write(
    &pkg_path,
    r#"{
        "hooks": {"pre-commit": "lint"},
        "scripts": {"lint": "eslint ."}
    }"#,
  )
  .unwrap();
  let cfg = HookConfig::discover(dir.path()).unwrap();
  match cfg.source {
    ConfigSource::DenoJson(ref path) => {
      assert_eq!(path.as_path(), deno_path.as_path())
    }
    _ => panic!("expected DenoJson"),
  }
}

#[test]
fn deno_task_dependencies_must_be_array() {
  let dir = tempdir().unwrap();
  let deno_path = dir.path().join("deno.json");
  fs::write(
    &deno_path,
    r#"{
        "hooks": {"pre-commit": "lint"},
        "tasks": {
          "lint": {"dependencies": "fmt"},
          "fmt": "deno fmt"
        }
    }"#,
  )
  .unwrap();
  let err = HookConfig::discover(dir.path()).unwrap_err();
  match err {
    ConfigError::InvalidTask(name, message) => {
      assert_eq!(name, "lint");
      assert!(message.contains("dependencies must be an array of strings"));
    }
    _ => panic!("expected invalid task error"),
  }
}

use crate::config::strip_json_comments;

#[test]
fn preserves_urls_and_strings_with_slashes() {
  let input = r#"
    {
      "author": { "url": "https://berlette.com/path" },
      "note": "keep // inside string",
      // line comment
      "value": "/* not a comment */"
    }
    "#;

  let cleaned = strip_json_comments(input);
  let parsed: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
  assert_eq!(
    parsed,
    json!({
      "author": { "url": "https://berlette.com/path" },
      "note": "keep // inside string",
      "value": "/* not a comment */"
    })
  );
}

#[test]
fn removes_comments_and_preserves_newlines() {
  let input = "{\n// comment 1\n\"a\": 1,\n/* multi\nline */\n\"b\": 2\n}\n";
  let cleaned = strip_json_comments(input);
  assert_eq!(cleaned.lines().count(), input.lines().count());
  let parsed: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
  assert_eq!(parsed, json!({ "a": 1, "b": 2 }));
}
