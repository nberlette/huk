//! Constants used throughout the `huk` tool.

/// Git hook names as defined by [Git documentation].
///
/// [Git documentation]: https://git-scm.com/docs/githooks
pub const GIT_HOOKS: [&'static str; 22] = [
  "pre-applypatch",
  "pre-auto-gc",
  "pre-checkout",
  "pre-commit",
  "pre-merge-commit",
  "pre-push",
  "pre-rebase",
  "pre-receive",
  "prepare-commit-msg",
  "applypatch-msg",
  "commit-msg",
  "post-applypatch",
  "post-checkout",
  "post-commit",
  "post-merge",
  "post-receive",
  "post-rewrite",
  "post-update",
  "push-to-checkout",
  "fsmonitor-watchman",
  "sendemail-validate",
  "update",
];
