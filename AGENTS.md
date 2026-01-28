# Repository Guidelines

## Project Structure & Module Organization
- `src/` contains the Rust CLI/TUI implementation; entry points live in `src/main.rs` and shared logic in `src/lib.rs`.
- `src/tests.rs` and `src/tests/` hold unit tests (currently `src/tests/config_test.rs`).
- `main.ts` is the Deno entry point for the published package.
- `deno.json` and `.huk.json` define tasks and hook configuration; `schema.json` documents the config schema.
- Build outputs land in `bin/` (custom artifacts) and `target/` (Cargo defaults).

## Build, Test, and Development Commands
- `deno task build` builds the release binary into `bin/` (uses Cargo under the hood).
- `deno task build:debug` builds a debug binary into `bin/`.
- `deno task test` runs `cargo test --all`; use `deno task test:verbose` for full logs.
- `deno task fmt` / `deno task fmt:check` format or verify formatting.
- `deno task lint` runs `cargo clippy --all --all-targets -- -D warnings`.
- `cargo run --bin huk -- <subcommand>` runs the CLI locally (e.g., `cargo run --bin huk -- dashboard`).

## Coding Style & Naming Conventions
- Rust edition is 2024; formatting is enforced by `.rustfmt.toml` (80 column max, 2-space tabs, item-level imports).
- Use `cargo fmt` before commits and keep Clippy clean (`deno task lint`).
- Follow Rust conventions: `snake_case` for modules/functions/tests (e.g., `parse_task_spec_string`), `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants.

## Testing Guidelines
- Use `cargo test --all` or `deno task test`; tests live in `src/tests.rs` and `src/tests/*.rs`.
- Add targeted tests for config parsing, hook resolution, and task execution paths.

## Commit & Pull Request Guidelines
- Commit messages follow Conventional Commits (`feat(tui): add tasks view`, `docs: update README`), with optional `[WIP]` suffix when needed.
- PRs should include a concise summary, tests run, and note any config schema or hook changes.
- Include screenshots or short clips for TUI-facing changes.

## Configuration & Hook Definitions
- Define hooks in `deno.json` or `.huk.json` under the `hooks` field; tasks live under `tasks`.
- When changing config formats or validation, update `schema.json` and add/adjust tests.
