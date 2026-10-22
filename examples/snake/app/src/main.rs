//! Checked-in execution policy for the Cargo-native Snake application.

/// Generated image tables and typed component bindings from host metadata.
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
