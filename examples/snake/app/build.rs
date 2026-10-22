//! Compile host metadata into one local Snake Federate.

use boomerang_builder::compiler::*;
use boomerang_codegen::{
    cargo_build::CargoBuild, Compilation, DeploymentConfiguration, PayloadExports,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo = CargoBuild::from_env()?;
    cargo.track_file("build.rs")?;
    let federate = FederateId::new("host")?;
    let configuration = DeploymentConfiguration {
        name: "snake".into(),
        placements: ["placement/keyboard", "placement/snake"]
            .into_iter()
            .map(|group| {
                Ok(PlacementAssignment::new(
                    PlacementGroupId::new(group)?,
                    federate.clone(),
                ))
            })
            .collect::<Result<_, TopologyAuthoringError>>()?,
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
        snake::topology()?,
        configuration,
        federate,
        cargo.target().clone(),
    );
    for (component, descriptor, payload) in [
        (
            "snake",
            snake_game::game::__boomerang::metadata::descriptor(),
            "snake_game::game::__boomerang::payload",
        ),
        (
            "keyboard",
            snake_keyboard::keyboard::__boomerang::metadata::descriptor(),
            "snake_keyboard::keyboard::__boomerang::payload",
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
    cargo.compile(&compilation, &mut std::io::stdout())?;
    Ok(())
}
