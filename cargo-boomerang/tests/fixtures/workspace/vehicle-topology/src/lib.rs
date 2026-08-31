use boomerang_builder::compiler::{
    ApplicationTopology, TopologyBuildError,
};
use boomerang_builder::{Assembly, Reactor};
use boomerang::prelude::*;

const _: () = assert!(
    option_env!("BOOMERANG_DESCRIPTOR_DRIVER").is_some(),
    "workspace resolution must not compile topology packages"
);

#[reactor(
    contract = "vehicle.controller",
    contract_version = 1,
    bounds(
        queue_capacity = 16,
        payload_bytes = 1024,
        state_bytes = 512,
        scratch_bytes = 256,
    )
)]
fn ControllerTopology() -> impl Reactor {
    reaction! { control (startup) {} }
    mode! { initial active {
        reaction! { (shutdown) {} }
    } }
}

#[reactor(
    contract = "vehicle.sensor",
    contract_version = 1,
    bounds(
        queue_capacity = 8,
        payload_bytes = 512,
        state_bytes = 256,
        scratch_bytes = 128,
    )
)]
fn SensorTopology() -> impl Reactor {
    reaction! { sample (startup) {} }
}

/// Builds the fixture's canonical logical topology without constructing a runtime graph.
pub fn topology() -> Result<ApplicationTopology, TopologyBuildError> {
    let mut assembly = Assembly::new();
    ControllerTopology()
        .build("controller", (), None, None, None, true, &mut assembly)
        .expect("fixture controller Assembly is valid");
    SensorTopology()
        .build("sensor", (), None, None, None, true, &mut assembly)
        .expect("fixture sensor Assembly is valid");
    Ok(assembly
        .application_topology()
        .expect("fixture topology projection is valid"))
}
