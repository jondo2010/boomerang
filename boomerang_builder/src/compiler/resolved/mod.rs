//! Canonical deployment selections resolved before image lowering.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    ApplicationTopology, BoundaryBinding, BoundaryId, ComponentInstanceId, ContractId,
    CoordinationSelection, FederateConfig, FederateId, ImplementationBinding, ImplementationId,
    PlacementAssignment, PlacementGroupId, ReactorId, StableEnclaveId,
};

#[cfg(test)]
mod tests;

/// Failure while resolving implementation and placement selections.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ResolveError {
    /// More than one implementation was selected for a component.
    #[error("duplicate implementation binding for component `{component}`")]
    DuplicateComponentBinding {
        /// Component with multiple selections.
        component: ComponentInstanceId,
    },
    /// An implementation was selected for a component absent from the topology.
    #[error("implementation `{implementation}` targets unknown component `{component}`")]
    UnknownComponentBinding {
        /// Unknown component identity.
        component: ComponentInstanceId,
        /// Selected implementation identity.
        implementation: ImplementationId,
    },
    /// A topology component has no selected implementation.
    #[error("component `{component}` has no selected implementation")]
    MissingComponentBinding {
        /// Component missing its selection.
        component: ComponentInstanceId,
    },
    /// A selected descriptor does not satisfy the component contract.
    #[error(
        "implementation `{implementation}` provides contract `{provided}` version {provided_version} but component `{component}` requires `{required}` version {required_version}"
    )]
    ContractMismatch {
        /// Component receiving the incompatible implementation.
        component: ComponentInstanceId,
        /// Contract required by the topology.
        required: ContractId,
        /// Contract version required by the topology.
        required_version: u64,
        /// Contract provided by the descriptor.
        provided: ContractId,
        /// Contract version provided by the descriptor.
        provided_version: u64,
        /// Selected implementation identity.
        implementation: ImplementationId,
    },
    /// More than one Federate was selected for a placement group.
    #[error("duplicate assignment for placement group `{placement_group}`")]
    DuplicatePlacementAssignment {
        /// Placement group with multiple assignments.
        placement_group: PlacementGroupId,
    },
    /// A deployment assigns a placement group absent from the topology.
    #[error("assignment targets unknown placement group `{placement_group}`")]
    UnknownPlacementGroup {
        /// Unknown placement-group identity.
        placement_group: PlacementGroupId,
    },
    /// A topology placement group has no owning Federate.
    #[error("placement group `{placement_group}` has no Federate assignment")]
    MissingPlacementAssignment {
        /// Placement group missing its assignment.
        placement_group: PlacementGroupId,
    },
    /// More than one configuration was supplied for a Federate.
    #[error("duplicate configuration for Federate `{federate}`")]
    DuplicateFederateConfig {
        /// Federate with multiple configurations.
        federate: FederateId,
    },
    /// More than one binding was supplied for a logical boundary.
    #[error("duplicate binding for boundary `{boundary}`")]
    DuplicateBoundaryBinding {
        /// Boundary with multiple selections.
        boundary: BoundaryId,
    },
    /// A cross-Federate topology boundary has no selected codec and transport.
    #[error("cross-Federate boundary `{boundary}` has no binding")]
    MissingBoundaryBinding {
        /// Cross-Federate topology boundary missing its selections.
        boundary: BoundaryId,
    },
    /// A binding targets a topology boundary that remains within one Federate.
    #[error("boundary binding for local boundary `{boundary}` is unexpected")]
    UnexpectedBoundaryBinding {
        /// Same-Federate topology boundary with superfluous selections.
        boundary: BoundaryId,
    },
    /// A binding targets no topology boundary.
    #[error("boundary binding targets unknown boundary `{boundary}`")]
    UnknownBoundaryBinding {
        /// Unknown topology boundary identity.
        boundary: BoundaryId,
    },
    /// A placement assignment references a Federate without a configuration.
    #[error(
        "placement group `{placement_group}` is assigned to Federate `{federate}` without a configuration"
    )]
    MissingFederateConfig {
        /// Placement group with the unresolved Federate assignment.
        placement_group: PlacementGroupId,
        /// Federate absent from the supplied configurations.
        federate: FederateId,
    },
    /// A supplied Federate configuration is not selected by any placement assignment.
    #[error("configuration for Federate `{federate}` is unused")]
    UnusedFederateConfig {
        /// Federate with no placement assignments.
        federate: FederateId,
    },
    /// A topology reactor does not belong to a source placement group.
    #[error("reactor `{reactor}` has no placement group")]
    UnplacedReactor {
        /// Reactor missing source placement.
        reactor: ReactorId,
    },
    /// Reactors in one Enclave resolve to different Federates.
    #[error("Enclave `{enclave}` is split between Federates `{first}` and `{second}`")]
    SplitEnclave {
        /// Enclave with conflicting Federate owners.
        enclave: StableEnclaveId,
        /// First resolved Federate owner in stable reactor order.
        first: FederateId,
        /// Conflicting Federate owner.
        second: FederateId,
    },
    /// No Federates are selected by the deployment placement assignments.
    #[error("deployment resolves to no Federates")]
    NoFederates,
    /// The coordination backend does not match the resolved Federate count.
    #[error("coordination `{coordination:?}` is invalid for {federate_count} resolved Federates")]
    InvalidCoordination {
        /// Number of canonical Federates selected by placement assignments.
        federate_count: usize,
        /// Coordination selection incompatible with that count.
        coordination: CoordinationSelection,
    },
}

/// Complete canonical deployment resolution for one topology.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedDeployment {
    /// Target-neutral application structure.
    topology: ApplicationTopology,
    /// Component implementation selections in stable component order.
    bindings: BTreeMap<ComponentInstanceId, ImplementationBinding>,
    /// Placement ownership in stable group order.
    placements: BTreeMap<PlacementGroupId, PlacementAssignment>,
    /// Federate configurations in stable identity order.
    federates: BTreeMap<FederateId, FederateConfig>,
    /// Coordination backend selection for this deployment.
    coordination: CoordinationSelection,
    /// Boundary codec and transport selections in stable boundary order.
    boundary_bindings: BTreeMap<BoundaryId, BoundaryBinding>,
}

impl ResolvedDeployment {
    /// Validates and canonicalizes all deployment selections.
    pub fn new(
        topology: ApplicationTopology,
        bindings: impl IntoIterator<Item = ImplementationBinding>,
        placements: impl IntoIterator<Item = PlacementAssignment>,
        federates: impl IntoIterator<Item = FederateConfig>,
        coordination: CoordinationSelection,
        boundary_bindings: impl IntoIterator<Item = BoundaryBinding>,
    ) -> Result<Self, ResolveError> {
        let mut canonical_bindings = BTreeMap::new();
        let mut duplicate_component_bindings = BTreeSet::new();
        for binding in bindings {
            let component = binding.component().clone();
            if canonical_bindings
                .insert(component.clone(), binding)
                .is_some()
            {
                duplicate_component_bindings.insert(component);
            }
        }
        if let Some(component) = duplicate_component_bindings.into_iter().next() {
            return Err(ResolveError::DuplicateComponentBinding { component });
        }

        let mut canonical_placements = BTreeMap::new();
        let mut duplicate_placement_assignments = BTreeSet::new();
        for assignment in placements {
            let placement_group = assignment.placement_group().clone();
            if canonical_placements
                .insert(placement_group.clone(), assignment)
                .is_some()
            {
                duplicate_placement_assignments.insert(placement_group);
            }
        }
        if let Some(placement_group) = duplicate_placement_assignments.into_iter().next() {
            return Err(ResolveError::DuplicatePlacementAssignment { placement_group });
        }

        let mut canonical_federates = BTreeMap::new();
        let mut duplicate_federate_configs = BTreeSet::new();
        for federate in federates {
            let federate_id = federate.id().clone();
            if canonical_federates
                .insert(federate_id.clone(), federate)
                .is_some()
            {
                duplicate_federate_configs.insert(federate_id);
            }
        }
        if let Some(federate) = duplicate_federate_configs.into_iter().next() {
            return Err(ResolveError::DuplicateFederateConfig { federate });
        }

        let mut canonical_boundary_bindings = BTreeMap::new();
        let mut duplicate_boundary_bindings = BTreeSet::new();
        for binding in boundary_bindings {
            let boundary = binding.boundary().clone();
            if canonical_boundary_bindings
                .insert(boundary.clone(), binding)
                .is_some()
            {
                duplicate_boundary_bindings.insert(boundary);
            }
        }
        if let Some(boundary) = duplicate_boundary_bindings.into_iter().next() {
            return Err(ResolveError::DuplicateBoundaryBinding { boundary });
        }

        for binding in canonical_bindings.values() {
            if topology.component(binding.component()).is_none() {
                return Err(ResolveError::UnknownComponentBinding {
                    component: binding.component().clone(),
                    implementation: binding.implementation().clone(),
                });
            }
        }
        for (component_id, component) in topology.components() {
            let binding = canonical_bindings.get(component_id).ok_or_else(|| {
                ResolveError::MissingComponentBinding {
                    component: component_id.clone(),
                }
            })?;
            let provided = binding.descriptor().contract_id();
            let provided_version = binding.descriptor().contract_version();
            if component.contract() != provided || component.contract_version() != provided_version
            {
                return Err(ResolveError::ContractMismatch {
                    component: component_id.clone(),
                    required: component.contract().clone(),
                    required_version: component.contract_version(),
                    provided: provided.clone(),
                    provided_version,
                    implementation: binding.implementation().clone(),
                });
            }
        }

        for placement_group in canonical_placements.keys() {
            if topology.placement_group(placement_group).is_none() {
                return Err(ResolveError::UnknownPlacementGroup {
                    placement_group: placement_group.clone(),
                });
            }
        }
        for (placement_group, _) in topology.placement_groups() {
            if !canonical_placements.contains_key(placement_group) {
                return Err(ResolveError::MissingPlacementAssignment {
                    placement_group: placement_group.clone(),
                });
            }
        }

        for assignment in canonical_placements.values() {
            if !canonical_federates.contains_key(assignment.federate()) {
                return Err(ResolveError::MissingFederateConfig {
                    placement_group: assignment.placement_group().clone(),
                    federate: assignment.federate().clone(),
                });
            }
        }
        for federate in canonical_federates.keys() {
            if !canonical_placements
                .values()
                .any(|assignment| assignment.federate() == federate)
            {
                return Err(ResolveError::UnusedFederateConfig {
                    federate: federate.clone(),
                });
            }
        }

        if canonical_federates.is_empty() {
            return Err(ResolveError::NoFederates);
        }
        let federate_count = canonical_federates.len();
        if !matches!(
            (&coordination, federate_count),
            (CoordinationSelection::Local, 1) | (CoordinationSelection::Distributed { .. }, 2..)
        ) {
            return Err(ResolveError::InvalidCoordination {
                federate_count,
                coordination,
            });
        }

        let mut reactor_federates = BTreeMap::new();
        let mut enclave_federates = BTreeMap::new();
        for (reactor_id, reactor) in topology.reactors() {
            let placement_group =
                reactor
                    .placement_group()
                    .ok_or_else(|| ResolveError::UnplacedReactor {
                        reactor: reactor_id.clone(),
                    })?;
            let federate = canonical_placements
                .get(placement_group)
                .expect("all topology placement groups are assigned")
                .federate()
                .clone();
            reactor_federates.insert(reactor_id.clone(), federate.clone());

            if let Some(first) =
                enclave_federates.insert(reactor.enclave().clone(), federate.clone())
            {
                if first != federate {
                    return Err(ResolveError::SplitEnclave {
                        enclave: reactor.enclave().clone(),
                        first,
                        second: federate,
                    });
                }
            }
        }

        let mut cross_federate_boundaries = BTreeSet::new();
        for (boundary, connection) in topology.connections() {
            let source_reactor = topology
                .port(connection.source())
                .expect("topology connections reference source ports")
                .reactor();
            let target_reactor = topology
                .port(connection.target())
                .expect("topology connections reference target ports")
                .reactor();
            let source_federate = reactor_federates
                .get(source_reactor)
                .expect("topology ports reference placed source reactors");
            let target_federate = reactor_federates
                .get(target_reactor)
                .expect("topology ports reference placed target reactors");
            if source_federate != target_federate {
                cross_federate_boundaries.insert(boundary.clone());
            }
        }

        for boundary in canonical_boundary_bindings.keys() {
            if topology.connection(boundary).is_none() {
                return Err(ResolveError::UnknownBoundaryBinding {
                    boundary: boundary.clone(),
                });
            }
            if !cross_federate_boundaries.contains(boundary) {
                return Err(ResolveError::UnexpectedBoundaryBinding {
                    boundary: boundary.clone(),
                });
            }
        }
        for boundary in cross_federate_boundaries {
            if !canonical_boundary_bindings.contains_key(&boundary) {
                return Err(ResolveError::MissingBoundaryBinding { boundary });
            }
        }

        Ok(Self {
            topology,
            bindings: canonical_bindings,
            placements: canonical_placements,
            federates: canonical_federates,
            coordination,
            boundary_bindings: canonical_boundary_bindings,
        })
    }

    /// Returns the resolved application topology.
    pub fn topology(&self) -> &ApplicationTopology {
        &self.topology
    }

    /// Iterates implementation bindings in stable component order.
    pub fn bindings(&self) -> impl Iterator<Item = &ImplementationBinding> {
        self.bindings.values()
    }

    /// Looks up the selected implementation for a component.
    pub fn binding(&self, component: &ComponentInstanceId) -> Option<&ImplementationBinding> {
        self.bindings.get(component)
    }

    /// Iterates placement assignments in stable group order.
    pub fn placements(&self) -> impl Iterator<Item = &PlacementAssignment> {
        self.placements.values()
    }

    /// Looks up the Federate assignment for a placement group.
    pub fn placement(&self, placement_group: &PlacementGroupId) -> Option<&PlacementAssignment> {
        self.placements.get(placement_group)
    }

    /// Iterates Federate configurations in stable identity order.
    pub fn federates(&self) -> impl Iterator<Item = &FederateConfig> {
        self.federates.values()
    }

    /// Looks up the configuration selected for a Federate.
    pub fn federate(&self, federate: &FederateId) -> Option<&FederateConfig> {
        self.federates.get(federate)
    }

    /// Returns the selected coordination backend.
    pub fn coordination(&self) -> &CoordinationSelection {
        &self.coordination
    }

    /// Iterates boundary bindings in stable boundary order.
    pub fn boundary_bindings(&self) -> impl Iterator<Item = &BoundaryBinding> {
        self.boundary_bindings.values()
    }

    /// Looks up the selections bound to a logical boundary.
    pub fn boundary_binding(&self, boundary: &BoundaryId) -> Option<&BoundaryBinding> {
        self.boundary_bindings.get(boundary)
    }
}
