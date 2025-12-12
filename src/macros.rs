#[doc(hidden)]
#[macro_export]
macro_rules! file_name {
  ($path:expr) => {
    $path
      .file_name()
      .and_then(|_| $path.components().last().map(|c| c.as_os_str()))
      .map(|o| o.to_string_lossy())
      .unwrap_or_else(|| "<unknown>".into())
  };
}

#[doc(hidden)]
#[macro_export]
macro_rules! print_tasks {
  ($cfg:expr) => {
    let (kind, source, path) = match $cfg.source {
      $crate::config::ConfigSource::DenoJson(ref path) => {
        ("task", $crate::file_name!(path), path)
      }
      $crate::config::ConfigSource::PackageJson(ref path) => {
        ("script", $crate::file_name!(path), path)
      }
    };
    let mut all_tasks: Vec<&String> = Vec::new();

    let mut deno_tasks: Vec<&String> = $cfg.deno_tasks.keys().collect();
    deno_tasks.sort();
    all_tasks.extend(deno_tasks.clone());

    let mut node_scripts: Vec<&String> = $cfg.node_scripts.keys().collect();
    node_scripts.sort();
    all_tasks.extend(node_scripts.clone());

    let mut n = all_tasks.len();
    if n == 0 {
      eprintln!(
        "No {kind}s found in '{path}'.",
        path = path.display().to_string()
      );
    } else {
      let s = if n == 1 { "" } else { "s" };
      eprintln!(
        "Discovered {n} {kind}{s} in '{path}':",
        path = path.display().to_string()
      );

      while n > 0 {
        let name = &all_tasks[all_tasks.len() - n];
        let cmd = if let Some(script) = $cfg.node_scripts.get(*name) {
          script
        } else if let Some(script) = $cfg.deno_tasks.get(*name) {
          script
        } else {
          "<unknown>"
        };
        let named = (*name).clone();
        let name = format!(
          r#"{cyan}{named}{reset}"#,
          cyan = "\x1b[1;36m",
          reset = "\x1b[0m"
        );
        let cmd = cmd.replace('\n', " ");
        let src = format!(
          r#"{italic}{gray}({source}){reset}"#,
          italic = "\x1b[3m",
          gray = "\x1b[90m",
          reset = "\x1b[0m"
        );
        eprintln!(r#"- {name} {src}"#);
        eprintln!(r#"  {cmd}"#);

        n -= 1;
      }
    }
  };
}

#[doc(hidden)]
#[macro_export]
macro_rules! print_available_hooks {
  ($cfg:expr) => {
    use ::ratatui::prelude::Stylize;
    let mut hook_names: Vec<&String> = $cfg.hooks.keys().collect();
    hook_names.sort();
    let path = $cfg.source.as_path_buf().display().to_string();
    let base = $crate::file_name!($cfg.source.as_path_buf());
    let mut n = hook_names.len();
    if n == 0 {
      eprintln!("No hooks are defined in '{path}'.");
    } else {
      let s = if n == 1 { "" } else { "s" };
      eprintln!(
        r#"{green}Discovered {n} hook{s} in{reset} {blue}{path}{reset}:"#,
        green = "\x1b[1;32m",
        reset = "\x1b[0m",
        blue = "\x1b[1;34m"
      );
      eprintln!();
      while n > 0 {
        let hook_name = &hook_names[hook_names.len() - n];
        let source = format!(
          r#"{italic}{gray}({base}){reset}"#,
          italic = "\x1b[3m",
          gray = "\x1b[90m",
          reset = "\x1b[0m"
        );
        let name = (*hook_name).clone().bold().cyan().to_string();
        let spec = $cfg
          .hooks
          .get(*hook_name)
          .map(|s| s.to_string())
          .unwrap_or_else(|| "<unknown>".into());
        let info = spec.replace('\n', " ").to_string();
        eprintln!(
          r#"- {cyan}{name}{reset} {source}"#,
          cyan = "\x1b[1;36m",
          reset = "\x1b[0m"
        );
        eprintln!(r#"  {info}"#);
        n -= 1;
      }
      eprintln!();
    }
  };
}
