//! Cargo-native terminal keyboard and arrow display demo.
//!
//! Run `cargo run -p snake-keyboard-app` from the repository root in an interactive terminal.
//! Press an arrow key to display its arrow. Press Ctrl-C to stop the application.
//!
//! # Host compilation
//!
//! The build script declares the keyboard-to-display topology directly.
//! It registers the keyboard and display descriptors from `snake-keyboard` against their owning payload exports.
//! Both components share one local placement group in the `host` Federate.
//! The optional Cargo adapter publishes image definitions and bindings under `OUT_DIR`.
//! Cargo owns dependency resolution and compilation; the application requires no compiler wrapper or generated Cargo package.
//!
//! # Runtime
//!
//! The executable links the `snake-keyboard` payload and the compiled runtime.
//! Its target dependencies exclude the builder, code generator, and game payload.
//! The checked-in main executes the generated Federate with fast-forward disabled and keep-alive enabled.
//! The existing source-free keyboard action uses ordinary delivery without an added physical-input source policy.
//! The keyboard worker enables raw terminal mode and sends key events to the display.
//! Ctrl-C requests shutdown. The shutdown reaction stops the worker and joins its thread.
//! The terminal guard restores terminal mode during teardown.
//!
//! Interactive verification is manual. Check all four arrow keys and Ctrl-C shutdown in a terminal.
//! Confirm that normal terminal input resumes after shutdown.

/// Generated image tables and bindings for the keyboard and display payloads.
// The shared generator retains fallback fields across binding configurations.
#[allow(clippy::needless_update)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/boomerang.rs"));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    boomerang_runtime::execute_owned_federate_slice(
        generated::FEDERATE,
        &generated::FEDERATE_VIEW,
        generated::generated_bindings(),
        boomerang_runtime::Config::default()
            .with_fast_forward(false)
            .with_keep_alive(true),
    )?;
    Ok(())
}
