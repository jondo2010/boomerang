//! Host-only metadata, topology, and code generation.

use boomerang::component_metadata as component;

// Expand only metadata from the shared declaration; omit its runtime implementation.
#[path = "src/components.rs"]
mod components;

use boomerang_builder::compiler::*;
use boomerang_codegen::{cargo_build::CargoBuild, render::BindingConfiguration, *};
use components::sensor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo = CargoBuild::from_env()?;
    for file in ["build.rs", "src/components.rs"] {
        cargo.track_file(file)?;
    }
    let federate = FederateId::new("host")?;
    let configuration = DeploymentConfiguration {
        name: "physical-input".into(),
        placements: vec![PlacementAssignment::new(
            PlacementGroupId::new("placement/sensor")?,
            federate.clone(),
        )],
        federates: vec![FederateConfig::new(
            federate.clone(),
            cargo.target().triple().clone(),
            RuntimeBackendId::new("std")?,
            boomerang::runtime::image::RecoveryPolicy::FailStop,
        )],
        coordination: CoordinationSelection::Local,
        boundaries: vec![],
    };
    let mut compilation =
        Compilation::new(topology()?, configuration, federate, cargo.target().clone());
    let implementation = register_component!(
        compilation,
        "sensor",
        metadata = sensor,
        payload = physical_input::sensor
    )?;
    let sensor = ComponentInstanceId::new("sensor")?;
    compilation.select(sensor.clone(), implementation)?;
    compilation.binding_configuration(BindingConfiguration {
        sources: [(sensor, 0)].into_iter().collect(),
        manual_clock: true,
        distributed: false,
    });
    cargo.compile(&compilation, &mut std::io::stdout())?;
    Ok(())
}

/// Place the sensor component in one Enclave.
fn topology() -> Result<ApplicationTopology, TopologyAuthoringError> {
    let mut app = TopologyBuilder::new("application/physical-input")?;
    let enclave = app.enclave("sensor")?;
    app.component("sensor", sensor::definition(), &enclave)?;
    app.finish()
}
