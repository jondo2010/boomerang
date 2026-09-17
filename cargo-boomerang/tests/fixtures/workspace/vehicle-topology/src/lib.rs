#[cfg(not(boomerang_workspace_config_probe))]
compile_error!("Cargo command ignored workspace configuration");

use boomerang::prelude::Duration;
use boomerang_builder::compiler::{
    ApplicationTopology, ConnectionSemantics, TopologyAuthoringError, TopologyBuilder,
};

const _: () = assert!(
    option_env!("BOOMERANG_DESCRIPTOR_DRIVER").is_some(),
    "workspace resolution must not compile topology packages"
);

/// Builds the fixture's canonical logical topology without constructing a runtime graph.
pub fn topology() -> Result<ApplicationTopology, TopologyAuthoringError> {
    topology_with_delay(None)
}

/// Uses a nonzero route tag for independent-process transport verification.
pub fn tagged_topology() -> Result<ApplicationTopology, TopologyAuthoringError> {
    topology_with_delay(Some(Duration::milliseconds(1)))
}

/// Builds the shared fixture with the requested logical boundary delay.
fn topology_with_delay(
    delay: Option<Duration>,
) -> Result<ApplicationTopology, TopologyAuthoringError> {
    let mut app = TopologyBuilder::new("application/backup/controller/sensor")?;
    let controller_enclave = app.enclave("controller")?;
    let backup_enclave = app.enclave("backup")?;
    let sensor_enclave = app.enclave("sensor")?;
    let controller = app.component(
        "controller",
        vehicle_control::controller::definition(),
        &controller_enclave,
    )?;
    app.component(
        "backup",
        vehicle_control::controller::definition(),
        &backup_enclave,
    )?;
    let sensor = app.component("sensor", sensor_host::sensor::definition(), &sensor_enclave)?;
    app.connect_with_semantics(
        &controller.command,
        &sensor.command,
        ConnectionSemantics::Logical { after: delay },
    )?;
    app.finish()
}
