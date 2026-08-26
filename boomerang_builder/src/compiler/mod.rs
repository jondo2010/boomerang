//! Target-neutral application compiler models.
#![deny(missing_docs)]

mod identity;
mod model;

pub use identity::{
    ActionId, ApplicationId, BindingSlotId, BoundaryId, ComponentInstanceId, ContractId,
    InvalidStableId, ModeId, PlacementGroupId, PortId, ReactionId, ReactorId, StableEnclaveId,
    StablePath, StablePathSegment, StableText,
};
pub use model::{
    Action, ActionKind, ApplicationTopology, ApplicationTopologyBuilder, ComponentInstance,
    Connection, Enclave, Mode, ModeTransition, ModeTransitionKind, PlacementGroup, Port,
    PortDirection, Reaction, ReactionOptions, ReactionRelation, ReactionRelationFlags,
    ReactionRelationTarget, Reactor, TopologyBuildError,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_instance_uses_stable_component_and_contract_ids() {
        let component = ComponentInstance::new("vehicle/sensor", "sensor.v1").unwrap();
        assert_eq!(component.id().to_string(), "vehicle/sensor");
        assert_eq!(component.contract().as_str(), "sensor.v1");
    }

    #[test]
    fn topology_builder_rejects_duplicate_component_ids() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        builder
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v1").unwrap())
            .unwrap();
        let error = builder
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v2").unwrap())
            .unwrap_err();
        assert!(
            matches!(error, TopologyBuildError::DuplicateIdentity { kind: "component", id } if id == "vehicle/sensor")
        );
        let topology = builder.finish().unwrap();
        let sensor = ComponentInstanceId::new("vehicle/sensor").unwrap();
        assert_eq!(
            topology.component(&sensor).unwrap().contract().as_str(),
            "sensor.v1"
        );
    }

    #[test]
    fn topology_debug_uses_stable_identity_not_dense_keys() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        builder
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v1").unwrap())
            .unwrap();
        let debug = format!("{:?}", builder.finish().unwrap());
        assert!(debug.contains("vehicle/sensor"));
        assert!(!debug.contains("ComponentKey"));
    }
}
