//! Explicit deployment selections supplied to the compiler.

use crate::descriptor::ComponentDescriptor;

use super::{
    BoundaryId, CodecCapabilityId, ComponentInstanceId, FederateId, ImplementationId,
    PlacementGroupId, RuntimeBackendId, TargetTriple, TransportCapabilityId,
};

/// One selected implementation for a logical component instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImplementationBinding {
    /// Logical component receiving the implementation.
    component: ComponentInstanceId,
    /// Stable identity of the selected implementation.
    implementation: ImplementationId,
    /// Validated structural descriptor supplied by the implementation.
    descriptor: ComponentDescriptor,
}

impl ImplementationBinding {
    /// Creates one component-to-implementation selection.
    pub fn new(
        component: ComponentInstanceId,
        implementation: ImplementationId,
        descriptor: ComponentDescriptor,
    ) -> Self {
        Self {
            component,
            implementation,
            descriptor,
        }
    }

    /// Returns the logical component identity.
    pub fn component(&self) -> &ComponentInstanceId {
        &self.component
    }

    /// Returns the selected implementation identity.
    pub fn implementation(&self) -> &ImplementationId {
        &self.implementation
    }

    /// Returns the selected implementation descriptor.
    pub fn descriptor(&self) -> &ComponentDescriptor {
        &self.descriptor
    }
}

/// One source placement group assigned to a deployment Federate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementAssignment {
    /// Source-declared placement group.
    placement_group: PlacementGroupId,
    /// Federate that owns the group in this deployment.
    federate: FederateId,
}

impl PlacementAssignment {
    /// Creates one placement-group-to-Federate assignment.
    pub fn new(placement_group: PlacementGroupId, federate: FederateId) -> Self {
        Self {
            placement_group,
            federate,
        }
    }

    /// Returns the assigned placement group.
    pub fn placement_group(&self) -> &PlacementGroupId {
        &self.placement_group
    }

    /// Returns the owning Federate.
    pub fn federate(&self) -> &FederateId {
        &self.federate
    }
}

/// Runtime and target configuration selected for one Federate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FederateConfig {
    /// Stable Federate identity.
    id: FederateId,
    /// Rust target triple selected for the Federate artifact.
    target: TargetTriple,
    /// Runtime backend capability selected for the Federate.
    runtime: RuntimeBackendId,
}

impl FederateConfig {
    /// Creates the deployment configuration for one Federate.
    pub fn new(id: FederateId, target: TargetTriple, runtime: RuntimeBackendId) -> Self {
        Self {
            id,
            target,
            runtime,
        }
    }

    /// Returns the configured Federate identity.
    pub fn id(&self) -> &FederateId {
        &self.id
    }

    /// Returns the selected Rust target triple.
    pub fn target(&self) -> &TargetTriple {
        &self.target
    }

    /// Returns the selected runtime backend capability.
    pub fn runtime(&self) -> &RuntimeBackendId {
        &self.runtime
    }
}

/// Coordination backend selected for the deployment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinationSelection {
    /// A one-Federate deployment requires no distributed coordination.
    Local,
    /// Federates coordinate through the selected distributed backend.
    Distributed {
        /// Stable identity of the coordination backend capability.
        backend: super::CoordinationBackendId,
    },
}

/// Codec and transport selections for one logical boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryBinding {
    /// Logical boundary receiving the selections.
    boundary: BoundaryId,
    /// Payload codec capability selected for the boundary.
    codec: CodecCapabilityId,
    /// Transport capability selected for the boundary.
    transport: TransportCapabilityId,
}

impl BoundaryBinding {
    /// Creates the deployment selections for one logical boundary.
    pub fn new(
        boundary: BoundaryId,
        codec: CodecCapabilityId,
        transport: TransportCapabilityId,
    ) -> Self {
        Self {
            boundary,
            codec,
            transport,
        }
    }

    /// Returns the logical boundary identity.
    pub fn boundary(&self) -> &BoundaryId {
        &self.boundary
    }

    /// Returns the selected payload codec capability.
    pub fn codec(&self) -> &CodecCapabilityId {
        &self.codec
    }

    /// Returns the selected transport capability.
    pub fn transport(&self) -> &TransportCapabilityId {
        &self.transport
    }
}
