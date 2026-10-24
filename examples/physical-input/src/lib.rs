//! One timestamped sample supplied by a component-owned driver worker.
//!
//! Run `cargo run -p physical-input` from the repository root.
//! Cargo compiles `build.rs` for the host and the application for the selected target.
//!
//! # Metadata and code generation: `build.rs`
//!
//! The build script owns topology, deployment configuration, component registration, and code generation.
//! It selects `boomerang::component_metadata` for the shared declaration in `src/components.rs`.
//! This facet emits descriptors and typed topology constructors.
//! It omits reaction bodies, implementation imports, and driver helpers.
//! The build script tracks itself and the shared declaration as build inputs.
//!
//! The compiler uses Cargo's target context to generate image tables, bindings, and compatibility checks.
//! The Cargo adapter publishes generated source beneath `OUT_DIR`.
//! `boomerang_builder` and `boomerang_codegen` are build dependencies.
//! Target dependencies disable hosted reactor construction to exclude the builder.
//!
//! # Shared declaration and runtime implementation: `src/components.rs`
//!
//! Both stages load this file through normal Rust modules.
//! Its reactor signature and attributes define the common metadata for both facets.
//! `src/lib.rs` selects `boomerang::component_payload` to emit the target implementation.
//! This facet includes reaction bodies and the adjacent driver state and worker code.
//! The library reexports [`sensor`] for the generated payload bindings.
//! The macro excludes runtime-only items from metadata emission without conditional compilation in the example.
//!
//! # Runtime configuration and execution: `src/main.rs`
//!
//! Main includes the generated source from `OUT_DIR`.
//! It constructs the input configuration and the manual clock in domain seven.
//! It installs physical inputs and the clock before runtime execution.
//! Runtime configuration keeps fast-forward disabled.
//!
//! The build script assigns source key zero to the sensor and declares manual clock control.
//! Main places the matching required `plant` source at key zero.
//! That source owns the compiled image's only physical action.
//! Both input capacity limits are one.
//! This explicit mapping depends on this example's single-source topology.
//!
//! Required source progress limits execution, even after the manual clock advances.
//!
//! The initializer receives a typed `DriverInit` capability bundle before tagged work starts.
//! Its `sample` field is a `SourceAction<u32>` for timestamp-preserving admission.
//! The finite worker advances the clock to 10 ms.
//! It submits value 42 at that acquisition time.
//! It reports an exclusive acquisition frontier of 10 ms plus 1 ns.
//! The action's 1 ms minimum delay maps the observation to logical time 11 ms.
//!
//! The worker advances the clock to 11 ms to permit physical pacing to reach that tag.
//! The reaction prints the sample.
//! It requests shutdown.
//!
//! Component state owns the finite worker.
//! It joins the worker during teardown.
//! Closed capabilities represent normal cancellation; other worker errors appear on standard error.
//! A continuous driver requires a cancellation-aware guard.
//! If initialization fails before state returns, the initializer must cancel its workers.
//! It must join those workers.
//!
//! The runtime closes capabilities before it drops successfully returned states.
//!
//! Ordinary callbacks use `AsyncActionRef<T>` with `SendContext` on a separate action.
//! That path can adjust late timestamps; source admission rejects late observations.
//! Host monotonic time also supports source admission without manual clock control.
//! Such applications omit `clock_control` and read physical time through `source.clock()`.

use boomerang::component_payload as component;

mod components;

pub use components::sensor;
