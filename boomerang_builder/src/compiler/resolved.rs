//! Canonical deployment selections resolved before image lowering.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    ApplicationTopology, BoundaryBinding, BoundaryId, ComponentInstanceId, ContractId,
    CoordinationSelection, FederateConfig, FederateId, ImplementationBinding, ImplementationId,
    PlacementAssignment, PlacementGroupId, ReactorId, StableEnclaveId,
};

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
        for binding in bindings {
            let component = binding.component().clone();
            if canonical_bindings
                .insert(component.clone(), binding)
                .is_some()
            {
                return Err(ResolveError::DuplicateComponentBinding { component });
            }
        }

        let mut canonical_placements = BTreeMap::new();
        for assignment in placements {
            let placement_group = assignment.placement_group().clone();
            if canonical_placements
                .insert(placement_group.clone(), assignment)
                .is_some()
            {
                return Err(ResolveError::DuplicatePlacementAssignment { placement_group });
            }
        }

        let mut canonical_federates = BTreeMap::new();
        for federate in federates {
            let federate_id = federate.id().clone();
            if canonical_federates
                .insert(federate_id.clone(), federate)
                .is_some()
            {
                return Err(ResolveError::DuplicateFederateConfig {
                    federate: federate_id,
                });
            }
        }

        let mut canonical_boundary_bindings = BTreeMap::new();
        for binding in boundary_bindings {
            let boundary = binding.boundary().clone();
            if canonical_boundary_bindings
                .insert(boundary.clone(), binding)
                .is_some()
            {
                return Err(ResolveError::DuplicateBoundaryBinding { boundary });
            }
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

#[cfg(test)]
mod tests {
    use crate::compiler::{
        ApplicationTopology, ApplicationTopologyBuilder, BoundaryBinding, BoundaryId,
        CodecCapabilityId, ComponentInstance, ComponentInstanceId, ConnectionSemantics, ContractId,
        CoordinationBackendId, CoordinationSelection, FederateConfig, FederateId,
        ImplementationBinding, ImplementationId, PlacementAssignment, PlacementGroupId,
        PortDirection, PortId, Reactor, ReactorId, ResolveError, ResolvedDeployment,
        RuntimeBackendId, StableEnclaveId, TargetTriple, TransportCapabilityId,
    };
    use crate::descriptor::{
        ComponentDescriptor, DescriptorBounds, COMPONENT_DESCRIPTOR_MACRO_ABI,
    };

    fn descriptor_at_version(contract: &str, contract_version: u64) -> ComponentDescriptor {
        ComponentDescriptor::try_new(
            ContractId::new(contract).unwrap(),
            contract_version,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            DescriptorBounds::default(),
        )
        .unwrap()
    }

    fn descriptor(contract: &str) -> ComponentDescriptor {
        descriptor_at_version(contract, 1)
    }

    fn topology() -> ApplicationTopology {
        topology_with_layout(true, true, false)
    }

    fn topology_with_layout(
        controller_is_placed: bool,
        sensor_is_placed: bool,
        shared_enclave: bool,
    ) -> ApplicationTopology {
        let mut topology = ApplicationTopologyBuilder::new("vehicle").unwrap();
        let controller_component = ComponentInstanceId::new("vehicle/controller").unwrap();
        let sensor_component = ComponentInstanceId::new("vehicle/sensor").unwrap();
        let controller_reactor = ReactorId::new("vehicle/controller").unwrap();
        let sensor_reactor = ReactorId::new("vehicle/sensor").unwrap();
        let controller_enclave = StableEnclaveId::new("vehicle/controller").unwrap();
        let sensor_enclave = StableEnclaveId::new("vehicle/sensor").unwrap();
        let controller_placement_group = PlacementGroupId::new("placement/controller").unwrap();
        let sensor_placement_group = PlacementGroupId::new("placement/sensor").unwrap();
        topology
            .add_component(
                ComponentInstance::new("vehicle/controller", "controller.v1", 1).unwrap(),
            )
            .unwrap();
        topology
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v1", 1).unwrap())
            .unwrap();
        topology
            .add_placement_group(controller_placement_group.clone(), None)
            .unwrap();
        topology
            .add_placement_group(sensor_placement_group.clone(), None)
            .unwrap();
        topology
            .add_reactor(Reactor::new(
                controller_reactor.clone(),
                controller_component,
                None,
                None,
                controller_enclave.clone(),
                controller_is_placed.then_some(controller_placement_group),
                None,
            ))
            .unwrap();
        topology
            .add_reactor(Reactor::new(
                sensor_reactor.clone(),
                sensor_component,
                None,
                None,
                if shared_enclave {
                    controller_enclave.clone()
                } else {
                    sensor_enclave.clone()
                },
                sensor_is_placed.then_some(sensor_placement_group),
                None,
            ))
            .unwrap();
        topology
            .add_enclave(controller_enclave, controller_reactor.clone())
            .unwrap();
        if !shared_enclave {
            topology
                .add_enclave(sensor_enclave, sensor_reactor.clone())
                .unwrap();
        }
        let controller_port = PortId::new("vehicle/controller/output").unwrap();
        let sensor_port = PortId::new("vehicle/sensor/input").unwrap();
        topology
            .add_port(
                controller_port.clone(),
                controller_reactor,
                PortDirection::Output,
                None,
                0,
                None,
            )
            .unwrap();
        topology
            .add_port(
                sensor_port.clone(),
                sensor_reactor,
                PortDirection::Input,
                None,
                0,
                None,
            )
            .unwrap();
        topology
            .add_connection(
                BoundaryId::new("controller-to-sensor").unwrap(),
                controller_port,
                sensor_port,
                ConnectionSemantics::Logical { after: None },
            )
            .unwrap();
        topology.finish().unwrap()
    }

    fn binding(component: &str, implementation: &str, contract: &str) -> ImplementationBinding {
        ImplementationBinding::new(
            ComponentInstanceId::new(component).unwrap(),
            ImplementationId::new(implementation).unwrap(),
            descriptor(contract),
        )
    }

    fn placement(group: &str, federate: &str) -> PlacementAssignment {
        PlacementAssignment::new(
            PlacementGroupId::new(group).unwrap(),
            FederateId::new(federate).unwrap(),
        )
    }

    fn federate(id: &str, target: &str, runtime: &str) -> FederateConfig {
        FederateConfig::new(
            FederateId::new(id).unwrap(),
            TargetTriple::new(target).unwrap(),
            RuntimeBackendId::new(runtime).unwrap(),
        )
    }

    fn boundary_binding(boundary: &str, codec: &str, transport: &str) -> BoundaryBinding {
        BoundaryBinding::new(
            BoundaryId::new(boundary).unwrap(),
            CodecCapabilityId::new(codec).unwrap(),
            TransportCapabilityId::new(transport).unwrap(),
        )
    }

    fn distributed_coordination() -> CoordinationSelection {
        CoordinationSelection::Distributed {
            backend: CoordinationBackendId::new("rti").unwrap(),
        }
    }

    fn standard_federates() -> [FederateConfig; 2] {
        [
            federate("host", "x86_64-unknown-linux-gnu", "native"),
            federate("edge", "aarch64-unknown-linux-gnu", "rtic"),
        ]
    }

    fn controller_to_sensor_binding() -> BoundaryBinding {
        boundary_binding("controller-to-sensor", "serde-json", "quic")
    }

    fn resolve(reverse: bool) -> ResolvedDeployment {
        let mut bindings = vec![
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ];
        let mut placements = vec![
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ];
        let mut federates = standard_federates().into_iter().collect::<Vec<_>>();
        let mut boundary_bindings = vec![controller_to_sensor_binding()];
        if reverse {
            bindings.reverse();
            placements.reverse();
            federates.reverse();
            boundary_bindings.reverse();
        }
        ResolvedDeployment::new(
            topology(),
            bindings,
            placements,
            federates,
            distributed_coordination(),
            boundary_bindings,
        )
        .unwrap()
    }

    #[test]
    fn resolution_requires_complete_reactor_placement_and_enclave_ownership() {
        let unplaced_reactor = ResolvedDeployment::new(
            topology_with_layout(true, false, false),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();
        assert!(matches!(
            unplaced_reactor,
            ResolveError::UnplacedReactor { reactor }
                if reactor.to_string() == "vehicle/sensor"
        ));

        let split_enclave = ResolvedDeployment::new(
            topology_with_layout(true, true, true),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();
        assert!(matches!(
            split_enclave,
            ResolveError::SplitEnclave {
                enclave,
                first,
                second,
            } if enclave.to_string() == "vehicle/controller"
                && first.as_str() == "host"
                && second.as_str() == "edge"
        ));

        let no_federates = ResolvedDeployment::new(
            ApplicationTopologyBuilder::new("empty")
                .unwrap()
                .finish()
                .unwrap(),
            [],
            [],
            [],
            CoordinationSelection::Local,
            [],
        )
        .unwrap_err();
        assert!(matches!(no_federates, ResolveError::NoFederates));
    }

    #[test]
    fn resolution_requires_coordination_matching_federate_cardinality() {
        let cases = [
            (
                "one Federate with local coordination",
                vec![
                    placement("placement/controller", "host"),
                    placement("placement/sensor", "host"),
                ],
                vec![federate("host", "x86_64-unknown-linux-gnu", "native")],
                CoordinationSelection::Local,
                vec![],
                1,
                true,
            ),
            (
                "one Federate with distributed coordination",
                vec![
                    placement("placement/controller", "host"),
                    placement("placement/sensor", "host"),
                ],
                vec![federate("host", "x86_64-unknown-linux-gnu", "native")],
                distributed_coordination(),
                vec![],
                1,
                false,
            ),
            (
                "two Federates with local coordination",
                vec![
                    placement("placement/controller", "host"),
                    placement("placement/sensor", "edge"),
                ],
                standard_federates().into(),
                CoordinationSelection::Local,
                vec![controller_to_sensor_binding()],
                2,
                false,
            ),
            (
                "two Federates with distributed coordination",
                vec![
                    placement("placement/controller", "host"),
                    placement("placement/sensor", "edge"),
                ],
                standard_federates().into(),
                distributed_coordination(),
                vec![controller_to_sensor_binding()],
                2,
                true,
            ),
        ];

        for (
            name,
            placements,
            federates,
            coordination,
            boundary_bindings,
            expected_federate_count,
            accepted,
        ) in cases
        {
            let result = ResolvedDeployment::new(
                topology(),
                [
                    binding("vehicle/controller", "controller-host", "controller.v1"),
                    binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
                ],
                placements,
                federates,
                coordination.clone(),
                boundary_bindings,
            );

            match (accepted, result) {
                (true, Ok(_)) => {}
                (
                    false,
                    Err(ResolveError::InvalidCoordination {
                        federate_count,
                        coordination: selected,
                    }),
                ) => {
                    assert_eq!(federate_count, expected_federate_count, "{name}");
                    assert_eq!(selected, coordination, "{name}");
                }
                (_, result) => panic!("unexpected resolution result for {name}: {result:?}"),
            }
        }
    }

    #[test]
    fn resolution_requires_exact_cross_federate_boundary_bindings() {
        let missing_binding = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [],
        )
        .unwrap_err();
        assert!(matches!(
            missing_binding,
            ResolveError::MissingBoundaryBinding { boundary }
                if boundary.to_string() == "controller-to-sensor"
        ));

        let resolved = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap();
        let selected_binding = resolved
            .boundary_binding(&BoundaryId::new("controller-to-sensor").unwrap())
            .expect("cross-Federate connection must retain its binding");
        assert_eq!(selected_binding.codec().to_string(), "serde-json");
        assert_eq!(selected_binding.transport().to_string(), "quic");

        let unexpected_binding = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "host"),
            ],
            [federate("host", "x86_64-unknown-linux-gnu", "native")],
            CoordinationSelection::Local,
            [controller_to_sensor_binding()],
        )
        .unwrap_err();
        assert!(matches!(
            unexpected_binding,
            ResolveError::UnexpectedBoundaryBinding { boundary }
                if boundary.to_string() == "controller-to-sensor"
        ));

        let unknown_binding = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [boundary_binding("unknown", "serde-json", "quic")],
        )
        .unwrap_err();
        assert!(matches!(
            unknown_binding,
            ResolveError::UnknownBoundaryBinding { boundary }
                if boundary.to_string() == "unknown"
        ));

        let duplicate_binding = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [
                controller_to_sensor_binding(),
                boundary_binding("controller-to-sensor", "postcard", "tcp"),
            ],
        )
        .unwrap_err();
        assert!(matches!(
            duplicate_binding,
            ResolveError::DuplicateBoundaryBinding { boundary }
                if boundary.to_string() == "controller-to-sensor"
        ));
    }

    #[test]
    fn resolution_is_canonical_under_selection_reordering() {
        let forward = resolve(false);
        let reverse = resolve(true);

        assert_eq!(forward, reverse);
        assert_eq!(
            forward
                .bindings()
                .map(|binding| binding.component().to_string())
                .collect::<Vec<_>>(),
            ["vehicle/controller", "vehicle/sensor"]
        );
        assert_eq!(
            forward
                .placements()
                .map(|assignment| (
                    assignment.placement_group().to_string(),
                    assignment.federate().as_str()
                ))
                .collect::<Vec<_>>(),
            [
                ("placement/controller".to_owned(), "host"),
                ("placement/sensor".to_owned(), "edge"),
            ]
        );
        assert_eq!(
            forward
                .federates()
                .map(|federate| federate.id().as_str())
                .collect::<Vec<_>>(),
            ["edge", "host"]
        );
        assert_eq!(
            forward
                .boundary_bindings()
                .map(|binding| binding.boundary().to_string())
                .collect::<Vec<_>>(),
            ["controller-to-sensor"]
        );
        assert_eq!(forward.coordination(), &distributed_coordination());
    }

    #[test]
    fn resolution_rejects_contract_mismatch_at_the_component_boundary() {
        let error = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "wrong.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ResolveError::ContractMismatch {
                component,
                required,
                required_version,
                provided,
                provided_version,
                implementation,
            }
                if component.to_string() == "vehicle/controller"
                    && required.as_str() == "controller.v1"
                    && required_version == 1
                    && provided.as_str() == "wrong.v1"
                    && provided_version == 1
                    && implementation.as_str() == "controller-host"
        ));
    }

    #[test]
    fn resolution_rejects_contract_version_mismatch_at_the_component_boundary() {
        let error = ResolvedDeployment::new(
            topology(),
            [
                ImplementationBinding::new(
                    ComponentInstanceId::new("vehicle/controller").unwrap(),
                    ImplementationId::new("controller-host").unwrap(),
                    descriptor_at_version("controller.v1", 2),
                ),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ResolveError::ContractMismatch {
                component,
                required,
                required_version,
                provided,
                provided_version,
                implementation,
            } if component.to_string() == "vehicle/controller"
                && required.as_str() == "controller.v1"
                && required_version == 1
                && provided.as_str() == "controller.v1"
                && provided_version == 2
                && implementation.as_str() == "controller-host"
        ));
    }

    #[test]
    fn resolution_requires_exact_component_and_placement_coverage() {
        let missing_binding = ResolvedDeployment::new(
            topology(),
            [binding(
                "vehicle/controller",
                "controller-host",
                "controller.v1",
            )],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();
        assert!(matches!(
            missing_binding,
            ResolveError::MissingComponentBinding { component }
                if component.to_string() == "vehicle/sensor"
        ));

        let missing_placement = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [placement("placement/controller", "host")],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();
        assert!(matches!(
            missing_placement,
            ResolveError::MissingPlacementAssignment { placement_group }
                if placement_group.to_string() == "placement/sensor"
        ));
    }

    #[test]
    fn resolution_rejects_duplicate_component_and_placement_selections() {
        let duplicate_binding = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/controller", "controller-sim", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();
        assert!(matches!(
            duplicate_binding,
            ResolveError::DuplicateComponentBinding { component }
                if component.to_string() == "vehicle/controller"
        ));

        let duplicate_placement = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/controller", "edge"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();
        assert!(matches!(
            duplicate_placement,
            ResolveError::DuplicatePlacementAssignment { placement_group }
                if placement_group.to_string() == "placement/controller"
        ));
    }

    #[test]
    fn resolution_rejects_duplicate_federate_configs() {
        let error = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            [
                federate("host", "x86_64-unknown-linux-gnu", "native"),
                federate("edge", "aarch64-unknown-linux-gnu", "rtic"),
                federate("edge", "aarch64-unknown-linux-musl", "rtic"),
            ],
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ResolveError::DuplicateFederateConfig { federate } if federate.as_str() == "edge"
        ));
    }

    #[test]
    fn resolution_rejects_duplicate_boundary_bindings() {
        let error = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [
                controller_to_sensor_binding(),
                boundary_binding("controller-to-sensor", "postcard", "tcp"),
            ],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ResolveError::DuplicateBoundaryBinding { boundary }
                if boundary.to_string() == "controller-to-sensor"
        ));
    }

    #[test]
    fn resolution_requires_a_federate_config_for_every_placement() {
        let error = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            [federate("host", "x86_64-unknown-linux-gnu", "native")],
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ResolveError::MissingFederateConfig {
                placement_group,
                federate,
            } if placement_group.to_string() == "placement/sensor" && federate.as_str() == "edge"
        ));
    }

    #[test]
    fn resolution_rejects_unused_federate_configs() {
        let error = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            [
                federate("host", "x86_64-unknown-linux-gnu", "native"),
                federate("edge", "aarch64-unknown-linux-gnu", "rtic"),
                federate("spare", "wasm32-wasip1", "sim"),
            ],
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ResolveError::UnusedFederateConfig { federate } if federate.as_str() == "spare"
        ));
    }
}
