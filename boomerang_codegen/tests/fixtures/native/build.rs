use boomerang::component_metadata as emit;
use boomerang_builder::compiler::*;
use boomerang_codegen::{cargo_build::CargoBuild, *};
include!("declaration.rs");
#[cfg(feature = "source-input")]
declare_controller!(emit, source = "plant",);
#[cfg(not(feature = "source-input"))]
declare_controller!(emit,);
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cargo = CargoBuild::from_env()?;
    cargo.track_file("build.rs")?;
    cargo.track_file("declaration.rs")?;
    for name in [
        "CARGO_FEATURE_MISSING_EXPORT",
        "CARGO_FEATURE_FINGERPRINT_MISMATCH",
        "CARGO_FEATURE_ABI_MISMATCH",
        "CARGO_FEATURE_SOURCE_INPUT",
    ] {
        cargo.track_env(name)?;
    }
    let mut topology = TopologyBuilder::new("native").unwrap();
    let enclave = topology.enclave("first")?;
    let first = topology.component("first", controller::definition(), &enclave)?;
    let second_enclave = topology.enclave("second")?;
    let second = topology.component("second", controller::definition(), &second_enclave)?;
    topology.connect(&first.output, &second.input)?;
    let federate = FederateId::new("local")?;
    let configuration = DeploymentConfiguration {
        name: "native".into(),
        placements: ["first", "second"]
            .into_iter()
            .map(|name| {
                Ok(PlacementAssignment::new(
                    PlacementGroupId::new(format!("placement/{name}"))?,
                    federate.clone(),
                ))
            })
            .collect::<Result<_, TopologyAuthoringError>>()?,
        federates: vec![FederateConfig::new(
            federate.clone(),
            cargo.target().triple().clone(),
            RuntimeBackendId::new("reference")?,
            boomerang::runtime::image::RecoveryPolicy::FailStop,
        )],
        coordination: CoordinationSelection::Local,
        boundaries: vec![],
    };
    let mut compilation = Compilation::new(
        topology.finish()?,
        configuration,
        federate,
        cargo.target().clone(),
    );
    let implementation = ImplementationId::new("controller")?;
    let path = if std::env::var_os("CARGO_FEATURE_MISSING_EXPORT").is_some() {
        "crate::absent::__boomerang::payload"
    } else if std::env::var_os("CARGO_FEATURE_FINGERPRINT_MISMATCH").is_some()
        || std::env::var_os("CARGO_FEATURE_ABI_MISMATCH").is_some()
    {
        "crate::incorrect_exports"
    } else {
        "crate::controller::__boomerang::payload"
    };
    compilation.register(
        implementation.clone(),
        controller::__boomerang::metadata::descriptor(),
        PayloadExports::new(path)?,
    )?;
    for component in ["first", "second"] {
        compilation.select(ComponentInstanceId::new(component)?, implementation.clone())?;
    }
    #[cfg(feature = "source-input")]
    compilation.binding_configuration(boomerang_codegen::render::BindingConfiguration {
        sources: ["first", "second"]
            .into_iter()
            .enumerate()
            .map(|(key, name)| Ok((ComponentInstanceId::new(name)?, key as u32)))
            .collect::<Result<_, TopologyAuthoringError>>()?,
        manual_clock: false,
        distributed: false,
    });
    cargo.compile(&compilation, &mut std::io::stdout())?;
    Ok(())
}
