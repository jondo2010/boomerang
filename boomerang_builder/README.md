# Boomerang Builder

[![crates.io](https://img.shields.io/crates/v/boomerang_builder.svg)](https://crates.io/crates/boomerang_builder)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](./LICENSE)
[![Downloads](https://img.shields.io/crates/d/boomerang_builder.svg)](https://crates.io/crates/boomerang_builder)
[![CI](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml)
[![docs](https://docs.rs/boomerang_builder/badge.svg)](https://docs.rs/boomerang_builder)
[![codecov](https://codecov.io/github/jondo2010/boomerang/graph/badge.svg?token=PYXF8VSNY9)](https://codecov.io/github/jondo2010/boomerang)

The Reactor assembly API for Boomerang. Builder is the API driven by the code that [`boomerang_derive`](https://docs.rs/boomerang_derive) generates.

The most important data structure for Builder is the [`EnvBuilder`], which also serves as the API entry-point. Once all
of the Reactors and Reactions have added their graph state into the [`EnvBuilder`], [`EnvBuilder::into_runtime_parts`]
is called to generate the data for the Runtime.

Most users will not need to interact with Builder, but for some specialized cases it is useful to manually implement the
[`reactor::Reactor`] and [`reaction::Reaction`] traits manually. It may also ocasionally useful to manually adjust the
[`EnvBuilder`] graph after all the Reactors have been built.