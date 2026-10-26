//! Compile keyboard and display metadata into one local Federate.

use boomerang_builder::compiler::*;
use snake_keyboard::{display, keyboard};

use boomerang_codegen::{
    cargo_build::CargoBuild, Compilation, DeploymentConfiguration, PayloadExports,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo = CargoBuild::from_env()?;
    cargo.track_file("build.rs")?;
    let federate = FederateId::new("host")?;
    let configuration = DeploymentConfiguration {
        name: "keyboard".into(),
        placements: vec![PlacementAssignment::new(
            PlacementGroupId::new("placement/keyboard")?,
            federate.clone(),
        )],
        federates: vec![FederateConfig::new(
            federate.clone(),
            cargo.target().triple().clone(),
            RuntimeBackendId::new("std")?,
            boomerang_runtime::image::RecoveryPolicy::FailStop,
        )],
        coordination: CoordinationSelection::Local,
        boundaries: vec![],
    };
    let mut compilation = Compilation::new(
        keyboard_topology()?,
        configuration,
        federate,
        cargo.target().clone(),
    );
    for (component, descriptor, payload) in [
        (
            "keyboard",
            snake_keyboard::keyboard::__boomerang::metadata::descriptor(),
            "snake_keyboard::keyboard::__boomerang::payload",
        ),
        (
            "display",
            snake_keyboard::display::__boomerang::metadata::descriptor(),
            "snake_keyboard::display::__boomerang::payload",
        ),
    ] {
        let implementation = ImplementationId::new(component)?;
        compilation.register(
            implementation.clone(),
            descriptor,
            PayloadExports::new(payload)?,
        )?;
        compilation.select(ComponentInstanceId::new(component)?, implementation)?;
    }
    cargo.compile(compilation, &mut std::io::stdout())?;
    Ok(())
}

/// Compose the application topology from component metadata.
fn keyboard_topology() -> Result<ApplicationTopology, TopologyAuthoringError> {
    let mut app = TopologyBuilder::new("application/display/keyboard")?;
    let enclave = app.enclave("keyboard")?;
    let keyboard = app.component("keyboard", keyboard::definition(), &enclave)?;
    let display = app.component("display", display::definition(), &enclave)?;
    app.connect(&keyboard.key, &display.key)?;
    app.finish()
}
