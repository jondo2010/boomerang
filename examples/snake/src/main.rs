//! Cargo-native terminal Snake application.
//!
//! Run `cargo run -p snake-app` from the repository root in an interactive terminal.
//! Use the arrow keys to turn. Use Ctrl-C to stop the game.
//!
//! # Host compilation and runtime dependencies
//!
//! `build.rs` declares the application topology directly.
//! The build script selects metadata from the owning `snake-game` and `snake-keyboard` crates.
//! Those additive component features enable the builder API without the game implementation or terminal worker.
//! The game enables its optional `rand` dependency only through `dynamic` or `payload`.
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
//! The executable uses `execute_owned_federate_slice_with_observations` with fast-forward disabled and keep-alive enabled.
//! The existing source-free keyboard action uses ordinary delivery without an added physical-input source policy.
//! Explicit logical actions preserve the game clock and food timer.
//!
//! Ctrl-C requests shutdown. The shutdown reaction stops the polling worker.
//! It joins the worker thread. It restores terminal mode.
//!
//! # Optional telemetry
//!
//! Enable the `telemetry` feature to publish scheduler observations through the hosted UDP exporter.
//! Set `BOOMERANG_TELEMETRY_ENDPOINT` to the monitor socket address to enable publication.
//! Supply a fresh `BOOMERANG_TELEMETRY_RUN_ID` with exactly 32 hexadecimal digits for each session.
//! `BOOMERANG_TELEMETRY_PERIOD_MS` sets a positive sampling interval. The default is 100 milliseconds.
//! Without an endpoint, the application starts no exporter and attaches no observation handles.
//!
//! ```sh
//! BOOMERANG_TELEMETRY_ENDPOINT=127.0.0.1:9000 \
//! BOOMERANG_TELEMETRY_RUN_ID=0123456789abcdef0123456789abcdef \
//! cargo run -p snake-app --features telemetry
//! ```
//!
//! Replace the example run ID for each session. The demo script creates a fresh ID automatically.
//! Each Enclave supplies one source in the `host` Federate and process.
//! The single process uses incarnation zero within its unique run.
//! The artifact ID hashes the embedded generated Rust definitions for telemetry grouping.
//! This ID provides no payload-compatibility or coordination guarantee.
//! UDP publication is best effort. The exporter repeats snapshots without a receiver-readiness handshake.
//! The exporter guard remains alive through execution. Its destructor publishes final snapshots and joins its worker.
//!
//! # Limits and regression coverage
//!
//! Food placement uses a runtime random source. This terminal example does not provide deterministic replay.
//! The executable regression requires Unix pseudo-terminals and Python 3.
//! It checks game readiness, an arrow-key turn, finite Ctrl-C shutdown, score output, and terminal-attribute restoration.
//! With `telemetry`, the regression also checks both UDP sources, scheduler activity, and final stopped snapshots.
//! Non-Unix hosts do not run the pseudo-terminal regression.

/// Generated image tables and typed component bindings from host metadata.
// The shared generator retains fallback fields across binding configurations.
#[allow(clippy::needless_update)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/boomerang.rs"));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "telemetry")]
    let exporter = boomerang_util::telemetry::TelemetryExporter::from_environment(
        *blake3::hash(include_bytes!(concat!(env!("OUT_DIR"), "/boomerang.rs"))).as_bytes(),
        generated::FEDERATE_VIEW.id().as_str(),
        0,
        generated::FEDERATE_VIEW.enclave_views().map(|enclave| {
            boomerang_telemetry::SourceIdentity {
                role: boomerang_telemetry::SourceRole::Federate,
                federate_id: Some(generated::FEDERATE_VIEW.id().as_str()),
                enclave_id: Some(enclave.enclave_id().as_str()),
            }
        }),
    )?;
    #[cfg(feature = "telemetry")]
    let observations = exporter.as_ref().map_or_else(Vec::new, |exporter| {
        let span = generated::FEDERATE_VIEW.enclaves();
        // The validated span uses deployment-wide Enclave keys, including its start offset.
        (span.start()..span.checked_end().expect("validated Enclave span"))
            .map(boomerang_runtime::image::EnclaveIndex::from)
            .zip(exporter.observation_handles())
            .collect::<Vec<_>>()
    });
    #[cfg(not(feature = "telemetry"))]
    let observations = [];

    boomerang_runtime::execute_owned_federate_slice_with_observations(
        generated::FEDERATE,
        &generated::FEDERATE_VIEW,
        generated::generated_bindings(),
        boomerang_runtime::Config::default()
            .with_fast_forward(false)
            .with_keep_alive(true),
        &observations,
    )?;
    Ok(())
}
