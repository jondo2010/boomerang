# Boomerang-Tinymap

[![crates.io](https://img.shields.io/crates/v/boomerang_tinymap.svg)](https://crates.io/crates/boomerang_tinymap)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](./LICENSE)
[![Downloads](https://img.shields.io/crates/d/boomerang_tinymap.svg)](https://crates.io/crates/boomerang_tinymap)
[![CI](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml)
[![docs](https://docs.rs/boomerang_tinymap/badge.svg)](https://docs.rs/boomerang_tinymap)
[![codecov](https://codecov.io/github/jondo2010/boomerang/graph/badge.svg?token=PYXF8VSNY9)](https://codecov.io/github/jondo2010/boomerang)

A tiny, fast, and simple Slotkey-type map implementation for [`boomerang`](https://docs.rs/boomerang).

[`TinyMap`], [`TinySecondaryMap`] and [`KeySet`] are built as a write-once, read-many data structures. Methods to remove elements are intentionally omitted.
