//! Cargo-native terminal Snake application.
//!
//! Run `cargo run -p snake-app` from the repository root in an interactive terminal.
//! Use the arrow keys to turn. Use Ctrl-C to stop the game.
//!
//! # Host compilation and runtime dependencies
//!
//! The `snake` crate supplies the shared host topology through its `metadata` feature.
//! The build script selects metadata from the owning `snake-game` and `snake-keyboard` crates.
//! Those additive component features enable the hosted builder API without the game implementation or terminal worker.
//! The game enables its optional `rand` dependency only through `hosted` or `payload`.
//! Crossterm supplies portable key-event signature types in every component lane.
//!
//! `build.rs` uses `CargoBuild` to compile that metadata and publish Rust definitions in Cargo's `OUT_DIR`.
//! It registers each descriptor against its owning crate's public payload exports.
//! Cargo compiles the payload dependencies for the application target.
//!
//! The target uses `boomerang_runtime` and typed bindings. It does not link the topology, builder, or code generator.
//! Cargo owns compilation and dependency resolution. The application requires no compiler wrapper or generated Cargo package.
//!
//! # Topology and execution policy
//!
//! One local `host` Federate contains separate `keyboard` and `snake` placement groups.
//! The keyboard sends typed key events to the game.
//! Its `ready` output starts rendering and game pacing after raw terminal initialization.
//!
//! The executable uses `execute_owned_federate_slice` with fast-forward disabled and keep-alive enabled.
//! The existing source-free keyboard action uses ordinary delivery without an added physical-input source policy.
//! Explicit logical actions preserve the game clock and food timer.
//!
//! Ctrl-C requests shutdown. The shutdown reaction stops the polling worker.
//! It joins the worker thread. It restores terminal mode.
//!
//! The component crates retain their default `hosted` features for legacy CLI consumers.
//! The retained `Boomerang.toml` manifests and telemetry demo still use the legacy generated-launcher path.
//! Their bindings select `hosted` explicitly because legacy descriptor drivers disable dependency defaults.
//! This application does not enable that telemetry exporter.
//!
//! # Limits and regression coverage
//!
//! Food placement uses a runtime random source. This terminal example does not provide deterministic replay.
//! The executable regression requires Unix pseudo-terminals and Python 3.
//! It checks game readiness, an arrow-key turn, finite Ctrl-C shutdown, score output, and terminal-attribute restoration.
//! Non-Unix hosts do not run the pseudo-terminal regression.
