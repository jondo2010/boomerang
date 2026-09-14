# Boomerang Utilities

[![crates.io](https://img.shields.io/crates/v/boomerang_util.svg)](https://crates.io/crates/boomerang_util)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](./LICENSE)
[![Downloads](https://img.shields.io/crates/d/boomerang_util.svg)](https://crates.io/crates/boomerang_util)
[![CI](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml)
[![docs](https://docs.rs/boomerang_util/badge.svg)](https://docs.rs/boomerang_util)
[![codecov](https://codecov.io/github/jondo2010/boomerang/graph/badge.svg?token=PYXF8VSNY9)](https://codecov.io/github/jondo2010/boomerang)

This crate owns optional host-side support that does not belong in the core
runtime. The `launcher` feature is the canonical process-policy layer for
generated launchers. It provides tracing initialization and the private
execution-summary protocol used by `cargo-boomerang`.

The `runner` feature retains the older convenience API that builds, lowers, and
executes a Reactor in one process. New production applications should use
`cargo-boomerang` and generated launchers. The `test-tracing` feature remains a
compatibility helper for existing tests while delegating subscriber setup to
the launcher support implementation.
