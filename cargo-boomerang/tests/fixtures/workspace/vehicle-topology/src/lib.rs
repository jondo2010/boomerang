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
fn ControllerTopology(#[output] command: u32) -> impl Reactor {
    reaction! { control (startup) -> command { *command = Some(42); } }
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
fn SensorTopology(#[input] command: u32) -> impl Reactor {
    reaction! { sample (command) { ctx.schedule_shutdown(None); } }
}

/// Builds the fixture's canonical logical topology without constructing a runtime graph.
pub fn topology() -> Result<ApplicationTopology, TopologyBuildError> {
    let mut assembly = Assembly::new();
    let controller = ControllerTopology()
        .build("controller", (), None, None, None, true, &mut assembly)
        .expect("fixture controller Assembly is valid");
    ControllerTopology()
        .build("backup", (), None, None, None, true, &mut assembly)
        .expect("fixture backup controller Assembly is valid");
    let sensor = SensorTopology()
        .build("sensor", (), None, None, None, true, &mut assembly)
        .expect("fixture sensor Assembly is valid");
    assembly
        .add_port_connection::<u32, _, _>(controller.command, sensor.command, None, false)
        .expect("fixture route is valid");
    Ok(assembly
        .application_topology()
        .expect("fixture topology projection is valid"))
}
