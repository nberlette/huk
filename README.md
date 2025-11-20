# hük

hük (pronounced *huk*) is a small CLI/TUI utility written in Rust that makes it
easy to manage Git hooks for projects using either Deno or Node.js. By placing a
`hooks` field in your `deno.json`/`deno.jsonc` or `package.json` file you can
declare which Git hook names should trigger which tasks. hük will install
light‑weight wrapper scripts into your repository’s hooks directory and run
those tasks automatically when the associated Git hook is fired.

## Motivation

Git hooks are scripts that run at particular points in the Git workflow such as
just before committing (`pre‑commit`), when preparing a commit message
(`prepare‑commit‑msg`) or before pushing (`pre‑push`). Git’s documentation
describes a rich set of client‑side hooks, including `pre‑commit`,
`prepare‑commit‑msg`, `commit‑msg`, `post‑commit`, `pre‑rebase` and
`pre‑push`【23307213681274†L240-L330】. Setting up and distributing these scripts
across multiple environments can be cumbersome. hük centralizes hook
definitions alongside your project’s existing task configuration, making it
simple to install and manage them.

If your project targets Node.js you can also specify a `packageManager` field in
`package.json` to pin the exact package manager binary. Supported values include
`npm@x.y.z`, `pnpm@x.y.z` and `yarn@x.y.z`. The [Corepack](https://nodejs.org/docs/latest/api/cli.html#corepack) tool uses this field to download and select the
appropriate package manager; hük respects it and falls back to `npm` when
unspecified【349948098167533†L48-L59】.

## Installation

Compile hük with Cargo and place the resulting binary somewhere in your `$PATH`.
Once built, navigate to your project directory and run:

```shell
huk install
```

This command reads your `deno.json`/`deno.jsonc` or `package.json`, parses the
`hooks` field and writes an executable script for each hook into the Git hooks
directory. The installer honours the `core.hooksPath` Git configuration; if
`core.hooksPath` is unset hük writes scripts into `.git/hooks`. Existing
scripts are left untouched unless you pass `--force`.

## Defining Hooks

Add a top‑level `hooks` object to your configuration file. Each property name
must be one of the valid Git hook names. The value can take one of three
forms:

1. **String** – treated as either the name of a script/task or a raw shell command.
2. **Object** – may contain `command`, `description` and `dependencies` keys.
   At least a `command` or one or more `dependencies` must be provided. If
   `dependencies` is present it should be an array of strings naming other tasks.
3. **Array** – a sequence of strings or objects, executed in order.

Tasks can refer to:

- **Deno tasks** defined in the `tasks` field of `deno.json`.
- **Node scripts** defined in the `scripts` field of `package.json`.
- **Other hook definitions** by name.
- **Raw shell commands** passed directly to the shell.

If both a `deno.json` and a `package.json` are present, hük prefers the
`deno.json` and falls back to `package.json`. When executing Node scripts
hük honours the `packageManager` field if present【349948098167533†L48-L59】.

### Example (Deno)

```jsonc
{
  // deno.json
  "tasks": {
    "fmt": "deno fmt",
    "lint": "deno lint"
  },
  "hooks": {
    "pre-commit": [
      "fmt",
      { "command": "deno test", "description": "Run unit tests" },
      { "dependencies": ["lint"], "description": "Ensure code is linted" }
    ],
    "pre-push": "deno task test"
  }
}
```

### Example (Node)

```json
{
  // package.json
  "scripts": {
    "lint": "eslint .",
    "test": "npm run lint && vitest"
  },
  "packageManager": "pnpm@9.1.4",
  "hooks": {
    "pre-commit": [
      "lint",
      { "command": "npm run test", "description": "Run tests" }
    ],
    "commit-msg": { "command": "echo Validate commit message" }
  }
}
```

## CLI Usage

```text
USAGE:
    huk <SUBCOMMAND>

SUBCOMMANDS:
    install    Install wrapper scripts into the Git hooks directory
    list       List configured Git hooks
    run        Execute the tasks associated with a specific hook
    tasks      List available tasks or run a named task
    dashboard  Launch a TUI for inspecting and running hooks
    add        Add a hook definition (not yet implemented)
    remove     Remove a hook definition (not yet implemented)
    update     Update a hook definition (not yet implemented)
```

The `dashboard` subcommand starts a simple interactive UI. Use the arrow keys
to navigate through hooks, press Enter to run a hook, and press `q` to exit.

## License

This project is dual‑licensed under the MIT License and the Apache License
2.0.
