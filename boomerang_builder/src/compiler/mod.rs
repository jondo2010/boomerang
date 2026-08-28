//! Target-neutral application compiler models.
#![deny(missing_docs)]

mod compiled;
mod debug;
/// Explicit deployment selections supplied to the compiler.
mod deployment;
/// Backend-neutral federation graph analysis.
pub mod federation;
mod from_assembly;
mod identity;
mod model;
/// Canonical implementation and placement resolution.
mod resolved;

pub use compiled::{
    GlobalFederationImage, OwnedCompiledDeployment, OwnedEnclaveImage, OwnedFederateImage,
    RequiredBinding, RequiredBindings,
};
pub use deployment::{
    BoundaryBinding, CoordinationSelection, FederateConfig, ImplementationBinding,
    PlacementAssignment,
};
pub use identity::{
    ActionId, ApplicationId, BindingSlotId, BoundaryId, CodecCapabilityId, ComponentInstanceId,
    ContractId, CoordinationBackendId, FederateId, ImplementationId, InvalidStableId, ModeId,
    PlacementGroupId, PortId, ReactionId, ReactorId, RuntimeBackendId, StableEnclaveId, StablePath,
    StablePathSegment, StableText, TargetTriple, TransportCapabilityId,
};
pub use model::{
    Action, ActionKind, ApplicationTopology, ApplicationTopologyBuilder, BankMember,
    ComponentInstance, Connection, ConnectionSemantics, Enclave, InvalidBankMember, Mode,
    ModeTransition, ModeTransitionKind, PlacementGroup, Port, PortDirection, Reaction,
    ReactionOptions, ReactionRelation, ReactionRelationFlags, ReactionRelationTarget, Reactor,
    TopologyBuildError,
};
pub use resolved::{ResolveError, ResolvedDeployment};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_instance_preserves_explicit_contract_requirement() {
        let component = ComponentInstance::new("vehicle/sensor", "sensor", 7).unwrap();
        assert_eq!(component.id().to_string(), "vehicle/sensor");
        assert_eq!(component.contract().as_str(), "sensor");
        assert_eq!(component.contract_version(), 7);
    }

    #[test]
    fn topology_builder_rejects_duplicate_component_ids() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        builder
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v1", 1).unwrap())
            .unwrap();
        let error = builder
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v2", 1).unwrap())
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
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor", 3).unwrap())
            .unwrap();
        let debug = format!("{:?}", builder.finish().unwrap());
        assert!(debug.contains("vehicle/sensor"));
        assert!(debug.contains("contract: \"sensor\""), "{debug}");
        assert!(debug.contains("contract_version: 3"), "{debug}");
        assert!(!debug.contains("ComponentKey"));
    }
}
