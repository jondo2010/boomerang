# Repository Guidelines

## Project Structure & Module Organization

This is a Rust workspace. Top-level crates live in `boomerang/`, `boomerang_builder/`, `boomerang_runtime/`, `boomerang_macros/`, `boomerang_util/`, and `boomerang_tinymap/`. Tests are primarily in `boomerang/tests/` and unit tests alongside code. Examples live in `examples/` (notably `examples/snake/`). Benchmarks are in `boomerang/benches/`. Agent process guidelines live in `.agent/`.

## Build, Test, and Development Commands

- `cargo build` — build the full workspace.
- `cargo test` — run all workspace tests and doc tests.
- `cargo test -p boomerang_runtime action::store` — run targeted action store tests.
- `cargo bench -p boomerang --bench ping_pong` — run the ping-pong benchmark.
- `BOOMERANG_PROFILE=1 cargo bench -p boomerang --bench ping_pong` — generate flamegraphs in `target/criterion/ping_pong/*/profile/`.

## Coding Style & Naming Conventions

Rust style follows `rustfmt` defaults (4-space indentation). Prefer clear, direct names (`ActionStore`, `Reactor`, `Reaction`). Tests use snake_case names and live either in `tests/` or `#[cfg(test)]` modules. Keep API doc comments concise and aligned with current behavior.

## Testing Guidelines

Core tests are Rust unit and integration tests using `cargo test`. New behavior should be covered by unit tests near the implementation or integration tests in `boomerang/tests/`. Benchmark regressions are checked with `cargo bench -p boomerang --bench ping_pong`.

## Commit & Pull Request Guidelines

Commit messages typically use a short prefix: `feat:`, `fix:`, `refactor:`, `chore:`, `test:`, or `merge:`. Keep subjects imperative and scoped. All changes should go through a PR; avoid direct pushes to `main`. PRs should include a concise summary, test results (commands + outcome), and any benchmark notes when performance is impacted.

## Agent-Specific Instructions

When writing complex features or significant refactors, use an ExecPlan (see `.agent/PLANS.md`) from design through implementation. Keep the plan updated as work proceeds.
