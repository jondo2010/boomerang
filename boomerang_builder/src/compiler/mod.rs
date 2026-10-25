//! Target-neutral application compiler models.
//!
//! # Ownership
//!
//! This module owns topology authoring, implementation resolution, federation analysis, lowering,
//! and host-owned image storage. Component descriptors share this crate's canonical identity types.
//! Semantic resolution and lowering must remain independent of Rust source emission and Cargo
//! publication. Keep target Rust export paths outside these semantic models.
//!
//! [`crate::Assembly::application_topology`] adapts hosted assembly records into these models.
//! Preserve the hosted assembly API without introducing assembly keys into compiler identities.
//!
//! `boomerang_codegen` owns Rust source emission and the target export-path table. Its optional
//! `cargo-build` adapter owns Cargo input tracking and generated-source publication.
//! [`boomerang_runtime::image`] owns the target schema consumed by generated Rust.
//! Host storage must lend borrowed image views without transferring its allocations into target
//! static storage.
//!
//! # Representation boundary
//!
//! Semantic compiler data—topology, deployment selections, resolution, and analysis—uses stable
//! typed identities. Those models do not retain runtime table keys, dense-key spans, or packed
//! slice coordinates. Stable identities remain meaningful across compiler runs and at diagnostic,
//! configuration, and interchange boundaries.
//!
//! [`ResolvedDeployment::lower`](crate::compiler::ResolvedDeployment::lower) is the one-way
//! transition into the runtime image domain:
//!
//! ```text
//! ApplicationTopology + deployment selections
//!                    -> ResolvedDeployment
//! ResolvedDeployment::lower()
//!                    -> OwnedCompiledDeployment
//! ```
//!
//! Lowering may temporarily map stable identities to runtime keys while resolving references, but
//! that state is private to image materialization. It must not leak back into semantic models.
#![deny(missing_docs)]

mod authoring;
mod compiled;
mod coordination;
mod debug;
/// Explicit deployment selections supplied to the compiler.
mod deployment;
/// Backend-neutral federation graph analysis.
pub mod federation;
mod from_assembly;
mod identity;
mod lower;
mod model;
mod packed;
/// Canonical implementation and placement resolution.
mod resolved;

pub use authoring::{
    ComponentDefinition, ComponentTopology, TopologyAuthoringError, TopologyBuilder,
    TopologyEnclave, TopologyInput, TopologyOutput,
};
pub use compiled::{
    direct_binding_symbol, CompiledDeploymentValidationError, FederateSlice, GlobalFederationImage,
    OwnedCompiledDeployment, OwnedEnclaveImage, OwnedFederateImage, RequiredBinding,
    RequiredBindings, ResolvedInitializerAction, ResolvedInitializerCapabilities,
};
pub use coordination::{CoordinationProjectionError, OwnedCoordinationProjection, OwnedRtiImage};
pub use deployment::{
    BoundaryBinding, BoundaryPolicies, CoordinationBackend, CoordinationSelection, FederateConfig,
    ImplementationBinding, ImplementationCatalog, PhysicalBoundaryMetadata, PlacementAssignment,
};
pub use identity::{
    ActionId, ApplicationId, BindingSlotId, BoundaryId, CodecCapabilityId, ComponentInstanceId,
    ContractId, FederateId, FlowId, ImplementationId, InvalidStableId, ModeId, PhysicalBoundaryId,
    PlacementGroupId, PortId, ReactionId, ReactorId, RuntimeBackendId, StableEnclaveId, StablePath,
    StablePathSegment, StableText, TargetTriple, TransportCapabilityId,
};
pub use lower::CompileError;
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
