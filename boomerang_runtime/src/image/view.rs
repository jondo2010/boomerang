//! Checked borrowed views over immutable compiled runtime images.
use super::*;
use tinymap::{IndexSpan, Key, SliceRange, TinyMapView};

/// A precise, allocation-free scheduler-image validation failure.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImageValidationError<'a> {
    /// A dense table exceeds its key domain.
    #[error("{table} exceeds its dense key domain")]
    TableTooLarge {
        /// Offending table.
        table: &'static str,
    },
    /// A dense reference is outside its target table.
    #[error("{table}[{index}].{field} references missing {target}[{referenced}]")]
    ReferenceOutOfBounds {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Source field.
        field: &'static str,
        /// Target table.
        target: &'static str,
        /// Invalid target index.
        referenced: u32,
    },
    /// A typed range exceeds its flattened target table.
    #[error("{table}[{index}].{field} range {start}+{len} exceeds {target}")]
    RangeOutOfBounds {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Source field.
        field: &'static str,
        /// Target flattened table.
        target: &'static str,
        /// Invalid range start.
        start: usize,
        /// Invalid range length.
        len: usize,
    },
    /// Canonical owner ranges overlap or move backwards.
    #[error("{table}[{index}].{field} starts at {start} before {previous_end}")]
    RangesNotMonotonic {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Source field.
        field: &'static str,
        /// Offending start.
        start: usize,
        /// End of the previous range.
        previous_end: usize,
    },
    /// Dependency entries are not in canonical order.
    #[error("{table}[{index}] is not sorted")]
    EntriesNotSorted {
        /// Flattened table.
        table: &'static str,
        /// Offending entry index.
        index: u32,
    },
    /// A dependency entry is repeated for one owner.
    #[error("{table}[{index}] duplicates its predecessor")]
    DuplicateEntry {
        /// Flattened table.
        table: &'static str,
        /// Offending entry index.
        index: u32,
    },
    /// A reactor, mode, or scope relationship disagrees.
    #[error("{table}[{index}].{field} has inconsistent ownership")]
    OwnershipMismatch {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Inconsistent relationship.
        field: &'static str,
    },
    /// A scope parent chain does not terminate at a root scope.
    #[error("scope parent cycle is reachable from scopes[{scope}]")]
    ScopeParentCycle {
        /// Dense scope whose parent chain contains a cycle.
        scope: u32,
    },
    /// A stable identity is empty, untrimmed, or contains controls.
    #[error("invalid {kind} identity at {index}: {id:?}")]
    InvalidStableId {
        /// Identity kind.
        kind: &'static str,
        /// Source record index.
        index: u32,
        /// Offending borrowed identity.
        id: &'a str,
    },
    /// Stable identities are not in lexical order.
    #[error("{kind} identity at {index} is not sorted: {id:?}")]
    StableIdsNotSorted {
        /// Identity kind.
        kind: &'static str,
        /// Source record index.
        index: u32,
        /// Offending borrowed identity.
        id: &'a str,
    },
    /// A stable identity duplicates its predecessor.
    #[error("duplicate {kind} identity at {index}: {id:?}")]
    DuplicateStableId {
        /// Identity kind.
        kind: &'static str,
        /// Source record index.
        index: u32,
        /// Offending borrowed identity.
        id: &'a str,
    },
    /// Reactor-bank metadata has an empty total or an out-of-range index.
    #[error("reactors[{reactor}] bank index {index} is outside total {total}")]
    InvalidBankInfo {
        /// Dense reactor index.
        reactor: u32,
        /// Invalid bank index.
        index: u32,
        /// Declared bank width.
        total: u32,
    },
    /// A dense storage slot reaches or exceeds its declared bound.
    #[error("{table}[{index}] {kind} slot {slot} exceeds bound {bound}")]
    StorageBoundExceeded {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Storage kind.
        kind: &'static str,
        /// Referenced slot.
        slot: u32,
        /// Exclusive bound.
        bound: u32,
    },
    /// A required implementation slot has the wrong binding kind.
    #[error("{table}[{index}].{field} has the wrong binding kind")]
    BindingKindMismatch {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Binding field.
        field: &'static str,
    },
    /// A binding is present for an executor-owned record or absent for a payload record.
    #[error("{table}[{index}].{field} has the wrong binding presence")]
    BindingPresenceMismatch {
        /// Source table.
        table: &'static str,
        /// Source record index.
        index: u32,
        /// Binding field.
        field: &'static str,
    },
    /// A scheduler-boundary route has no opposite-direction peer.
    #[error("boundary route '{boundary}' has no peer for its {direction:?} half")]
    UnpairedRoute {
        /// Stable boundary identity.
        boundary: &'a str,
        /// Direction of the existing route half.
        direction: RouteDirection,
    },
    /// A scheduler boundary has more than one route half in one direction.
    #[error("boundary route '{boundary}' has multiple {direction:?} halves")]
    DuplicateRouteHalf {
        /// Stable boundary identity.
        boundary: &'a str,
        /// Duplicated route direction.
        direction: RouteDirection,
    },
    /// Paired route halves disagree on scheduling semantics.
    #[error("boundary route '{boundary}' has mismatched {field}")]
    RoutePairMismatch {
        /// Stable boundary identity.
        boundary: &'a str,
        /// Scheduling field that differs between route halves.
        field: &'static str,
    },
}

/// A validated allocation-free view of one complete compiled deployment.
#[derive(Debug)]
pub struct CompiledDeploymentView<'a> {
    image: &'a CompiledDeploymentImage<'a>,
}

impl<'a> CompiledDeploymentView<'a> {
    /// Validates `image` and borrows its complete immutable hierarchy.
    pub fn new(image: &'a CompiledDeploymentImage<'a>) -> Result<Self, ImageValidationError<'a>> {
        validate_compiled_deployment(image)?;
        Ok(Self { image })
    }

    /// Returns the dense Federate table.
    pub const fn federates(&self) -> TinyMapView<'a, FederateIndex, FederateImage<'a>> {
        self.image.federates
    }

    /// Returns one validated Federate view.
    pub fn federate(&self, key: FederateIndex) -> FederateImageView<'a> {
        FederateImageView {
            image: self.image,
            federate: self.image.federates[key],
        }
    }

    /// Returns the backend-neutral federation structure.
    pub const fn federation(&self) -> GlobalFederationImage<'a> {
        self.image.federation
    }

    /// Returns the selected coordination projection.
    pub const fn coordination(&self) -> CoordinationProjection<'a> {
        self.image.coordination
    }
}

/// A validated borrowed view of one Federate and its Enclaves.
#[derive(Debug)]
pub struct FederateImageView<'a> {
    image: &'a CompiledDeploymentImage<'a>,
    federate: FederateImage<'a>,
}

impl<'a> FederateImageView<'a> {
    /// Returns the stable Federate identity.
    pub fn id(&self) -> FederateId<'a> {
        self.federate.id()
    }

    /// Returns the configured compilation target.
    pub fn target(&self) -> TargetId<'a> {
        self.federate.target()
    }

    /// Returns the configured runtime backend.
    pub fn runtime(&self) -> RuntimeBackendId<'a> {
        self.federate.runtime()
    }

    /// Returns the typed deployment-wide range of Enclaves owned by this Federate.
    pub const fn enclaves(&self) -> IndexSpan<EnclaveIndex> {
        self.federate.enclaves()
    }

    /// Iterates validated Enclave views in canonical identity order.
    pub fn enclave_views(&self) -> impl ExactSizeIterator<Item = EnclaveImageView<'a>> + 'a {
        let images = self
            .image
            .enclaves
            .get_span(self.federate.enclaves())
            .expect("compiled deployment ranges are validated");
        images.iter().map(EnclaveImageView::validated)
    }
}

/// A validated, allocation-free borrowed view of one Enclave image.
#[derive(Debug)]
pub struct EnclaveImageView<'a> {
    image: EnclaveImage<'a>,
}

impl<'a> EnclaveImageView<'a> {
    /// Validates `image` and borrows all of its tables without copying.
    pub fn new(image: &EnclaveImage<'a>) -> Result<Self, ImageValidationError<'a>> {
        validate(image)?;
        Ok(Self { image: *image })
    }

    fn validated(image: &EnclaveImage<'a>) -> Self {
        Self { image: *image }
    }
    /// Returns the stable Enclave identity.
    pub fn enclave_id(&self) -> EnclaveId<'a> {
        self.image.enclave_id
    }
    /// Returns the dense reactor table.
    pub const fn reactors(&self) -> TinyMapView<'a, ReactorIndex, ReactorImage> {
        self.image.reactors
    }
    /// Returns the dense action table.
    pub const fn actions(&self) -> TinyMapView<'a, ActionIndex, ActionImage> {
        self.image.actions
    }
    /// Returns the dense port table.
    pub const fn ports(&self) -> TinyMapView<'a, PortIndex, PortImage> {
        self.image.ports
    }
    /// Returns the dense reaction table.
    pub const fn reactions(&self) -> TinyMapView<'a, ReactionIndex, ReactionImage> {
        self.image.reactions
    }
    /// Returns the dense mode table.
    pub const fn modes(&self) -> TinyMapView<'a, ModeIndex, ModeImage> {
        self.image.modes
    }
    /// Returns the dense scope table.
    pub const fn scopes(&self) -> TinyMapView<'a, ScopeIndex, ScopeImage> {
        self.image.scopes
    }
    /// Returns the dense boundary-route table.
    pub const fn routes(&self) -> TinyMapView<'a, RouteIndex, RouteImage<'a>> {
        self.image.routes
    }
    /// Returns the dense required-binding table.
    pub const fn required_bindings(
        &self,
    ) -> TinyMapView<'a, BindingSlotIndex, RequiredBindingImage<'a>> {
        self.image.required_bindings
    }
    /// Resolves a route's stable boundary identity.
    pub fn route_boundary_id(&self, key: RouteIndex) -> BoundaryId<'a> {
        self.image.routes[key].boundary()
    }
    /// Resolves a required implementation binding's stable identity.
    pub fn required_binding_id(&self, key: BindingSlotIndex) -> BindingSlotId<'a> {
        self.image.required_bindings[key].id()
    }
    /// Returns the declared mutable-storage and workspace bounds.
    pub const fn storage_bounds(&self) -> StorageBounds {
        self.image.storage_bounds
    }
    /// Returns an action's ordered leveled triggers.
    pub fn action_triggers(&self, key: ActionIndex) -> &'a [LevelReactionImage] {
        self.image.actions[key]
            .triggers()
            .get(self.image.reaction_triggers)
            .expect("image table ranges are validated")
    }
    /// Returns a port's ordered leveled triggers.
    pub fn port_triggers(&self, key: PortIndex) -> &'a [LevelReactionImage] {
        self.image.ports[key]
            .triggers()
            .get(self.image.reaction_triggers)
            .expect("image table ranges are validated")
    }
    /// Returns a reaction's ordered use ports.
    pub fn reaction_use_ports(&self, key: ReactionIndex) -> &'a [PortIndex] {
        self.image.reactions[key]
            .use_ports()
            .get(self.image.reaction_use_ports)
            .expect("image table ranges are validated")
    }
    /// Returns a reaction's ordered effect ports.
    pub fn reaction_effect_ports(&self, key: ReactionIndex) -> &'a [PortIndex] {
        self.image.reactions[key]
            .effect_ports()
            .get(self.image.reaction_effect_ports)
            .expect("image table ranges are validated")
    }
    /// Returns a reaction's ordered action references.
    pub fn reaction_actions(&self, key: ReactionIndex) -> &'a [ActionIndex] {
        self.image.reactions[key]
            .actions()
            .get(self.image.reaction_actions)
            .expect("image table ranges are validated")
    }
    /// Returns a reaction's enabled modes.
    pub fn reaction_modes(&self, key: ReactionIndex) -> &'a [ModeIndex] {
        self.image.reactions[key]
            .enabled_modes()
            .get(self.image.reaction_modes)
            .expect("image table ranges are validated")
    }
    /// Returns a scope's precomputed descendants.
    pub fn scope_descendants(&self, key: ScopeIndex) -> &'a [ScopeIndex] {
        self.image.scopes[key]
            .descendants()
            .get(self.image.scope_descendants)
            .expect("image table ranges are validated")
    }
    /// Returns a scope's precomputed logical actions.
    pub fn scope_logical_actions(&self, key: ScopeIndex) -> &'a [ActionIndex] {
        self.image.scopes[key]
            .logical_actions()
            .get(self.image.scope_logical_actions)
            .expect("image table ranges are validated")
    }
    /// Returns a scope's precomputed timer startups.
    pub fn scope_timer_startups(&self, key: ScopeIndex) -> &'a [TimerStartupImage] {
        self.image.scopes[key]
            .timer_startups()
            .get(self.image.scope_timer_startups)
            .expect("image table ranges are validated")
    }
    /// Returns a scope's precomputed reset reactions.
    pub fn scope_reset_reactions(&self, key: ScopeIndex) -> &'a [LevelReactionImage] {
        self.image.scopes[key]
            .reset_reactions()
            .get(self.image.scope_reset_reactions)
            .expect("image table ranges are validated")
    }
    /// Returns a scope's precomputed startup reactions.
    pub fn scope_startup_reactions(&self, key: ScopeIndex) -> &'a [LifecycleReactionImage] {
        self.image.scopes[key]
            .startup_reactions()
            .get(self.image.scope_startup_reactions)
            .expect("image table ranges are validated")
    }
    /// Returns a scope's precomputed shutdown reactions.
    pub fn scope_shutdown_reactions(&self, key: ScopeIndex) -> &'a [LifecycleReactionImage] {
        self.image.scopes[key]
            .shutdown_reactions()
            .get(self.image.scope_shutdown_reactions)
            .expect("image table ranges are validated")
    }
    /// Returns global startup action entries.
    pub const fn startup_actions(&self) -> &'a [TimerStartupImage] {
        self.image.startup_actions
    }
    /// Returns global timer startup entries.
    pub const fn timer_startup_actions(&self) -> &'a [TimerStartupImage] {
        self.image.timer_startup_actions
    }
    /// Returns global shutdown reaction entries.
    pub const fn shutdown_reactions(&self) -> &'a [LifecycleReactionImage] {
        self.image.shutdown_reactions
    }
    /// Returns unique actions populated before global shutdown reactions.
    pub const fn shutdown_actions(&self) -> &'a [ActionIndex] {
        self.image.shutdown_actions
    }
}

fn check_len<K: Key>(table: &'static str, len: usize) -> Result<(), ImageValidationError<'static>> {
    if len > K::MAX_LEN {
        Err(ImageValidationError::TableTooLarge { table })
    } else {
        Ok(())
    }
}

fn check_ref<'a, K: Key, V>(
    table: &'static str,
    index: u32,
    field: &'static str,
    target: &'static str,
    value: K,
    values: TinyMapView<'_, K, V>,
) -> Result<(), ImageValidationError<'a>> {
    if values.get(value).is_none() {
        Err(ImageValidationError::ReferenceOutOfBounds {
            table,
            index,
            field,
            target,
            referenced: u32::try_from(value.index()).unwrap_or(u32::MAX),
        })
    } else {
        Ok(())
    }
}

fn check_range<'a, T>(
    table: &'static str,
    index: u32,
    field: &'static str,
    target: &'static str,
    range: SliceRange<T>,
    len: usize,
    previous_end: &mut usize,
) -> Result<(), ImageValidationError<'a>> {
    let end = range.checked_end();
    if end.map(|end| end <= len) != Some(true) {
        return Err(ImageValidationError::RangeOutOfBounds {
            table,
            index,
            field,
            target,
            start: range.start() as usize,
            len: range.len() as usize,
        });
    }
    if (range.start() as usize) < *previous_end {
        return Err(ImageValidationError::RangesNotMonotonic {
            table,
            index,
            field,
            start: range.start() as usize,
            previous_end: *previous_end,
        });
    }
    *previous_end = end.unwrap();
    Ok(())
}

fn check_span<'a, K: Key>(
    table: &'static str,
    index: u32,
    field: &'static str,
    target: &'static str,
    span: IndexSpan<K>,
    len: usize,
    previous_end: &mut usize,
) -> Result<(), ImageValidationError<'a>> {
    let end = span.checked_end();
    if end.is_none_or(|end| end > len) {
        return Err(ImageValidationError::RangeOutOfBounds {
            table,
            index,
            field,
            target,
            start: span.start(),
            len: span.len(),
        });
    }
    if span.start() < *previous_end {
        return Err(ImageValidationError::RangesNotMonotonic {
            table,
            index,
            field,
            start: span.start(),
            previous_end: *previous_end,
        });
    }
    *previous_end = end.expect("checked dense span has an end");
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

fn validate_id<'a>(
    kind: &'static str,
    index: u32,
    id: &'a str,
    previous: &mut Option<&'a str>,
) -> Result<(), ImageValidationError<'a>> {
    if !valid_id(id) {
        return Err(ImageValidationError::InvalidStableId { kind, index, id });
    }
    if let Some(before) = *previous {
        if id == before {
            return Err(ImageValidationError::DuplicateStableId { kind, index, id });
        }
        if id < before {
            return Err(ImageValidationError::StableIdsNotSorted { kind, index, id });
        }
    }
    *previous = Some(id);
    Ok(())
}

fn validate_rti_identity_table<'a, K: Key>(
    table: &'static str,
    kind: &'static str,
    values: IdentityTable<'a, K>,
) -> Result<(), ImageValidationError<'a>> {
    check_len::<K>(table, values.len())?;
    let mut previous = None;
    for (index, id) in values.values().copied().enumerate() {
        validate_id(kind, index as u32, id, &mut previous)?;
    }
    Ok(())
}

fn check_rti_range<'a, T>(
    index: u32,
    field: &'static str,
    target: &'static str,
    range: SliceRange<T>,
    len: usize,
    previous_end: &mut usize,
) -> Result<(), ImageValidationError<'a>> {
    if range.start() as usize != *previous_end {
        return Err(ImageValidationError::OwnershipMismatch {
            table: "coordination.rti.members",
            index,
            field,
        });
    }
    check_range(
        "coordination.rti.members",
        index,
        field,
        target,
        range,
        len,
        previous_end,
    )
}

fn validate_rti_federate_refs<'a>(
    table: &'static str,
    offset: u32,
    values: impl Iterator<Item = FederateIndex>,
    federates: TinyMapView<'_, FederateIndex, FederateImage<'_>>,
) -> Result<(), ImageValidationError<'a>> {
    let mut previous = None;
    for (position, value) in values.enumerate() {
        let index = offset + position as u32;
        check_ref(table, index, "federate", "federates", value, federates)?;
        if let Some(before) = previous {
            if value == before {
                return Err(ImageValidationError::DuplicateEntry { table, index });
            }
            if value < before {
                return Err(ImageValidationError::EntriesNotSorted { table, index });
            }
        }
        previous = Some(value);
    }
    Ok(())
}

fn validate_rti<'a>(
    image: &CompiledDeploymentImage<'a>,
    rti: RtiImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    if rti.members.len() != image.federates.len() {
        return Err(ImageValidationError::OwnershipMismatch {
            table: "coordination.rti",
            index: 0,
            field: "members",
        });
    }
    check_len::<FederateIndex>("coordination.rti.dependencies", rti.dependencies.len())?;
    check_len::<FederateIndex>(
        "coordination.rti.affected_downstream",
        rti.affected_downstream.len(),
    )?;
    macro_rules! identities { ($($table:literal, $kind:literal, $values:expr);+ $(;)?) => {
        $(validate_rti_identity_table($table, $kind, $values)?;)+
    }; }
    identities! {
        "coordination.rti.flows", "flow", rti.flows; "coordination.rti.physical_boundaries", "physical boundary", rti.physical_boundaries;
        "coordination.rti.transport_capabilities", "transport capability", rti.transport_capabilities; "coordination.rti.codec_capabilities", "codec capability", rti.codec_capabilities;
    }
    let mut dependency_end = 0;
    let mut downstream_end = 0;
    for (position, member) in rti.members.values().copied().enumerate() {
        let index = position as u32;
        for (field, range) in [
            ("direct_incoming", member.direct_incoming),
            ("transitive_incoming", member.transitive_incoming),
        ] {
            check_rti_range(
                index,
                field,
                "coordination.rti.dependencies",
                range,
                rti.dependencies.len(),
                &mut dependency_end,
            )?;
            let dependencies = range
                .get(rti.dependencies)
                .expect("checked RTI dependency range");
            validate_rti_federate_refs(
                "coordination.rti.dependencies",
                range.start(),
                dependencies.iter().map(|value| value.source()),
                image.federates,
            )?;
        }
        check_rti_range(
            index,
            "affected_downstream",
            "coordination.rti.affected_downstream",
            member.affected_downstream,
            rti.affected_downstream.len(),
            &mut downstream_end,
        )?;
        let downstream = member
            .affected_downstream
            .get(rti.affected_downstream)
            .expect("checked RTI downstream range");
        validate_rti_federate_refs(
            "coordination.rti.affected_downstream",
            member.affected_downstream.start(),
            downstream.iter().copied(),
            image.federates,
        )?;
    }
    for (field, end, len) in [
        ("dependencies", dependency_end, rti.dependencies.len()),
        (
            "affected_downstream",
            downstream_end,
            rti.affected_downstream.len(),
        ),
    ] {
        if end != len {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "coordination.rti",
                index: 0,
                field,
            });
        }
    }
    let mut previous_boundary = None;
    for (route_index, route) in rti.routes.iter() {
        let index = route_index.as_u32();
        let boundary = route.boundary;
        validate_id(
            "RTI boundary",
            index,
            boundary.as_str(),
            &mut previous_boundary,
        )?;
        check_ref(
            "coordination.rti.routes",
            index,
            "flow",
            "coordination.rti.flows",
            route.flow,
            rti.flows,
        )?;
        check_ref(
            "coordination.rti.routes",
            index,
            "transport_capability",
            "coordination.rti.transport_capabilities",
            route.transport_capability,
            rti.transport_capabilities,
        )?;
        check_ref(
            "coordination.rti.routes",
            index,
            "codec_capability",
            "coordination.rti.codec_capabilities",
            route.codec_capability,
            rti.codec_capabilities,
        )?;
        check_ref(
            "coordination.rti.routes",
            index,
            "source",
            "federates",
            route.source,
            image.federates,
        )?;
        check_ref(
            "coordination.rti.routes",
            index,
            "target",
            "federates",
            route.target,
            image.federates,
        )?;
        if let Some(value) = route.physical_input {
            check_ref(
                "coordination.rti.routes",
                index,
                "physical_input",
                "coordination.rti.physical_boundaries",
                value,
                rti.physical_boundaries,
            )?;
        }
        if let Some(value) = route.physical_output {
            check_ref(
                "coordination.rti.routes",
                index,
                "physical_output",
                "coordination.rti.physical_boundaries",
                value,
                rti.physical_boundaries,
            )?;
        }
    }
    if rti.routes.len() != image.federation.edges.len() {
        return Err(ImageValidationError::OwnershipMismatch {
            table: "coordination.rti",
            index: 0,
            field: "routes",
        });
    }
    for (position, (route, edge)) in rti.routes.values().zip(image.federation.edges).enumerate() {
        if route.boundary != edge.boundary()
            || route.source != edge.source()
            || route.target != edge.target()
            || route.delay_nanos() != edge.delay_nanos()
        {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "coordination.rti.routes",
                index: position as u32,
                field: "federation",
            });
        }
    }
    Ok(())
}

/// Validates deployment ownership, identities, federation edges, and nested images.
fn validate_compiled_deployment<'a>(
    image: &CompiledDeploymentImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    check_len::<FederateIndex>("federates", image.federates.len())?;
    check_len::<EnclaveIndex>("enclaves", image.enclaves.len())?;
    check_len::<FederateIndex>("federation.members", image.federation.members.len())?;

    let mut previous_federate = None;
    let mut enclave_end = 0;
    for (i, federate) in image.federates.values().copied().enumerate() {
        let index = i as u32;
        let id = federate.id();
        validate_id("federate", index, id.as_str(), &mut previous_federate)?;
        for (field, value) in [
            ("target", federate.target().as_str()),
            ("runtime", federate.runtime().as_str()),
        ] {
            if !valid_id(value) {
                return Err(ImageValidationError::InvalidStableId {
                    kind: field,
                    index,
                    id: value,
                });
            }
        }
        if federate.enclaves().start() != enclave_end {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "federates",
                index,
                field: "enclaves",
            });
        }
        check_span(
            "federates",
            index,
            "enclaves",
            "enclaves",
            federate.enclaves(),
            image.enclaves.len(),
            &mut enclave_end,
        )?;
    }
    if enclave_end != image.enclaves.len() {
        return Err(ImageValidationError::OwnershipMismatch {
            table: "image",
            index: 0,
            field: "enclaves",
        });
    }

    if image.federation.members.len() != image.federates.len() {
        return Err(ImageValidationError::OwnershipMismatch {
            table: "federation",
            index: 0,
            field: "members",
        });
    }
    for (i, member) in image.federation.members.iter().copied().enumerate() {
        check_ref(
            "federation.members",
            i as u32,
            "federate",
            "federates",
            member,
            image.federates,
        )?;
        if member.as_u32() != i as u32 {
            return Err(ImageValidationError::EntriesNotSorted {
                table: "federation.members",
                index: i as u32,
            });
        }
    }

    let mut previous_boundary = None;
    for (i, edge) in image.federation.edges.iter().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "federation.edges",
            index,
            "source",
            "federates",
            edge.source(),
            image.federates,
        )?;
        check_ref(
            "federation.edges",
            index,
            "target",
            "federates",
            edge.target(),
            image.federates,
        )?;
        let boundary = edge.boundary();
        validate_id(
            "federation boundary",
            index,
            boundary.as_str(),
            &mut previous_boundary,
        )?;
    }

    match image.coordination {
        CoordinationProjection::Local if image.federates.len() == 1 => {}
        CoordinationProjection::CentralRti(rti) => {
            validate_rti(image, rti)?;
            if image.federates.len() <= 1 {
                return Err(ImageValidationError::OwnershipMismatch {
                    table: "coordination",
                    index: 0,
                    field: "kind",
                });
            }
        }
        _ => {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "coordination",
                index: 0,
                field: "kind",
            })
        }
    }

    for federate in image.federates.values().copied() {
        let mut previous_enclave = None;
        let enclaves = image
            .enclaves
            .get_span(federate.enclaves())
            .expect("compiled deployment ranges are validated");
        for (offset, enclave) in enclaves.iter().enumerate() {
            let index = u32::try_from(federate.enclaves().start() + offset)
                .expect("validated Enclave key fits its u32 representation");
            validate(enclave)?;
            validate_id(
                "enclave",
                index,
                enclave.enclave_id.as_str(),
                &mut previous_enclave,
            )?;
        }
    }
    validate_route_pairs(image)?;
    Ok(())
}

/// Validates that every deployment boundary has one matching route half per direction.
fn validate_route_pairs<'a>(
    image: &CompiledDeploymentImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    for enclave in image.enclaves.values() {
        for route in enclave.routes.values().copied() {
            let boundary = route.boundary();
            let mut inbound = None;
            let mut outbound = None;
            let mut inbound_count = 0_usize;
            let mut outbound_count = 0_usize;
            for candidate_enclave in image.enclaves.values() {
                for candidate in candidate_enclave.routes.values().copied() {
                    let candidate_boundary = candidate.boundary();
                    if candidate_boundary != boundary {
                        continue;
                    }
                    match candidate.direction() {
                        RouteDirection::Inbound => {
                            inbound_count += 1;
                            inbound = Some(candidate);
                        }
                        RouteDirection::Outbound => {
                            outbound_count += 1;
                            outbound = Some(candidate);
                        }
                    }
                }
            }
            if inbound_count > 1 {
                return Err(ImageValidationError::DuplicateRouteHalf {
                    boundary: boundary.as_str(),
                    direction: RouteDirection::Inbound,
                });
            }
            if outbound_count > 1 {
                return Err(ImageValidationError::DuplicateRouteHalf {
                    boundary: boundary.as_str(),
                    direction: RouteDirection::Outbound,
                });
            }
            let (Some(inbound), Some(outbound)) = (inbound, outbound) else {
                return Err(ImageValidationError::UnpairedRoute {
                    boundary: boundary.as_str(),
                    direction: route.direction(),
                });
            };
            if inbound.timing_domain() != outbound.timing_domain() {
                return Err(ImageValidationError::RoutePairMismatch {
                    boundary: boundary.as_str(),
                    field: "timing_domain",
                });
            }
            if inbound.delay_nanos() != outbound.delay_nanos() {
                return Err(ImageValidationError::RoutePairMismatch {
                    boundary: boundary.as_str(),
                    field: "delay_nanos",
                });
            }
        }
    }
    Ok(())
}

fn validate_level_ref<'a>(
    table: &'static str,
    index: u32,
    entry: LevelReactionImage,
    image: &EnclaveImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    check_ref(
        table,
        index,
        "reaction",
        "reactions",
        entry.reaction(),
        image.reactions,
    )?;
    if image.reactions[entry.reaction()].dependency_level() != entry.level() {
        return Err(ImageValidationError::OwnershipMismatch {
            table,
            index,
            field: "dependency_level",
        });
    }
    Ok(())
}

fn validate_levels<'a>(
    table: &'static str,
    offset: u32,
    values: &[LevelReactionImage],
    image: &EnclaveImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    let mut previous = None;
    for (position, entry) in values.iter().copied().enumerate() {
        let index = offset + position as u32;
        validate_level_ref(table, index, entry, image)?;
        if let Some(before) = previous {
            if entry == before {
                return Err(ImageValidationError::DuplicateEntry { table, index });
            }
            if entry < before {
                return Err(ImageValidationError::EntriesNotSorted { table, index });
            }
        }
        previous = Some(entry);
    }
    Ok(())
}

fn validate_lifecycle<'a>(
    table: &'static str,
    offset: u32,
    values: &[LifecycleReactionImage],
    image: &EnclaveImage<'a>,
) -> Result<(), ImageValidationError<'a>> {
    let mut previous = None;
    for (position, entry) in values.iter().copied().enumerate() {
        let index = offset + position as u32;
        check_ref(
            table,
            index,
            "action",
            "actions",
            entry.action(),
            image.actions,
        )?;
        validate_level_ref(table, index, entry.reaction(), image)?;
        if let Some(before) = previous {
            if entry.reaction() == before {
                return Err(ImageValidationError::DuplicateEntry { table, index });
            }
            if entry.reaction() < before {
                return Err(ImageValidationError::EntriesNotSorted { table, index });
            }
        }
        previous = Some(entry.reaction());
    }
    Ok(())
}

fn validate<'a>(image: &EnclaveImage<'a>) -> Result<(), ImageValidationError<'a>> {
    check_len::<ReactorIndex>("reactors", image.reactors.len())?;
    check_len::<ActionIndex>("actions", image.actions.len())?;
    check_len::<PortIndex>("ports", image.ports.len())?;
    check_len::<ReactionIndex>("reactions", image.reactions.len())?;
    check_len::<ModeIndex>("modes", image.modes.len())?;
    check_len::<ScopeIndex>("scopes", image.scopes.len())?;
    check_len::<RouteIndex>("routes", image.routes.len())?;
    check_len::<BindingSlotIndex>("required_bindings", image.required_bindings.len())?;
    check_ref(
        "image",
        0,
        "root_reactor",
        "reactors",
        ReactorIndex::new(0),
        image.reactors,
    )?;
    validate_id("enclave", 0, image.enclave_id.as_str(), &mut None)?;

    let mut mode_end = 0;
    for (i, reactor) in image.reactors.values().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "reactors",
            index,
            "state_binding",
            "required_bindings",
            reactor.state_binding(),
            image.required_bindings,
        )?;
        if image.required_bindings[reactor.state_binding()].kind() != BindingKind::StateInitializer
        {
            return Err(ImageValidationError::BindingKindMismatch {
                table: "reactors",
                index,
                field: "state_binding",
            });
        }
        if reactor.state_slot().as_u32() >= image.storage_bounds.state_slots() {
            return Err(ImageValidationError::StorageBoundExceeded {
                table: "reactors",
                index,
                kind: "state slots",
                slot: reactor.state_slot().as_u32(),
                bound: image.storage_bounds.state_slots(),
            });
        }
        check_ref(
            "reactors",
            index,
            "root_scope",
            "scopes",
            reactor.root_scope(),
            image.scopes,
        )?;
        let root_scope = image.scopes[reactor.root_scope()];
        if root_scope.reactor() != ReactorIndex::new(index) || root_scope.mode().is_some() {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "reactors",
                index,
                field: "root_scope",
            });
        }
        check_span(
            "reactors",
            index,
            "modes",
            "modes",
            reactor.modes(),
            image.modes.len(),
            &mut mode_end,
        )?;
        if let Some(mode) = reactor.initial_mode() {
            check_ref(
                "reactors",
                index,
                "initial_mode",
                "modes",
                mode,
                image.modes,
            )?;
            if !reactor.modes().contains(mode) {
                return Err(ImageValidationError::OwnershipMismatch {
                    table: "reactors",
                    index,
                    field: "initial_mode",
                });
            }
        }
        if let Some(bank) = reactor.bank() {
            if bank.total() == 0 || bank.index() >= bank.total() {
                return Err(ImageValidationError::InvalidBankInfo {
                    reactor: index,
                    index: bank.index(),
                    total: bank.total(),
                });
            }
        }
    }

    let mut trigger_end = 0;
    for (i, action) in image.actions.values().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "actions",
            index,
            "scope",
            "scopes",
            action.scope(),
            image.scopes,
        )?;
        if action.storage_slot().as_u32() >= image.storage_bounds.action_slots() {
            return Err(ImageValidationError::StorageBoundExceeded {
                table: "actions",
                index,
                kind: "action slots",
                slot: action.storage_slot().as_u32(),
                bound: image.storage_bounds.action_slots(),
            });
        }
        match (action.timing(), action.binding()) {
            (ActionTiming::Standard { .. }, Some(binding)) => {
                check_ref(
                    "actions",
                    index,
                    "binding",
                    "required_bindings",
                    binding,
                    image.required_bindings,
                )?;
                if image.required_bindings[binding].kind() != BindingKind::Action {
                    return Err(ImageValidationError::BindingKindMismatch {
                        table: "actions",
                        index,
                        field: "binding",
                    });
                }
            }
            (ActionTiming::Standard { .. }, None)
            | (ActionTiming::Timer { .. } | ActionTiming::Shutdown, Some(_)) => {
                return Err(ImageValidationError::BindingPresenceMismatch {
                    table: "actions",
                    index,
                    field: "binding",
                });
            }
            (ActionTiming::Timer { .. } | ActionTiming::Shutdown, None) => {}
        }
        check_range(
            "actions",
            index,
            "triggers",
            "reaction_triggers",
            action.triggers(),
            image.reaction_triggers.len(),
            &mut trigger_end,
        )?;
    }
    trigger_end = 0;
    for (i, port) in image.ports.values().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "ports",
            index,
            "binding",
            "required_bindings",
            port.binding(),
            image.required_bindings,
        )?;
        if image.required_bindings[port.binding()].kind() != BindingKind::Port {
            return Err(ImageValidationError::BindingKindMismatch {
                table: "ports",
                index,
                field: "binding",
            });
        }
        check_ref(
            "ports",
            index,
            "scope",
            "scopes",
            port.scope(),
            image.scopes,
        )?;
        check_range(
            "ports",
            index,
            "triggers",
            "reaction_triggers",
            port.triggers(),
            image.reaction_triggers.len(),
            &mut trigger_end,
        )?;
    }

    let (mut use_end, mut effect_end, mut action_end, mut reaction_mode_end) = (0, 0, 0, 0);
    for (i, reaction) in image.reactions.values().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "reactions",
            index,
            "reactor",
            "reactors",
            reaction.reactor(),
            image.reactors,
        )?;
        check_ref(
            "reactions",
            index,
            "scope",
            "scopes",
            reaction.scope(),
            image.scopes,
        )?;
        check_ref(
            "reactions",
            index,
            "binding",
            "required_bindings",
            reaction.binding(),
            image.required_bindings,
        )?;
        if image.required_bindings[reaction.binding()].kind() != BindingKind::Reaction {
            return Err(ImageValidationError::BindingKindMismatch {
                table: "reactions",
                index,
                field: "binding",
            });
        }
        if image.scopes[reaction.scope()].reactor() != reaction.reactor() {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "reactions",
                index,
                field: "scope.reactor",
            });
        }
        if let Some(effect) = reaction.mode_effect() {
            check_ref(
                "reactions",
                index,
                "mode_effect.target",
                "modes",
                effect.target,
                image.modes,
            )?;
            if image.modes[effect.target].reactor() != reaction.reactor() {
                return Err(ImageValidationError::OwnershipMismatch {
                    table: "reactions",
                    index,
                    field: "mode_effect.target.reactor",
                });
            }
        }
        check_range(
            "reactions",
            index,
            "use_ports",
            "reaction_use_ports",
            reaction.use_ports(),
            image.reaction_use_ports.len(),
            &mut use_end,
        )?;
        check_range(
            "reactions",
            index,
            "effect_ports",
            "reaction_effect_ports",
            reaction.effect_ports(),
            image.reaction_effect_ports.len(),
            &mut effect_end,
        )?;
        check_range(
            "reactions",
            index,
            "actions",
            "reaction_actions",
            reaction.actions(),
            image.reaction_actions.len(),
            &mut action_end,
        )?;
        check_range(
            "reactions",
            index,
            "enabled_modes",
            "reaction_modes",
            reaction.enabled_modes(),
            image.reaction_modes.len(),
            &mut reaction_mode_end,
        )?;
    }

    for (i, mode) in image.modes.values().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "modes",
            index,
            "reactor",
            "reactors",
            mode.reactor(),
            image.reactors,
        )?;
        check_ref(
            "modes",
            index,
            "scope",
            "scopes",
            mode.scope(),
            image.scopes,
        )?;
        let scope = image.scopes[mode.scope()];
        if scope.reactor() != mode.reactor() {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "modes",
                index,
                field: "scope.reactor",
            });
        }
        if scope.mode() != Some(ModeIndex::new(index)) {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "modes",
                index,
                field: "scope.mode",
            });
        }
        let modes = image.reactors[mode.reactor()].modes();
        if !modes.contains(ModeIndex::new(index)) {
            return Err(ImageValidationError::OwnershipMismatch {
                table: "modes",
                index,
                field: "reactor.modes",
            });
        }
    }

    let mut ends = [0_usize; 6];
    for (i, scope) in image.scopes.values().copied().enumerate() {
        let index = i as u32;
        check_ref(
            "scopes",
            index,
            "reactor",
            "reactors",
            scope.reactor(),
            image.reactors,
        )?;
        if let Some(parent) = scope.parent() {
            check_ref("scopes", index, "parent", "scopes", parent, image.scopes)?;
        }
        if let Some(mode) = scope.mode() {
            check_ref("scopes", index, "mode", "modes", mode, image.modes)?;
            let owner = image.modes[mode];
            if owner.scope() != ScopeIndex::new(index) || owner.reactor() != scope.reactor() {
                return Err(ImageValidationError::OwnershipMismatch {
                    table: "scopes",
                    index,
                    field: "mode",
                });
            }
        }
        check_range(
            "scopes",
            index,
            "descendants",
            "scope_descendants",
            scope.descendants(),
            image.scope_descendants.len(),
            &mut ends[0],
        )?;
        check_range(
            "scopes",
            index,
            "logical_actions",
            "scope_logical_actions",
            scope.logical_actions(),
            image.scope_logical_actions.len(),
            &mut ends[1],
        )?;
        check_range(
            "scopes",
            index,
            "timer_startups",
            "scope_timer_startups",
            scope.timer_startups(),
            image.scope_timer_startups.len(),
            &mut ends[2],
        )?;
        check_range(
            "scopes",
            index,
            "reset_reactions",
            "scope_reset_reactions",
            scope.reset_reactions(),
            image.scope_reset_reactions.len(),
            &mut ends[3],
        )?;
        check_range(
            "scopes",
            index,
            "startup_reactions",
            "scope_startup_reactions",
            scope.startup_reactions(),
            image.scope_startup_reactions.len(),
            &mut ends[4],
        )?;
        check_range(
            "scopes",
            index,
            "shutdown_reactions",
            "scope_shutdown_reactions",
            scope.shutdown_reactions(),
            image.scope_shutdown_reactions.len(),
            &mut ends[5],
        )?;
    }
    for scope in image.scopes.keys() {
        let mut ancestor = Some(scope);
        for _ in 0..image.scopes.len() {
            ancestor = ancestor.and_then(|key| image.scopes[key].parent());
        }
        if ancestor.is_some() {
            return Err(ImageValidationError::ScopeParentCycle {
                scope: scope.as_u32(),
            });
        }
    }

    for action in image.actions.values().copied() {
        validate_levels(
            "reaction_triggers",
            action.triggers().start(),
            action
                .triggers()
                .get(image.reaction_triggers)
                .expect("image table ranges are validated"),
            image,
        )?;
    }
    for port in image.ports.values().copied() {
        validate_levels(
            "reaction_triggers",
            port.triggers().start(),
            port.triggers()
                .get(image.reaction_triggers)
                .expect("image table ranges are validated"),
            image,
        )?;
    }
    for (i, value) in image.reaction_use_ports.iter().enumerate() {
        check_ref(
            "reaction_use_ports",
            i as u32,
            "port",
            "ports",
            *value,
            image.ports,
        )?;
    }
    for (i, value) in image.reaction_effect_ports.iter().enumerate() {
        check_ref(
            "reaction_effect_ports",
            i as u32,
            "port",
            "ports",
            *value,
            image.ports,
        )?;
    }
    for (i, value) in image.reaction_actions.iter().enumerate() {
        check_ref(
            "reaction_actions",
            i as u32,
            "action",
            "actions",
            *value,
            image.actions,
        )?;
    }
    for (i, value) in image.reaction_modes.iter().enumerate() {
        check_ref(
            "reaction_modes",
            i as u32,
            "mode",
            "modes",
            *value,
            image.modes,
        )?;
    }
    for (i, reaction) in image.reactions.values().copied().enumerate() {
        for mode in reaction
            .enabled_modes()
            .get(image.reaction_modes)
            .expect("image table ranges are validated")
        {
            if image.modes[*mode].reactor() != reaction.reactor() {
                return Err(ImageValidationError::OwnershipMismatch {
                    table: "reactions",
                    index: i as u32,
                    field: "enabled_modes.reactor",
                });
            }
        }
    }
    for (i, value) in image.scope_descendants.iter().enumerate() {
        check_ref(
            "scope_descendants",
            i as u32,
            "scope",
            "scopes",
            *value,
            image.scopes,
        )?;
    }
    for (i, value) in image.scope_logical_actions.iter().enumerate() {
        check_ref(
            "scope_logical_actions",
            i as u32,
            "action",
            "actions",
            *value,
            image.actions,
        )?;
    }
    for (i, value) in image.scope_timer_startups.iter().enumerate() {
        check_ref(
            "scope_timer_startups",
            i as u32,
            "action",
            "actions",
            value.action(),
            image.actions,
        )?;
    }
    for scope in image.scopes.values().copied() {
        validate_levels(
            "scope_reset_reactions",
            scope.reset_reactions().start(),
            scope
                .reset_reactions()
                .get(image.scope_reset_reactions)
                .expect("image table ranges are validated"),
            image,
        )?;
        validate_lifecycle(
            "scope_startup_reactions",
            scope.startup_reactions().start(),
            scope
                .startup_reactions()
                .get(image.scope_startup_reactions)
                .expect("image table ranges are validated"),
            image,
        )?;
        validate_lifecycle(
            "scope_shutdown_reactions",
            scope.shutdown_reactions().start(),
            scope
                .shutdown_reactions()
                .get(image.scope_shutdown_reactions)
                .expect("image table ranges are validated"),
            image,
        )?;
    }
    for (i, entry) in image.reaction_triggers.iter().copied().enumerate() {
        validate_level_ref("reaction_triggers", i as u32, entry, image)?;
    }
    for (i, entry) in image.scope_reset_reactions.iter().copied().enumerate() {
        validate_level_ref("scope_reset_reactions", i as u32, entry, image)?;
    }
    for (table, entries) in [
        ("scope_startup_reactions", image.scope_startup_reactions),
        ("scope_shutdown_reactions", image.scope_shutdown_reactions),
    ] {
        for (i, entry) in entries.iter().copied().enumerate() {
            check_ref(
                table,
                i as u32,
                "action",
                "actions",
                entry.action(),
                image.actions,
            )?;
            validate_level_ref(table, i as u32, entry.reaction(), image)?;
        }
    }
    for (i, value) in image.startup_actions.iter().enumerate() {
        check_ref(
            "startup_actions",
            i as u32,
            "action",
            "actions",
            value.action(),
            image.actions,
        )?;
    }
    for (i, value) in image.timer_startup_actions.iter().enumerate() {
        check_ref(
            "timer_startup_actions",
            i as u32,
            "action",
            "actions",
            value.action(),
            image.actions,
        )?;
    }
    validate_lifecycle("shutdown_reactions", 0, image.shutdown_reactions, image)?;
    let mut previous_action = None;
    for (i, action) in image.shutdown_actions.iter().copied().enumerate() {
        check_ref(
            "shutdown_actions",
            i as u32,
            "action",
            "actions",
            action,
            image.actions,
        )?;
        if let Some(previous) = previous_action {
            if action == previous {
                return Err(ImageValidationError::DuplicateEntry {
                    table: "shutdown_actions",
                    index: i as u32,
                });
            }
            if action < previous {
                return Err(ImageValidationError::EntriesNotSorted {
                    table: "shutdown_actions",
                    index: i as u32,
                });
            }
        }
        previous_action = Some(action);
    }

    let mut previous_route = None;
    for (i, route) in image.routes.values().copied().enumerate() {
        let id = route.boundary();
        if !valid_id(id.as_str()) {
            return Err(ImageValidationError::InvalidStableId {
                kind: "boundary",
                index: i as u32,
                id: id.as_str(),
            });
        }
        if let Some((previous_id, previous_direction)) = previous_route {
            match id.cmp(&previous_id) {
                std::cmp::Ordering::Less => {
                    return Err(ImageValidationError::StableIdsNotSorted {
                        kind: "boundary",
                        index: i as u32,
                        id: id.as_str(),
                    })
                }
                std::cmp::Ordering::Equal if route.direction() == previous_direction => {
                    return Err(ImageValidationError::DuplicateRouteHalf {
                        boundary: id.as_str(),
                        direction: route.direction(),
                    })
                }
                std::cmp::Ordering::Equal if previous_direction == RouteDirection::Outbound => {
                    return Err(ImageValidationError::EntriesNotSorted {
                        table: "routes",
                        index: i as u32,
                    })
                }
                _ => {}
            }
        }
        previous_route = Some((id, route.direction()));
        check_ref(
            "routes",
            i as u32,
            "local_port",
            "ports",
            route.local_port(),
            image.ports,
        )?;
    }
    let mut previous = None;
    for (i, binding) in image.required_bindings.values().copied().enumerate() {
        let id = binding.id();
        validate_id("binding", i as u32, id.as_str(), &mut previous)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use tinymap::{IndexSpan, SliceRange};

    const fn federate_image(
        id: &'static str,
        target: &'static str,
        runtime: &'static str,
        enclaves: IndexSpan<EnclaveIndex>,
    ) -> FederateImage<'static> {
        FederateImage::new(
            FederateId::new(id),
            TargetId::new(target),
            RuntimeBackendId::new(runtime),
            enclaves,
        )
    }

    const fn route_image(
        boundary: &'static str,
        local_port: PortIndex,
        direction: RouteDirection,
        timing_domain: TimingDomain,
        delay_nanos: u64,
    ) -> RouteImage<'static> {
        RouteImage::new(
            BoundaryId::new(boundary),
            local_port,
            direction,
            timing_domain,
            delay_nanos,
        )
    }

    const fn binding_image(id: &'static str, kind: BindingKind) -> RequiredBindingImage<'static> {
        RequiredBindingImage::new(BindingSlotId::new(id), kind)
    }

    #[test]
    fn slice_ranges_address_their_packed_backing_slice() {
        let values = [10, 20, 30];

        assert_eq!(SliceRange::new(1, 2).get(&values), Some(&values[1..3]));
    }

    #[test]
    fn image_rows_borrow_utf8_identities_directly() {
        let federate = FederateImage::new(
            FederateId::new("fédérate"),
            TargetId::new("aarch64-unknown-none"),
            RuntimeBackendId::new("static"),
            IndexSpan::new(0, 0),
        );
        let boundary = BoundaryId::new("network/in");
        let route = RouteImage::new(
            boundary,
            PortIndex::new(0),
            RouteDirection::Inbound,
            TimingDomain::Logical,
            0,
        );
        let edge =
            FederationEdgeImage::new(boundary, FederateIndex::new(0), FederateIndex::new(1), 0);
        let binding =
            RequiredBindingImage::new(BindingSlotId::new("reaction/main"), BindingKind::Reaction);

        assert_eq!(federate.id().as_str(), "fédérate");
        assert_eq!(federate.target().as_str(), "aarch64-unknown-none");
        assert_eq!(federate.runtime().as_str(), "static");
        assert_eq!(route.boundary(), boundary);
        assert_eq!(edge.boundary(), boundary);
        assert_eq!(binding.id().as_str(), "reaction/main");
    }

    const RANGE_0_0: SliceRange<PortIndex> = SliceRange::new(0, 0);

    static REACTORS: [ReactorImage; 2] = [
        ReactorImage::new(
            BindingSlotIndex::new(2),
            StateSlotIndex::new(0),
            ScopeIndex::new(0),
            IndexSpan::new(0, 1),
            Some(ModeIndex::new(0)),
            Some(BankInfoImage::new(0, 2)),
        ),
        ReactorImage::new(
            BindingSlotIndex::new(3),
            StateSlotIndex::new(1),
            ScopeIndex::new(2),
            IndexSpan::new(1, 0),
            None,
            Some(BankInfoImage::new(1, 2)),
        ),
    ];
    static ACTIONS: [ActionImage; 1] = [ActionImage::new(
        ScopeIndex::new(1),
        ActionSlotIndex::new(0),
        ActionTiming::Standard {
            domain: TimingDomain::Logical,
            min_delay_nanos: 7,
        },
        SliceRange::new(0, 2),
        Some(BindingSlotIndex::new(5)),
    )];
    static PORTS: [PortImage; 2] = [
        PortImage::new(
            ScopeIndex::new(0),
            SliceRange::new(2, 1),
            BindingSlotIndex::new(4),
        ),
        PortImage::new(
            ScopeIndex::new(2),
            SliceRange::new(3, 1),
            BindingSlotIndex::new(4),
        ),
    ];
    static REACTIONS: [ReactionImage; 2] = [
        ReactionImage::new(
            ReactorIndex::new(0),
            ScopeIndex::new(1),
            0,
            BindingSlotIndex::new(0),
            SliceRange::new(0, 1),
            SliceRange::new(0, 1),
            SliceRange::new(0, 1),
            SliceRange::new(0, 1),
        ),
        ReactionImage::new(
            ReactorIndex::new(1),
            ScopeIndex::new(2),
            1,
            BindingSlotIndex::new(1),
            SliceRange::new(1, 1),
            SliceRange::new(1, 0),
            SliceRange::new(1, 0),
            SliceRange::new(1, 0),
        ),
    ];
    static MODES: [ModeImage; 1] = [ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(1))];
    static SCOPES: [ScopeImage; 3] = [
        ScopeImage::new(
            None,
            ReactorIndex::new(0),
            None,
            SliceRange::new(0, 2),
            SliceRange::new(0, 1),
            SliceRange::new(0, 1),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        ),
        ScopeImage::new(
            Some(ScopeIndex::new(0)),
            ReactorIndex::new(0),
            Some(ModeIndex::new(0)),
            SliceRange::new(2, 1),
            SliceRange::new(1, 1),
            SliceRange::new(1, 1),
            SliceRange::new(0, 1),
            SliceRange::new(0, 1),
            SliceRange::new(0, 1),
        ),
        ScopeImage::new(
            None,
            ReactorIndex::new(1),
            None,
            SliceRange::new(3, 1),
            SliceRange::new(2, 0),
            SliceRange::new(2, 0),
            SliceRange::new(1, 0),
            SliceRange::new(1, 0),
            SliceRange::new(1, 0),
        ),
    ];
    static REACTION_TRIGGERS: [LevelReactionImage; 4] = [
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        LevelReactionImage::new(1, ReactionIndex::new(1)),
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        LevelReactionImage::new(1, ReactionIndex::new(1)),
    ];
    static REACTION_USE_PORTS: [PortIndex; 2] = [PortIndex::new(0), PortIndex::new(1)];
    static REACTION_EFFECT_PORTS: [PortIndex; 1] = [PortIndex::new(1)];
    static REACTION_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
    static REACTION_MODES: [ModeIndex; 1] = [ModeIndex::new(0)];
    static SCOPE_DESCENDANTS: [ScopeIndex; 4] = [
        ScopeIndex::new(0),
        ScopeIndex::new(1),
        ScopeIndex::new(1),
        ScopeIndex::new(2),
    ];
    static SCOPE_LOGICAL_ACTIONS: [ActionIndex; 2] = [ActionIndex::new(0), ActionIndex::new(0)];
    static SCOPE_TIMER_STARTUPS: [TimerStartupImage; 2] = [
        TimerStartupImage::new(ActionIndex::new(0), 5),
        TimerStartupImage::new(ActionIndex::new(0), 5),
    ];
    static SCOPE_RESET_REACTIONS: [LevelReactionImage; 1] =
        [LevelReactionImage::new(0, ReactionIndex::new(0))];
    static SCOPE_STARTUP_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        ActionIndex::new(0),
    )];
    static SCOPE_SHUTDOWN_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        ActionIndex::new(0),
    )];
    static STARTUP_ACTIONS: [TimerStartupImage; 1] =
        [TimerStartupImage::new(ActionIndex::new(0), 0)];
    static TIMER_STARTUP_ACTIONS: [TimerStartupImage; 1] =
        [TimerStartupImage::new(ActionIndex::new(0), 5)];
    static SHUTDOWN_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
        LevelReactionImage::new(1, ReactionIndex::new(1)),
        ActionIndex::new(0),
    )];
    static SHUTDOWN_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
    static ROUTES: [RouteImage; 1] = [route_image(
        "network/in",
        PortIndex::new(1),
        RouteDirection::Inbound,
        TimingDomain::Physical,
        10,
    )];
    static OUTBOUND_ROUTES: [RouteImage; 1] = [route_image(
        "network/in",
        PortIndex::new(1),
        RouteDirection::Outbound,
        TimingDomain::Physical,
        10,
    )];
    static EMPTY_ROUTES: [RouteImage; 0] = [];
    static REQUIRED_BINDINGS: [RequiredBindingImage; 6] = [
        binding_image("reaction/r0", BindingKind::Reaction),
        binding_image("reaction/r1", BindingKind::Reaction),
        binding_image("state/r0", BindingKind::StateInitializer),
        binding_image("state/r1", BindingKind::StateInitializer),
        binding_image("twork", BindingKind::Port),
        binding_image("work", BindingKind::Action),
    ];

    static IMAGE: EnclaveImage<'static> = EnclaveImage {
        enclave_id: EnclaveId::new("plant/control"),
        reactors: TinyMapView::new(&REACTORS),
        actions: TinyMapView::new(&ACTIONS),
        ports: TinyMapView::new(&PORTS),
        reactions: TinyMapView::new(&REACTIONS),
        modes: TinyMapView::new(&MODES),
        scopes: TinyMapView::new(&SCOPES),
        reaction_triggers: &REACTION_TRIGGERS,
        reaction_use_ports: &REACTION_USE_PORTS,
        reaction_effect_ports: &REACTION_EFFECT_PORTS,
        reaction_actions: &REACTION_ACTIONS,
        reaction_modes: &REACTION_MODES,
        scope_descendants: &SCOPE_DESCENDANTS,
        scope_logical_actions: &SCOPE_LOGICAL_ACTIONS,
        scope_timer_startups: &SCOPE_TIMER_STARTUPS,
        scope_reset_reactions: &SCOPE_RESET_REACTIONS,
        scope_startup_reactions: &SCOPE_STARTUP_REACTIONS,
        scope_shutdown_reactions: &SCOPE_SHUTDOWN_REACTIONS,
        startup_actions: &STARTUP_ACTIONS,
        timer_startup_actions: &TIMER_STARTUP_ACTIONS,
        shutdown_reactions: &SHUTDOWN_REACTIONS,
        shutdown_actions: &SHUTDOWN_ACTIONS,
        routes: TinyMapView::new(&ROUTES),
        required_bindings: TinyMapView::new(&REQUIRED_BINDINGS),
        storage_bounds: StorageBounds::new(2, 1, 8, 0, 0, 4),
    };

    static SECOND_IMAGE: EnclaveImage<'static> = EnclaveImage {
        enclave_id: EnclaveId::new("plant/otherx"),
        routes: TinyMapView::new(&OUTBOUND_ROUTES),
        ..IMAGE
    };
    static FEDERATES: [FederateImage; 1] = [federate_image(
        "host",
        "aarch64-unknown-linux-gnu",
        "hosted",
        IndexSpan::new(0, 2),
    )];
    static ENCLAVES: [EnclaveImage<'static>; 2] = [IMAGE, SECOND_IMAGE];
    static FEDERATION_MEMBERS: [FederateIndex; 1] = [FederateIndex::new(0)];
    static FEDERATION: GlobalFederationImage<'static> =
        GlobalFederationImage::new(&FEDERATION_MEMBERS, &[]);
    static COMPILED: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
        federation: FEDERATION,
        federates: TinyMapView::new(&FEDERATES),
        enclaves: TinyMapView::new(&ENCLAVES),
        coordination: CoordinationProjection::Local,
    };
    static RTI_IDENTITIES: [&str; 1] = ["x"];

    fn rti_fixture<'a>(
        members: &'a [RtiMemberImage],
        dependencies: &'a [RtiDependencyImage],
        routes: &'a [RtiRouteImage<'a>],
        flows: &'a [&'a str],
    ) -> RtiImage<'a> {
        RtiImage::new(
            TinyMapView::new(members),
            dependencies,
            &[],
            TinyMapView::new(routes),
            IdentityTable::new(flows),
            IdentityTable::new(&[]),
            IdentityTable::new(&RTI_IDENTITIES),
            IdentityTable::new(&RTI_IDENTITIES),
        )
    }

    #[test]
    fn static_scheduler_image_exposes_borrowed_typed_tables() {
        assert_eq!(
            IMAGE.reactors[ReactorIndex::new(0)].root_scope(),
            ScopeIndex::new(0)
        );
        assert_eq!(
            COMPILED.federates[FederateIndex::new(0)].enclaves(),
            IndexSpan::new(0, 2)
        );

        let view = EnclaveImageView::new(&IMAGE).unwrap();

        assert_eq!(view.enclave_id().as_str(), "plant/control");
        assert_eq!(view.reactors().len(), 2);
        assert_eq!(
            view.reactors()[ReactorIndex::new(1)].bank(),
            Some(BankInfoImage::new(1, 2))
        );
        assert_eq!(view.actions().len(), 1);
        assert_eq!(
            view.actions()[ActionIndex::new(0)].binding(),
            Some(BindingSlotIndex::new(5))
        );
        assert_eq!(
            view.ports()[PortIndex::new(0)].binding(),
            BindingSlotIndex::new(4)
        );
        assert_eq!(
            view.actions()[ActionIndex::new(0)].timing(),
            ActionTiming::Standard {
                domain: TimingDomain::Logical,
                min_delay_nanos: 7,
            }
        );
        assert_eq!(view.ports().len(), 2);
        assert_eq!(
            view.reactions()[ReactionIndex::new(1)].dependency_level(),
            1
        );
        assert_eq!(
            view.reactions()[ReactionIndex::new(1)].binding(),
            BindingSlotIndex::new(1)
        );
        assert_eq!(
            view.action_triggers(ActionIndex::new(0)),
            &[
                LevelReactionImage::new(0, ReactionIndex::new(0)),
                LevelReactionImage::new(1, ReactionIndex::new(1)),
            ]
        );
        assert_eq!(
            view.route_boundary_id(RouteIndex::new(0)).as_str(),
            "network/in"
        );
        assert_eq!(
            view.routes()[RouteIndex::new(0)].timing_domain(),
            TimingDomain::Physical
        );
        assert_eq!(view.shutdown_actions(), &SHUTDOWN_ACTIONS);
        assert_eq!(
            view.required_binding_id(BindingSlotIndex::new(2)).as_str(),
            "state/r0"
        );
        assert_eq!(view.storage_bounds().event_capacity(), 8);
    }

    #[test]
    fn storage_bounds_preserve_slots_queues_and_byte_limits() {
        let bounds = StorageBounds::new(2, 4, 8, 16, 32, 64);

        assert_eq!(bounds.state_slots(), 2);
        assert_eq!(bounds.action_slots(), 4);
        assert_eq!(bounds.event_capacity(), 8);
        assert_eq!(bounds.payload_bytes(), 16);
        assert_eq!(bounds.state_bytes(), 32);
        assert_eq!(bounds.scratch_bytes(), 64);
    }

    #[test]
    fn compiled_view_resolves_static_federate_and_enclave_ranges() {
        let view = CompiledDeploymentView::new(&COMPILED).unwrap();

        assert_eq!(view.federates().len(), 1);
        let federate = view.federate(FederateIndex::new(0));
        assert_eq!(federate.id().as_str(), "host");
        assert_eq!(federate.enclaves(), IndexSpan::new(0, 2));
        assert_eq!(federate.enclave_views().count(), 2);
    }

    #[test]
    fn compiled_view_enforces_coordination_cardinality() {
        let member = RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        );
        let one = [member];
        let image = CompiledDeploymentImage {
            coordination: CoordinationProjection::CentralRti(rti_fixture(&one, &[], &[], &[])),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&image),
            Err(ImageValidationError::OwnershipMismatch {
                table: "coordination",
                index: 0,
                field: "kind"
            })
        ));

        let federates = [
            FEDERATES[0],
            federate_image(
                "z",
                FEDERATES[0].target().as_str(),
                FEDERATES[0].runtime().as_str(),
                IndexSpan::new(2, 0),
            ),
        ];
        let federation_members = [FederateIndex::new(0), FederateIndex::new(1)];
        let image = CompiledDeploymentImage {
            federation: GlobalFederationImage::new(&federation_members, &[]),
            federates: TinyMapView::new(&federates),
            coordination: CoordinationProjection::Local,
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&image),
            Err(ImageValidationError::OwnershipMismatch {
                table: "coordination",
                index: 0,
                field: "kind"
            })
        ));
    }

    #[test]
    fn compiled_view_requires_rti_routes_to_match_the_authoritative_federation() {
        let members = [RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )];
        let edges = [FederationEdgeImage::new(
            BoundaryId::new("host"),
            FederateIndex::new(0),
            FederateIndex::new(0),
            0,
        )];
        let routes = [RtiRouteImage::new(
            BoundaryId::new("x"),
            FlowIndex::new(0),
            None,
            None,
            BoundaryFailurePolicy::PropagateStop,
            TransportPolicy::ReliableOrderedFramed,
            CodecPolicy::CanonicalBounded,
            TimingPolicy::BestEffort,
            SecurityPolicy::None,
            TransportCapabilityIndex::new(0),
            CodecCapabilityIndex::new(0),
            FederateIndex::new(0),
            FederateIndex::new(0),
            0,
        )];
        let image = CompiledDeploymentImage {
            federation: GlobalFederationImage::new(&FEDERATION_MEMBERS, &edges),
            coordination: CoordinationProjection::CentralRti(rti_fixture(
                &members,
                &[],
                &routes,
                &RTI_IDENTITIES,
            )),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&image),
            Err(ImageValidationError::OwnershipMismatch {
                table: "coordination.rti.routes",
                index: 0,
                field: "federation"
            })
        ));
    }

    #[test]
    fn compiled_view_rejects_invalid_central_rti_dense_references() {
        let flows = [];
        let member = RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        );
        let members = [member];
        let routes = [RtiRouteImage::new(
            BoundaryId::new("federation-route"),
            FlowIndex::new(0),
            None,
            None,
            BoundaryFailurePolicy::PropagateStop,
            TransportPolicy::ReliableOrderedFramed,
            CodecPolicy::CanonicalBounded,
            TimingPolicy::BestEffort,
            SecurityPolicy::None,
            TransportCapabilityIndex::new(0),
            CodecCapabilityIndex::new(0),
            FederateIndex::new(0),
            FederateIndex::new(0),
            0,
        )];
        let image = CompiledDeploymentImage {
            coordination: CoordinationProjection::CentralRti(rti_fixture(
                &members,
                &[],
                &routes,
                &flows,
            )),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&image),
            Err(ImageValidationError::ReferenceOutOfBounds {
                table: "coordination.rti.routes",
                index: 0,
                field: "flow",
                target: "coordination.rti.flows",
                referenced: 0
            })
        ));

        let ranged_members = [RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 1),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )];
        let image = CompiledDeploymentImage {
            coordination: CoordinationProjection::CentralRti(rti_fixture(
                &ranged_members,
                &[],
                &[],
                &flows,
            )),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&image),
            Err(ImageValidationError::RangeOutOfBounds {
                table: "coordination.rti.members",
                index: 0,
                field: "direct_incoming",
                target: "coordination.rti.dependencies",
                start: 0,
                len: 1
            })
        ));

        let dependencies = [RtiDependencyImage::new(FederateIndex::new(0), 0)];
        let image = CompiledDeploymentImage {
            coordination: CoordinationProjection::CentralRti(rti_fixture(
                &members,
                &dependencies,
                &[],
                &flows,
            )),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&image),
            Err(ImageValidationError::OwnershipMismatch {
                table: "coordination.rti",
                index: 0,
                field: "dependencies"
            })
        ));
    }

    #[test]
    fn compiled_view_requires_matching_deployment_wide_route_halves() {
        let missing_enclaves = [
            IMAGE,
            EnclaveImage {
                routes: TinyMapView::new(&EMPTY_ROUTES),
                ..SECOND_IMAGE
            },
        ];
        let missing_image = CompiledDeploymentImage {
            enclaves: TinyMapView::new(&missing_enclaves),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&missing_image).unwrap_err(),
            ImageValidationError::UnpairedRoute {
                boundary: "network/in",
                direction: RouteDirection::Inbound,
            }
        ));

        let wrong_domain = [route_image(
            "network/in",
            PortIndex::new(1),
            RouteDirection::Outbound,
            TimingDomain::Logical,
            10,
        )];
        let mismatched_enclaves = [
            IMAGE,
            EnclaveImage {
                routes: TinyMapView::new(&wrong_domain),
                ..SECOND_IMAGE
            },
        ];
        let mismatched_image = CompiledDeploymentImage {
            enclaves: TinyMapView::new(&mismatched_enclaves),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&mismatched_image).unwrap_err(),
            ImageValidationError::RoutePairMismatch {
                boundary: "network/in",
                field: "timing_domain",
            }
        ));

        let wrong_delay = [route_image(
            "network/in",
            PortIndex::new(1),
            RouteDirection::Outbound,
            TimingDomain::Physical,
            11,
        )];
        let delayed_enclaves = [
            IMAGE,
            EnclaveImage {
                routes: TinyMapView::new(&wrong_delay),
                ..SECOND_IMAGE
            },
        ];
        let delayed_image = CompiledDeploymentImage {
            enclaves: TinyMapView::new(&delayed_enclaves),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&delayed_image).unwrap_err(),
            ImageValidationError::RoutePairMismatch {
                boundary: "network/in",
                field: "delay_nanos",
            }
        ));

        let duplicate_federates = [federate_image(
            "host",
            "aarch64-unknown-linux-gnu",
            "hosted",
            IndexSpan::new(0, 3),
        )];
        let duplicate_enclaves = [
            EnclaveImage {
                enclave_id: EnclaveId::new("plant/another"),
                ..IMAGE
            },
            IMAGE,
            SECOND_IMAGE,
        ];
        let duplicate_image = CompiledDeploymentImage {
            federates: TinyMapView::new(&duplicate_federates),
            enclaves: TinyMapView::new(&duplicate_enclaves),
            ..COMPILED
        };
        assert!(matches!(
            CompiledDeploymentView::new(&duplicate_image).unwrap_err(),
            ImageValidationError::DuplicateRouteHalf {
                boundary: "network/in",
                direction: RouteDirection::Inbound,
            }
        ));
    }

    #[test]
    fn compiled_view_orders_enclaves_within_each_federate() {
        let federates = [
            federate_image("alpha", "target", "runtime", IndexSpan::new(0, 2)),
            federate_image("beta", "target", "runtime", IndexSpan::new(2, 2)),
        ];
        let enclaves = [
            EnclaveImage {
                enclave_id: EnclaveId::new("zzzza/control"),
                routes: TinyMapView::new(&EMPTY_ROUTES),
                ..IMAGE
            },
            EnclaveImage {
                enclave_id: EnclaveId::new("zzzzb/control"),
                routes: TinyMapView::new(&EMPTY_ROUTES),
                ..IMAGE
            },
            EnclaveImage {
                enclave_id: EnclaveId::new("aaaaa/control"),
                routes: TinyMapView::new(&EMPTY_ROUTES),
                ..IMAGE
            },
            EnclaveImage {
                enclave_id: EnclaveId::new("aaaab/control"),
                routes: TinyMapView::new(&EMPTY_ROUTES),
                ..IMAGE
            },
        ];
        let members = [FederateIndex::new(0), FederateIndex::new(1)];
        let rti_members = [RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        ); 2];
        let image = CompiledDeploymentImage {
            federation: GlobalFederationImage::new(&members, &[]),
            federates: TinyMapView::new(&federates),
            enclaves: TinyMapView::new(&enclaves),
            coordination: CoordinationProjection::CentralRti(rti_fixture(
                &rti_members,
                &[],
                &[],
                &[],
            )),
        };

        let view = CompiledDeploymentView::new(&image).unwrap();
        let second_ids = view
            .federate(FederateIndex::new(1))
            .enclave_views()
            .map(|enclave| enclave.enclave_id().as_str())
            .collect::<Vec<_>>();
        assert_eq!(second_ids, ["aaaaa/control", "aaaab/control"]);
    }

    #[test]
    fn compiled_view_rejects_a_federate_enclave_range_outside_the_root_table() {
        let federates = [federate_image(
            "host",
            "aarch64-unknown-linux-gnu",
            "hosted",
            IndexSpan::new(0, 3),
        )];
        let image = CompiledDeploymentImage {
            federates: TinyMapView::new(&federates),
            ..COMPILED
        };

        assert!(matches!(
            CompiledDeploymentView::new(&image),
            Err(ImageValidationError::RangeOutOfBounds {
                table: "federates",
                index: 0,
                field: "enclaves",
                ..
            })
        ));
    }

    #[test]
    fn action_timing_preserves_standard_timer_and_shutdown_semantics() {
        let cases = [
            ActionTiming::Standard {
                domain: TimingDomain::Physical,
                min_delay_nanos: 19,
            },
            ActionTiming::Timer {
                period_nanos: Some(23),
            },
            ActionTiming::Shutdown,
        ];

        for timing in cases {
            let actions = [ActionImage::new(
                ScopeIndex::new(1),
                ActionSlotIndex::new(0),
                timing,
                SliceRange::new(0, 2),
                matches!(timing, ActionTiming::Standard { .. }).then_some(BindingSlotIndex::new(5)),
            )];
            let image = EnclaveImage {
                actions: TinyMapView::new(&actions),
                ..IMAGE
            };
            let view = EnclaveImageView::new(&image).unwrap();
            assert_eq!(view.actions()[ActionIndex::new(0)].timing(), timing);
            assert_eq!(
                view.actions()[ActionIndex::new(0)].binding(),
                matches!(timing, ActionTiming::Standard { .. }).then_some(BindingSlotIndex::new(5))
            );
        }
    }

    #[test]
    fn invalid_cross_references_and_ranges_report_the_source_location() {
        let bad_reactions = [ReactionImage::new(
            ReactorIndex::new(9),
            ScopeIndex::new(1),
            0,
            BindingSlotIndex::new(0),
            RANGE_0_0,
            RANGE_0_0,
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )];
        let bad_actions = [ActionImage::new(
            ScopeIndex::new(1),
            ActionSlotIndex::new(0),
            ActionTiming::Standard {
                domain: TimingDomain::Logical,
                min_delay_nanos: 7,
            },
            SliceRange::new(4, 1),
            Some(BindingSlotIndex::new(5)),
        )];
        let cases = [
            (
                "primary cross-reference",
                EnclaveImage {
                    reactions: TinyMapView::new(&bad_reactions),
                    ..IMAGE
                },
                ImageValidationError::ReferenceOutOfBounds {
                    table: "reactions",
                    index: 0,
                    field: "reactor",
                    target: "reactors",
                    referenced: 9,
                },
            ),
            (
                "flattened range",
                EnclaveImage {
                    actions: TinyMapView::new(&bad_actions),
                    ..IMAGE
                },
                ImageValidationError::RangeOutOfBounds {
                    table: "actions",
                    index: 0,
                    field: "triggers",
                    target: "reaction_triggers",
                    start: 4,
                    len: 1,
                },
            ),
        ];

        for (name, image, expected) in cases {
            assert_eq!(
                EnclaveImageView::new(&image).unwrap_err(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn cyclic_scope_parents_are_rejected_before_execution() {
        let scopes = [
            ScopeImage::new(
                Some(ScopeIndex::new(0)),
                ReactorIndex::new(0),
                None,
                SliceRange::new(0, 2),
                SliceRange::new(0, 1),
                SliceRange::new(0, 1),
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
                SliceRange::new(0, 0),
            ),
            SCOPES[1],
            SCOPES[2],
        ];
        let image = EnclaveImage {
            scopes: TinyMapView::new(&scopes),
            ..IMAGE
        };

        let error = EnclaveImageView::new(&image).expect_err("scope cycle must be rejected");

        assert_eq!(error, ImageValidationError::ScopeParentCycle { scope: 0 });
    }

    #[test]
    fn invalid_trigger_order_and_duplicates_report_the_flattened_entry() {
        let unsorted = [
            LevelReactionImage::new(1, ReactionIndex::new(1)),
            LevelReactionImage::new(0, ReactionIndex::new(0)),
            REACTION_TRIGGERS[2],
            REACTION_TRIGGERS[3],
        ];
        let duplicate = [
            LevelReactionImage::new(0, ReactionIndex::new(0)),
            LevelReactionImage::new(0, ReactionIndex::new(0)),
            REACTION_TRIGGERS[2],
            REACTION_TRIGGERS[3],
        ];
        let cases = [
            (
                "unsorted",
                &unsorted[..],
                ImageValidationError::EntriesNotSorted {
                    table: "reaction_triggers",
                    index: 1,
                },
            ),
            (
                "duplicate",
                &duplicate[..],
                ImageValidationError::DuplicateEntry {
                    table: "reaction_triggers",
                    index: 1,
                },
            ),
        ];

        for (name, triggers, expected) in cases {
            let image = EnclaveImage {
                reaction_triggers: triggers,
                ..IMAGE
            };
            assert_eq!(
                EnclaveImageView::new(&image).unwrap_err(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn invalid_ownership_identity_and_storage_report_specific_errors() {
        let bad_modes = [ModeImage::new(ReactorIndex::new(1), ScopeIndex::new(1))];
        let bad_mode_effect_reactions = [
            REACTIONS[0],
            REACTIONS[1].with_mode_effect(crate::CompiledModeEffectRef {
                target: ModeIndex::new(0),
                transition: crate::TransitionKind::Reset,
            }),
        ];
        let invalid_routes = [route_image(
            " boundary",
            PortIndex::new(1),
            RouteDirection::Inbound,
            TimingDomain::Logical,
            0,
        )];
        let unsorted_routes = [
            route_image(
                "z",
                PortIndex::new(0),
                RouteDirection::Outbound,
                TimingDomain::Logical,
                0,
            ),
            route_image(
                "a",
                PortIndex::new(1),
                RouteDirection::Inbound,
                TimingDomain::Logical,
                0,
            ),
        ];
        let duplicate_bindings = [
            binding_image("same", BindingKind::Reaction),
            binding_image("same", BindingKind::Reaction),
            binding_image("state/r0", BindingKind::StateInitializer),
            binding_image("state/r1", BindingKind::StateInitializer),
            binding_image("tate/r0", BindingKind::Port),
            binding_image("trol", BindingKind::Action),
        ];
        let invalid_bank_reactors = [
            ReactorImage::new(
                BindingSlotIndex::new(2),
                StateSlotIndex::new(0),
                ScopeIndex::new(0),
                IndexSpan::new(0, 1),
                Some(ModeIndex::new(0)),
                Some(BankInfoImage::new(2, 2)),
            ),
            REACTORS[1],
        ];
        let cases = [
            (
                "ownership",
                EnclaveImage {
                    modes: TinyMapView::new(&bad_modes),
                    ..IMAGE
                },
                ImageValidationError::OwnershipMismatch {
                    table: "modes",
                    index: 0,
                    field: "scope.reactor",
                },
            ),
            (
                "mode effect ownership",
                EnclaveImage {
                    reactions: TinyMapView::new(&bad_mode_effect_reactions),
                    ..IMAGE
                },
                ImageValidationError::OwnershipMismatch {
                    table: "reactions",
                    index: 1,
                    field: "mode_effect.target.reactor",
                },
            ),
            (
                "invalid boundary",
                EnclaveImage {
                    routes: TinyMapView::new(&invalid_routes),
                    ..IMAGE
                },
                ImageValidationError::InvalidStableId {
                    kind: "boundary",
                    index: 0,
                    id: " boundary",
                },
            ),
            (
                "unsorted boundary",
                EnclaveImage {
                    routes: TinyMapView::new(&unsorted_routes),
                    ..IMAGE
                },
                ImageValidationError::StableIdsNotSorted {
                    kind: "boundary",
                    index: 1,
                    id: "a",
                },
            ),
            (
                "duplicate binding",
                EnclaveImage {
                    required_bindings: TinyMapView::new(&duplicate_bindings),
                    ..IMAGE
                },
                ImageValidationError::DuplicateStableId {
                    kind: "binding",
                    index: 1,
                    id: "same",
                },
            ),
            (
                "invalid bank",
                EnclaveImage {
                    reactors: TinyMapView::new(&invalid_bank_reactors),
                    ..IMAGE
                },
                ImageValidationError::InvalidBankInfo {
                    reactor: 0,
                    index: 2,
                    total: 2,
                },
            ),
            (
                "state storage bound",
                EnclaveImage {
                    storage_bounds: StorageBounds::new(1, 1, 8, 0, 0, 4),
                    ..IMAGE
                },
                ImageValidationError::StorageBoundExceeded {
                    table: "reactors",
                    index: 1,
                    kind: "state slots",
                    slot: 1,
                    bound: 1,
                },
            ),
        ];

        for (name, image, expected) in cases {
            assert_eq!(
                EnclaveImageView::new(&image).unwrap_err(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn enclave_image_requires_a_root_reactor() {
        let image = EnclaveImage {
            enclave_id: EnclaveId::new("rootless"),
            reactors: TinyMapView::new(&[]),
            actions: TinyMapView::new(&[]),
            ports: TinyMapView::new(&[]),
            reactions: TinyMapView::new(&[]),
            modes: TinyMapView::new(&[]),
            scopes: TinyMapView::new(&[]),
            reaction_triggers: &[],
            reaction_use_ports: &[],
            reaction_effect_ports: &[],
            reaction_actions: &[],
            reaction_modes: &[],
            scope_descendants: &[],
            scope_logical_actions: &[],
            scope_timer_startups: &[],
            scope_reset_reactions: &[],
            scope_startup_reactions: &[],
            scope_shutdown_reactions: &[],
            startup_actions: &[],
            timer_startup_actions: &[],
            shutdown_reactions: &[],
            shutdown_actions: &[],
            routes: TinyMapView::new(&[]),
            required_bindings: TinyMapView::new(&[]),
            storage_bounds: StorageBounds::new(0, 0, 0, 0, 0, 0),
        };

        assert_eq!(
            EnclaveImageView::new(&image).unwrap_err(),
            ImageValidationError::ReferenceOutOfBounds {
                table: "image",
                index: 0,
                field: "root_reactor",
                target: "reactors",
                referenced: 0,
            }
        );
    }
}
