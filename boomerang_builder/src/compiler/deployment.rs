//! Explicit deployment selections resolved before canonical analysis and projection.
//!
//! A [`FederateConfig`] supplies build/runtime capabilities plus an explicit recovery policy.
//! Each [`BoundaryBinding`] attaches one topology boundary to a stable end-to-end flow, optional
//! physical input/output endpoints, concrete codec and transport implementations, and five
//! safety-relevant policy identities. A single flow may span multiple boundary identities. No
//! policy default is invented here: manifest parsing preserves the full roadmap vocabulary and
//! compilation rejects unknown or known-but-unsupported selections before target images escape.

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
    /// Explicit recovery behavior selected for this closed-world member.
    recovery: super::RecoveryPolicyId,
}

impl FederateConfig {
    /// Creates the deployment configuration for one Federate.
    pub fn new(
        id: FederateId,
        target: TargetTriple,
        runtime: RuntimeBackendId,
        recovery: super::RecoveryPolicyId,
    ) -> Self {
        Self {
            id,
            target,
            runtime,
            recovery,
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

    /// Returns the selected recovery policy identity.
    pub fn recovery(&self) -> &super::RecoveryPolicyId {
        &self.recovery
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

/// Optional identities that delimit an end-to-end physical response interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalBoundaryMetadata {
    /// Physical input where external data enters Boomerang.
    input: Option<super::PhysicalBoundaryId>,
    /// Physical output where Boomerang commits an external effect.
    output: Option<super::PhysicalBoundaryId>,
}

impl PhysicalBoundaryMetadata {
    /// Creates physical endpoint metadata; either role may be absent.
    pub fn new(
        input: Option<super::PhysicalBoundaryId>,
        output: Option<super::PhysicalBoundaryId>,
    ) -> Self {
        Self { input, output }
    }

    /// Returns the physical input identity, when this route admits external data.
    pub fn input(&self) -> Option<&super::PhysicalBoundaryId> {
        self.input.as_ref()
    }

    /// Returns the physical output identity, when this route commits an external effect.
    pub fn output(&self) -> Option<&super::PhysicalBoundaryId> {
        self.output.as_ref()
    }
}

/// Explicit policy identities selected for one cross-Federate boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryPolicies {
    failure: super::BoundaryFailurePolicyId,
    transport: super::TransportPolicyId,
    codec: super::CodecPolicyId,
    timing: super::TimingPolicyId,
    security: super::SecurityPolicyId,
}

impl BoundaryPolicies {
    /// Creates one complete boundary policy selection without implicit defaults.
    pub fn new(
        failure: super::BoundaryFailurePolicyId,
        transport: super::TransportPolicyId,
        codec: super::CodecPolicyId,
        timing: super::TimingPolicyId,
        security: super::SecurityPolicyId,
    ) -> Self {
        Self {
            failure,
            transport,
            codec,
            timing,
            security,
        }
    }

    /// Returns the selected source-loss behavior.
    pub fn failure(&self) -> &super::BoundaryFailurePolicyId {
        &self.failure
    }
    /// Returns the selected transport contract.
    pub fn transport(&self) -> &super::TransportPolicyId {
        &self.transport
    }
    /// Returns the selected codec contract.
    pub fn codec(&self) -> &super::CodecPolicyId {
        &self.codec
    }
    /// Returns the selected timing class.
    pub fn timing(&self) -> &super::TimingPolicyId {
        &self.timing
    }
    /// Returns the selected security profile.
    pub fn security(&self) -> &super::SecurityPolicyId {
        &self.security
    }
}

/// Flow, physical endpoint, capability, and policy selections for one boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryBinding {
    /// Logical boundary receiving the selections.
    boundary: BoundaryId,
    /// Stable end-to-end flow containing this boundary.
    flow: super::FlowId,
    /// Optional physical input and output endpoint identities.
    physical: PhysicalBoundaryMetadata,
    /// Payload codec capability selected for the boundary.
    codec: CodecCapabilityId,
    /// Transport capability selected for the boundary.
    transport: TransportCapabilityId,
    /// Explicit safety-relevant policy selections.
    policies: BoundaryPolicies,
}

impl BoundaryBinding {
    /// Creates the deployment selections for one logical boundary.
    pub fn new(
        boundary: BoundaryId,
        flow: super::FlowId,
        physical: PhysicalBoundaryMetadata,
        codec: CodecCapabilityId,
        transport: TransportCapabilityId,
        policies: BoundaryPolicies,
    ) -> Self {
        Self {
            boundary,
            flow,
            physical,
            codec,
            transport,
            policies,
        }
    }

    /// Returns the logical boundary identity.
    pub fn boundary(&self) -> &BoundaryId {
        &self.boundary
    }

    /// Returns the end-to-end flow containing this boundary.
    pub fn flow(&self) -> &super::FlowId {
        &self.flow
    }

    /// Returns optional physical input and output endpoint metadata.
    pub fn physical(&self) -> &PhysicalBoundaryMetadata {
        &self.physical
    }

    /// Returns the selected payload codec capability.
    pub fn codec(&self) -> &CodecCapabilityId {
        &self.codec
    }

    /// Returns the selected transport capability.
    pub fn transport(&self) -> &TransportCapabilityId {
        &self.transport
    }

    /// Returns the complete explicit boundary policy selection.
    pub fn policies(&self) -> &BoundaryPolicies {
        &self.policies
    }
}
