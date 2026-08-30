use boomerang_builder::compiler::{
    ApplicationTopology, ApplicationTopologyBuilder, ComponentInstance, TopologyBuildError,
};

const _: () = assert!(
    option_env!("BOOMERANG_DESCRIPTOR_DRIVER").is_some(),
    "workspace resolution must not compile topology packages"
);

/// Builds the fixture's canonical logical topology without constructing a runtime graph.
pub fn topology() -> Result<ApplicationTopology, TopologyBuildError> {
    let mut topology =
        ApplicationTopologyBuilder::new("vehicle").expect("fixture application ID is valid");
    topology.add_component(
        ComponentInstance::new("vehicle/controller", "vehicle.controller", 1)
            .expect("fixture component IDs are valid"),
    )?;
    topology.add_component(
        ComponentInstance::new("vehicle/sensor", "vehicle.sensor", 1)
            .expect("fixture component IDs are valid"),
    )?;
    topology.finish()
}
