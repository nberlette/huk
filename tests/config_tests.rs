use huk::config::ConfigSource;
use huk::config::HookConfig;
use huk::task::TaskSpec;
use huk::task::TaskSpecParseError;
use serde_json::json;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn parse_task_spec_string() {
  let v = json!("npm test");
  let spec = TaskSpec::from_json(&v).unwrap();
  assert_eq!(spec, TaskSpec::Single("npm test".into()));
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
      assert_eq!(command, Some("deno fmt".into()));
      assert_eq!(description, Some("Format code".into()));
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
