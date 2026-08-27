use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use super::{
    action::{Action, ActionKind},
    component::ComponentInstance,
    connection::{Connection, ConnectionSemantics},
    enclave::Enclave,
    mode::Mode,
    placement_group::PlacementGroup,
    port::{Port, PortDirection},
    reaction::{Reaction, ReactionOptions, ReactionRelation, ReactionRelationTarget},
    reactor::Reactor,
    BankMember,
};
use crate::compiler::{
    ActionId, ApplicationId, BoundaryId, ComponentInstanceId, InvalidStableId, ModeId,
    PlacementGroupId, PortId, ReactionId, ReactorId, StableEnclaveId, StablePath,
    StablePathSegment,
};

/// Reports an invalid declaration or cross-entity structural reference.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TopologyBuildError {
    /// A stable identity was declared more than once for an entity category.
    #[error("duplicate {kind} identity '{id}'")]
    DuplicateIdentity {
        /// Entity category.
        kind: &'static str,
        /// Conflicting canonical identity.
        id: String,
    },
    /// A stable reference did not resolve to the expected entity category.
    #[error("{owner} references missing {kind} '{id}'")]
    MissingReference {
        /// Entity containing the invalid reference.
        owner: String,
        /// Expected entity category.
        kind: &'static str,
        /// Missing canonical identity.
        id: String,
    },
    /// An entity is attached to an inconsistent owner or parent.
    #[error("invalid ownership for {entity}: {reason}")]
    InvalidOwnership {
        /// Entity with invalid ownership.
        entity: String,
        /// Validation reason.
        reason: String,
    },
    /// A connection violates structural direction rules.
    #[error("invalid connection '{connection}': {reason}")]
    InvalidConnection {
        /// Stable connection identity.
        connection: BoundaryId,
        /// Validation reason.
        reason: &'static str,
    },
    /// A reaction relation is duplicated or has inconsistent ordering.
    #[error("invalid relations for reaction '{reaction}': {reason}")]
    InvalidReactionRelations {
        /// Stable reaction identity.
        reaction: ReactionId,
        /// Validation reason.
        reason: &'static str,
    },
    /// A mode hierarchy or modal reaction reference is invalid.
    #[error("invalid modal structure for '{entity}': {reason}")]
    InvalidModalStructure {
        /// Stable entity identity.
        entity: String,
        /// Validation reason.
        reason: String,
    },
}

/// Stages structural declarations by stable identity before validation.
#[derive(Debug)]
pub struct ApplicationTopologyBuilder {
    /// Stable identity of the application being assembled.
    application_id: ApplicationId,
    /// Components ordered by stable identity.
    components: BTreeMap<ComponentInstanceId, ComponentInstance>,
    /// Reactors ordered by stable identity.
    reactors: BTreeMap<ReactorId, Reactor>,
    /// Actions ordered by stable identity.
    actions: BTreeMap<ActionId, Action>,
    /// Ports ordered by stable identity.
    ports: BTreeMap<PortId, Port>,
    /// Reactions ordered by stable identity.
    reactions: BTreeMap<ReactionId, Reaction>,
    /// Connections ordered by stable identity.
    connections: BTreeMap<BoundaryId, Connection>,
    /// Modes ordered by stable identity.
    modes: BTreeMap<ModeId, Mode>,
    /// Enclaves ordered by stable identity.
    enclaves: BTreeMap<StableEnclaveId, Enclave>,
    /// Placement groups ordered by stable identity.
    placement_groups: BTreeMap<PlacementGroupId, PlacementGroup>,
}

macro_rules! insert_unique {
    ($map:expr, $id:expr, $value:expr, $kind:literal) => {{
        let id = $id;
        match $map.entry(id.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert($value);
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(_) => {
                Err(TopologyBuildError::DuplicateIdentity {
                    kind: $kind,
                    id: id.to_string(),
                })
            }
        }
    }};
}

impl ApplicationTopologyBuilder {
    /// Creates an empty topology builder.
    pub fn new(application_id: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
        Ok(Self {
            application_id: ApplicationId::new(application_id)?,
            components: BTreeMap::new(),
            reactors: BTreeMap::new(),
            actions: BTreeMap::new(),
            ports: BTreeMap::new(),
            reactions: BTreeMap::new(),
            connections: BTreeMap::new(),
            modes: BTreeMap::new(),
            enclaves: BTreeMap::new(),
            placement_groups: BTreeMap::new(),
        })
    }

    /// Stages one component declaration.
    pub fn add_component(
        &mut self,
        component: ComponentInstance,
    ) -> Result<(), TopologyBuildError> {
        let id = component.id.clone();
        insert_unique!(self.components, id, component, "component")
    }

    /// Stages one reactor declaration.
    pub fn add_reactor(&mut self, reactor: Reactor) -> Result<(), TopologyBuildError> {
        let id = reactor.id.clone();
        insert_unique!(self.reactors, id, reactor, "reactor")
    }

    /// Stages one action declaration.
    pub fn add_action(
        &mut self,
        id: ActionId,
        reactor: ReactorId,
        kind: ActionKind,
        declaration_position: u32,
        mode: Option<ModeId>,
    ) -> Result<(), TopologyBuildError> {
        let action = Action {
            id: id.clone(),
            reactor,
            kind,
            declaration_position,
            mode,
        };
        insert_unique!(self.actions, id, action, "action")
    }

    /// Stages one port declaration.
    pub fn add_port(
        &mut self,
        id: PortId,
        reactor: ReactorId,
        direction: PortDirection,
        bank: Option<BankMember>,
        declaration_position: u32,
        mode: Option<ModeId>,
    ) -> Result<(), TopologyBuildError> {
        let port = Port {
            id: id.clone(),
            reactor,
            direction,
            bank,
            declaration_position,
            mode,
        };
        insert_unique!(self.ports, id, port, "port")
    }

    /// Stages one reaction declaration.
    pub fn add_reaction(
        &mut self,
        id: ReactionId,
        reactor: ReactorId,
        relations: impl IntoIterator<Item = ReactionRelation>,
        options: ReactionOptions,
    ) -> Result<(), TopologyBuildError> {
        let reaction = Reaction {
            id: id.clone(),
            reactor,
            relations: relations.into_iter().collect(),
            options,
        };
        insert_unique!(self.reactions, id, reaction, "reaction")
    }

    /// Stages one directed connection.
    pub fn add_connection(
        &mut self,
        id: BoundaryId,
        source: PortId,
        target: PortId,
        semantics: ConnectionSemantics,
    ) -> Result<(), TopologyBuildError> {
        let connection = Connection {
            id: id.clone(),
            source,
            target,
            semantics,
        };
        insert_unique!(self.connections, id, connection, "connection")
    }

    /// Stages one mode declaration.
    pub fn add_mode(
        &mut self,
        id: ModeId,
        reactor: ReactorId,
        parent: Option<ModeId>,
        initial: bool,
    ) -> Result<(), TopologyBuildError> {
        let mode = Mode {
            id: id.clone(),
            reactor,
            parent,
            initial,
        };
        insert_unique!(self.modes, id, mode, "mode")
    }

    /// Stages one Enclave and its root reactor.
    pub fn add_enclave(
        &mut self,
        id: StableEnclaveId,
        root: ReactorId,
    ) -> Result<(), TopologyBuildError> {
        let enclave = Enclave {
            id: id.clone(),
            root,
        };
        insert_unique!(self.enclaves, id, enclave, "enclave")
    }

    /// Stages one hierarchical placement group.
    pub fn add_placement_group(
        &mut self,
        id: PlacementGroupId,
        parent: Option<PlacementGroupId>,
    ) -> Result<(), TopologyBuildError> {
        let group = PlacementGroup {
            id: id.clone(),
            parent,
        };
        insert_unique!(self.placement_groups, id, group, "placement group")
    }

    /// Validates and freezes the target-neutral topology.
    pub fn finish(mut self) -> Result<ApplicationTopology, TopologyBuildError> {
        validate_topology(&self)?;
        for reaction in self.reactions.values_mut() {
            reaction.canonicalize();
        }
        Ok(ApplicationTopology {
            application_id: self.application_id,
            components: self.components,
            reactors: self.reactors,
            actions: self.actions,
            ports: self.ports,
            reactions: self.reactions,
            connections: self.connections,
            modes: self.modes,
            enclaves: self.enclaves,
            placement_groups: self.placement_groups,
        })
    }
}

fn require<'a, I, T>(
    map: &'a BTreeMap<I, T>,
    id: &I,
    owner: &impl fmt::Display,
    kind: &'static str,
) -> Result<&'a T, TopologyBuildError>
where
    I: Ord + fmt::Display,
{
    map.get(id)
        .ok_or_else(|| TopologyBuildError::MissingReference {
            owner: owner.to_string(),
            kind,
            id: id.to_string(),
        })
}

fn relative_segments<'a>(
    child: &'a StablePath,
    owner: &StablePath,
) -> Option<&'a [StablePathSegment]> {
    child.segments().strip_prefix(owner.segments())
}

fn validate_name_only_path(
    id: &(impl std::ops::Deref<Target = StablePath> + fmt::Display),
    kind: &'static str,
) -> Result<(), TopologyBuildError> {
    if id
        .segments()
        .iter()
        .all(|segment| matches!(segment, StablePathSegment::Name(_)))
    {
        Ok(())
    } else {
        Err(invalid_ownership(
            id,
            format!("{kind} path must contain only named segments"),
        ))
    }
}

fn validate_name_child(
    child: &(impl std::ops::Deref<Target = StablePath> + fmt::Display),
    owner: &StablePath,
    kind: &'static str,
) -> Result<(), TopologyBuildError> {
    let path: &StablePath = child;
    if matches!(
        relative_segments(path, owner),
        Some([StablePathSegment::Name(_)])
    ) {
        Ok(())
    } else {
        Err(invalid_ownership(
            child,
            format!("{kind} path must be one named child of declared owner '{owner}'"),
        ))
    }
}

/// Collected structural members belonging to one bank declaration.
struct BankFamily {
    /// Total declared by every member of the family.
    total: u32,
    /// Bank indices present in the topology.
    indices: BTreeSet<u32>,
}

fn validate_bank_child(
    child: &(impl std::ops::Deref<Target = StablePath> + fmt::Display),
    owner: &StablePath,
    bank: Option<BankMember>,
    kind: &'static str,
) -> Result<Option<(StablePath, BankMember)>, TopologyBuildError> {
    let path: &StablePath = child;
    match (relative_segments(path, owner), bank) {
        (Some([StablePathSegment::Name(_)]), None) => Ok(None),
        (
            Some([StablePathSegment::Name(_), StablePathSegment::BankIndex(path_index)]),
            Some(member),
        ) if *path_index == member.index() => Ok(Some((
            path.parent().expect("bank member has a named base"),
            member,
        ))),
        (Some([StablePathSegment::Name(_)]), Some(_)) => Err(invalid_ownership(
            child,
            format!("{kind} bank metadata requires a typed bank suffix"),
        )),
        (
            Some([StablePathSegment::Name(_), StablePathSegment::BankIndex(_)]),
            None,
        ) => Err(invalid_ownership(
            child,
            format!("{kind} typed bank suffix requires bank metadata"),
        )),
        (
            Some([StablePathSegment::Name(_), StablePathSegment::BankIndex(path_index)]),
            Some(member),
        ) => Err(invalid_ownership(
            child,
            format!(
                "{kind} bank suffix index {path_index} does not match metadata index {}",
                member.index()
            ),
        )),
        _ => Err(invalid_ownership(
            child,
            format!(
                "{kind} path must be owner/name or banked owner/name/#bN for declared owner '{owner}'"
            ),
        )),
    }
}

fn record_bank_member(
    families: &mut BTreeMap<StablePath, BankFamily>,
    base: StablePath,
    member: BankMember,
) -> Result<(), TopologyBuildError> {
    let family = families.entry(base.clone()).or_insert_with(|| BankFamily {
        total: member.total(),
        indices: BTreeSet::new(),
    });
    if family.total != member.total() {
        return Err(invalid_ownership(
            &base,
            format!(
                "bank family has inconsistent totals {} and {}",
                family.total,
                member.total()
            ),
        ));
    }
    family.indices.insert(member.index());
    Ok(())
}

fn validate_bank_families(
    families: BTreeMap<StablePath, BankFamily>,
    ordinary_bases: &BTreeSet<StablePath>,
) -> Result<(), TopologyBuildError> {
    for (base, family) in families {
        if ordinary_bases.contains(&base) {
            return Err(invalid_ownership(
                &base,
                "declaration base cannot be both ordinary and banked",
            ));
        }
        if family.indices.iter().copied().ne(0..family.total) {
            return Err(invalid_ownership(
                &base,
                format!(
                    "bank family must contain every index from 0 through {}",
                    family.total - 1
                ),
            ));
        }
    }
    Ok(())
}

fn validate_declaration_positions(
    positions: BTreeMap<ReactorId, Vec<u32>>,
    kind: &'static str,
) -> Result<(), TopologyBuildError> {
    for (reactor, mut positions) in positions {
        positions.sort_unstable();
        let len = u32::try_from(positions.len()).unwrap_or(u32::MAX);
        if positions.into_iter().ne(0..len) {
            return Err(invalid_ownership(
                &reactor,
                format!(
                    "{kind} declaration positions must be unique contiguous ordinals starting at zero"
                ),
            ));
        }
    }
    Ok(())
}

fn invalid_ownership(entity: &impl fmt::Display, reason: impl Into<String>) -> TopologyBuildError {
    TopologyBuildError::InvalidOwnership {
        entity: entity.to_string(),
        reason: reason.into(),
    }
}

fn invalid_modal(entity: &impl fmt::Display, reason: impl Into<String>) -> TopologyBuildError {
    TopologyBuildError::InvalidModalStructure {
        entity: entity.to_string(),
        reason: reason.into(),
    }
}

fn validate_topology(builder: &ApplicationTopologyBuilder) -> Result<(), TopologyBuildError> {
    validate_components(&builder.components)?;
    validate_modes(&builder.modes, &builder.reactors)?;
    validate_placement_groups(&builder.placement_groups)?;
    validate_reactors(
        &builder.reactors,
        &builder.components,
        &builder.modes,
        &builder.enclaves,
        &builder.placement_groups,
    )?;
    validate_actions(&builder.actions, &builder.reactors, &builder.modes)?;
    validate_ports(&builder.ports, &builder.reactors, &builder.modes)?;
    validate_connections(&builder.connections, &builder.ports, &builder.reactors)?;
    validate_enclaves(&builder.enclaves, &builder.reactors)?;
    validate_reactions(
        &builder.reactions,
        &builder.reactors,
        &builder.actions,
        &builder.ports,
        &builder.modes,
    )
}

fn validate_components(
    components: &BTreeMap<ComponentInstanceId, ComponentInstance>,
) -> Result<(), TopologyBuildError> {
    for component in components.values() {
        validate_name_only_path(&component.id, "component")?;
    }
    Ok(())
}

fn validate_modes(
    modes: &BTreeMap<ModeId, Mode>,
    reactors: &BTreeMap<ReactorId, Reactor>,
) -> Result<(), TopologyBuildError> {
    for mode in modes.values() {
        require(reactors, &mode.reactor, &mode.id, "reactor")?;
        let expected_parent = mode
            .parent
            .as_ref()
            .map(|id| id.path())
            .unwrap_or(mode.reactor.path());
        if !matches!(
            relative_segments(mode.id.path(), expected_parent),
            Some([StablePathSegment::Name(_)])
        ) {
            return Err(invalid_modal(
                &mode.id,
                format!("mode path must be one named child of declared owner '{expected_parent}'"),
            ));
        }
        if let Some(parent) = &mode.parent {
            let parent_mode = require(modes, parent, &mode.id, "mode")?;
            if parent_mode.reactor != mode.reactor {
                return Err(invalid_modal(
                    &mode.id,
                    "parent mode belongs to another reactor",
                ));
            }
        }
    }

    let mut initial = BTreeSet::new();
    for mode in modes.values() {
        if let Some(parent) = &mode.parent {
            let mut cursor = Some(parent);
            let mut seen = BTreeSet::new();
            while let Some(id) = cursor {
                if id == &mode.id || !seen.insert(id) {
                    return Err(invalid_modal(&mode.id, "mode hierarchy contains a cycle"));
                }
                cursor = require(modes, id, &mode.id, "mode")?.parent.as_ref();
            }
        }
        if mode.initial && !initial.insert((mode.reactor.clone(), mode.parent.clone())) {
            return Err(invalid_modal(&mode.id, "multiple initial sibling modes"));
        }
    }
    Ok(())
}

fn validate_reactors(
    reactors: &BTreeMap<ReactorId, Reactor>,
    components: &BTreeMap<ComponentInstanceId, ComponentInstance>,
    modes: &BTreeMap<ModeId, Mode>,
    enclaves: &BTreeMap<StableEnclaveId, Enclave>,
    placement_groups: &BTreeMap<PlacementGroupId, PlacementGroup>,
) -> Result<(), TopologyBuildError> {
    let mut bank_families = BTreeMap::new();
    let mut ordinary_bases = BTreeSet::new();
    for reactor in reactors.values() {
        require(components, &reactor.component, &reactor.id, "component")?;
        let bank = if let Some(parent) = reactor.parent.as_ref() {
            validate_bank_child(&reactor.id, parent.path(), reactor.bank, "reactor")?
        } else {
            // Component ownership is the typed reference above; a top-level reactor path is an
            // independent name-only ancestry, optionally ending in one typed bank index.
            match (reactor.id.segments(), reactor.bank) {
                (segments, None)
                    if !segments.is_empty()
                        && segments
                            .iter()
                            .all(|segment| matches!(segment, StablePathSegment::Name(_))) =>
                {
                    None
                }
                (segments, Some(member))
                    if segments.len() > 1
                        && matches!(segments.last(), Some(StablePathSegment::BankIndex(index)) if *index == member.index())
                        && segments[..segments.len() - 1]
                            .iter()
                            .all(|segment| matches!(segment, StablePathSegment::Name(_))) =>
                {
                    Some((
                        reactor
                            .id
                            .parent()
                            .expect("bank member has a named base")
                            .path()
                            .clone(),
                        member,
                    ))
                }
                _ => {
                    return Err(invalid_ownership(
                        &reactor.id,
                        "top-level reactor path must be name-only or end in a matching bank index",
                    ));
                }
            }
        };
        match bank {
            Some((base, member)) => record_bank_member(&mut bank_families, base, member)?,
            None => {
                ordinary_bases.insert(reactor.id.path().clone());
            }
        }
        if let Some(parent_id) = &reactor.parent {
            let parent = require(reactors, parent_id, &reactor.id, "reactor")?;
            if reactor.component != parent.component {
                return Err(invalid_ownership(
                    &reactor.id,
                    "child reactor belongs to a different component than its parent",
                ));
            }
        }
        require(enclaves, &reactor.enclave, &reactor.id, "enclave")?;
        if let Some(group) = &reactor.placement_group {
            require(placement_groups, group, &reactor.id, "placement group")?;
        }
        if let Some(scope_mode) = &reactor.scope_mode {
            let parent = reactor.parent.as_ref().ok_or_else(|| {
                invalid_modal(
                    &reactor.id,
                    "reactor scope mode requires a structural parent",
                )
            })?;
            let mode = require(modes, scope_mode, &reactor.id, "mode")?;
            if &mode.reactor != parent {
                return Err(invalid_modal(
                    &reactor.id,
                    "reactor scope mode is not owned by its structural parent",
                ));
            }
        }
    }
    validate_bank_families(bank_families, &ordinary_bases)?;

    for reactor in reactors.values() {
        let enclave = require(enclaves, &reactor.enclave, &reactor.id, "enclave")?;
        let enclave_root = require(reactors, &enclave.root, &reactor.id, "reactor")?;
        let mut cursor = &reactor.id;
        loop {
            if cursor == &enclave.root {
                break;
            }
            let current = require(reactors, cursor, &reactor.id, "reactor")?;
            if current.enclave != reactor.enclave {
                return Err(invalid_ownership(
                    &reactor.id,
                    "reactor ancestry crosses an enclave boundary",
                ));
            }
            let Some(parent) = current.parent.as_ref() else {
                // One scheduler domain may contain multiple disconnected top-level component
                // trees, but a nested Enclave remains rooted in its structural subtree.
                if enclave_root.parent.is_some() {
                    return Err(invalid_ownership(
                        &reactor.id,
                        "reactor is disconnected from its nested Enclave root",
                    ));
                }
                break;
            };
            cursor = parent;
        }
    }
    Ok(())
}

fn validate_placement_groups(
    groups: &BTreeMap<PlacementGroupId, PlacementGroup>,
) -> Result<(), TopologyBuildError> {
    for group in groups.values() {
        validate_name_only_path(&group.id, "placement group")?;
        if let Some(parent) = &group.parent {
            validate_name_child(&group.id, parent.path(), "placement group")?;
            require(groups, parent, &group.id, "placement group")?;
        }
    }

    for group in groups.values() {
        let mut cursor = group.parent.as_ref();
        let mut seen = BTreeSet::new();
        while let Some(id) = cursor {
            if id == &group.id || !seen.insert(id) {
                return Err(invalid_ownership(
                    &group.id,
                    "placement-group hierarchy contains a cycle",
                ));
            }
            cursor = require(groups, id, &group.id, "placement group")?
                .parent
                .as_ref();
        }
    }
    Ok(())
}

fn validate_mode_owner(
    mode: Option<&ModeId>,
    reactor: &ReactorId,
    modes: &BTreeMap<ModeId, Mode>,
    owner: &impl fmt::Display,
) -> Result<(), TopologyBuildError> {
    if let Some(mode_id) = mode {
        let mode = require(modes, mode_id, owner, "mode")?;
        if &mode.reactor != reactor {
            return Err(invalid_modal(owner, "mode belongs to another reactor"));
        }
    }
    Ok(())
}

fn validate_actions(
    actions: &BTreeMap<ActionId, Action>,
    reactors: &BTreeMap<ReactorId, Reactor>,
    modes: &BTreeMap<ModeId, Mode>,
) -> Result<(), TopologyBuildError> {
    let mut positions = BTreeMap::<ReactorId, Vec<u32>>::new();
    for action in actions.values() {
        validate_name_child(&action.id, action.reactor.path(), "action")?;
        require(reactors, &action.reactor, &action.id, "reactor")?;
        validate_mode_owner(action.mode.as_ref(), &action.reactor, modes, &action.id)?;
        positions
            .entry(action.reactor.clone())
            .or_default()
            .push(action.declaration_position);
    }
    validate_declaration_positions(positions, "action")
}

fn validate_ports(
    ports: &BTreeMap<PortId, Port>,
    reactors: &BTreeMap<ReactorId, Reactor>,
    modes: &BTreeMap<ModeId, Mode>,
) -> Result<(), TopologyBuildError> {
    let mut bank_families = BTreeMap::new();
    let mut ordinary_bases = BTreeSet::new();
    let mut positions = BTreeMap::<ReactorId, Vec<u32>>::new();
    for port in ports.values() {
        match validate_bank_child(&port.id, port.reactor.path(), port.bank, "port")? {
            Some((base, member)) => record_bank_member(&mut bank_families, base, member)?,
            None => {
                ordinary_bases.insert(port.id.path().clone());
            }
        }
        require(reactors, &port.reactor, &port.id, "reactor")?;
        validate_mode_owner(port.mode.as_ref(), &port.reactor, modes, &port.id)?;
        positions
            .entry(port.reactor.clone())
            .or_default()
            .push(port.declaration_position);
    }
    validate_bank_families(bank_families, &ordinary_bases)?;
    validate_declaration_positions(positions, "port")
}

fn validate_connections(
    connections: &BTreeMap<BoundaryId, Connection>,
    ports: &BTreeMap<PortId, Port>,
    reactors: &BTreeMap<ReactorId, Reactor>,
) -> Result<(), TopologyBuildError> {
    for connection in connections.values() {
        validate_name_only_path(&connection.id, "boundary")?;
        let source = require(ports, &connection.source, &connection.id, "port")?;
        let target = require(ports, &connection.target, &connection.id, "port")?;
        let source_reactor = require(reactors, &source.reactor, &connection.id, "reactor")?;
        let target_reactor = require(reactors, &target.reactor, &connection.id, "reactor")?;
        let (valid, reason) = match (source.direction, target.direction) {
            (PortDirection::Input, PortDirection::Input) => (
                target_reactor.parent.as_ref() == Some(&source.reactor),
                "input-to-input connection must enter a direct child reactor",
            ),
            (PortDirection::Output, PortDirection::Input) => (
                source_reactor.parent == target_reactor.parent,
                "output-to-input connection endpoints must be siblings",
            ),
            (PortDirection::Output, PortDirection::Output) => (
                source_reactor.parent.as_ref() == Some(&target.reactor),
                "output-to-output connection must leave a direct child reactor",
            ),
            (PortDirection::Input, PortDirection::Output) => {
                (false, "input-to-output connection is invalid")
            }
        };
        if !valid {
            return Err(TopologyBuildError::InvalidConnection {
                connection: connection.id.clone(),
                reason,
            });
        }
    }
    Ok(())
}

fn validate_enclaves(
    enclaves: &BTreeMap<StableEnclaveId, Enclave>,
    reactors: &BTreeMap<ReactorId, Reactor>,
) -> Result<(), TopologyBuildError> {
    for enclave in enclaves.values() {
        let root = require(reactors, &enclave.root, &enclave.id, "reactor")?;
        if enclave.id.path() != enclave.root.path() {
            return Err(invalid_ownership(
                &enclave.id,
                "enclave path must exactly match its declared root reactor path",
            ));
        }
        if root.enclave != enclave.id {
            return Err(invalid_ownership(
                &enclave.id,
                "declared root belongs to another enclave",
            ));
        }
    }
    Ok(())
}

fn validate_reactions(
    reactions: &BTreeMap<ReactionId, Reaction>,
    reactors: &BTreeMap<ReactorId, Reactor>,
    actions: &BTreeMap<ActionId, Action>,
    ports: &BTreeMap<PortId, Port>,
    modes: &BTreeMap<ModeId, Mode>,
) -> Result<(), TopologyBuildError> {
    let mut generated_families = BTreeMap::<StablePath, BTreeSet<u32>>::new();
    let mut scalar_bases = BTreeSet::new();
    for reaction in reactions.values() {
        let path = reaction.id.path();
        match relative_segments(path, reaction.reactor.path()) {
            Some([StablePathSegment::Name(_)]) => {
                scalar_bases.insert(path.clone());
            }
            Some([StablePathSegment::GeneratedOrdinal(ordinal)]) => {
                generated_families
                    .entry(reaction.reactor.path().clone())
                    .or_default()
                    .insert(*ordinal);
            }
            Some([StablePathSegment::Name(_), StablePathSegment::GeneratedOrdinal(ordinal)]) => {
                generated_families
                    .entry(path.parent().expect("named generated reaction has a base"))
                    .or_default()
                    .insert(*ordinal);
            }
            _ => {
                return Err(invalid_ownership(
                    &reaction.id,
                    "reaction path must be owner/name, owner/#gN, or owner/name/#gN",
                ));
            }
        }
        require(reactors, &reaction.reactor, &reaction.id, "reactor")?;

        let mut targets = BTreeSet::new();
        for relation in &reaction.relations {
            if relation.flags.is_empty() || !targets.insert(relation.target.clone()) {
                return Err(TopologyBuildError::InvalidReactionRelations {
                    reaction: reaction.id.clone(),
                    reason: "empty flags or duplicate target relation",
                });
            }
            match &relation.target {
                ReactionRelationTarget::Action(id) => {
                    require(actions, id, &reaction.id, "action")?;
                }
                ReactionRelationTarget::Port(id) => {
                    require(ports, id, &reaction.id, "port")?;
                }
            }
        }

        for action_targets in [true, false] {
            let mut positions = reaction
                .relations
                .iter()
                .filter(|relation| {
                    matches!(relation.target, ReactionRelationTarget::Action(_)) == action_targets
                })
                .map(|relation| relation.declaration_position)
                .collect::<Vec<_>>();
            positions.sort_unstable();
            if positions
                .iter()
                .copied()
                .ne(0..u32::try_from(positions.len()).unwrap_or(u32::MAX))
            {
                return Err(TopologyBuildError::InvalidReactionRelations {
                    reaction: reaction.id.clone(),
                    reason:
                        "action and port declaration positions must each be unique and contiguous",
                });
            }
        }

        validate_mode_owner(
            reaction.options.mode.as_ref(),
            &reaction.reactor,
            modes,
            &reaction.id,
        )?;
        for (kind, memberships) in [
            ("enabled", &reaction.options.enabled_modes),
            ("reset", &reaction.options.reset_modes),
        ] {
            let mut seen = BTreeSet::new();
            if memberships.iter().any(|mode| !seen.insert(mode)) {
                return Err(invalid_modal(
                    &reaction.id,
                    format!("duplicate {kind} mode membership"),
                ));
            }
        }
        for mode in reaction
            .options
            .enabled_modes
            .iter()
            .chain(&reaction.options.reset_modes)
        {
            let mode = require(modes, mode, &reaction.id, "mode")?;
            if mode.reactor != reaction.reactor {
                return Err(invalid_modal(
                    &reaction.id,
                    "reaction modal target belongs to another reactor",
                ));
            }
        }
        if let Some(transition) = &reaction.options.transition {
            validate_mode_owner(
                Some(&transition.target),
                &reaction.reactor,
                modes,
                &reaction.id,
            )?;
        }
    }
    for (base, ordinals) in generated_families {
        if scalar_bases.contains(&base) {
            return Err(invalid_ownership(
                &base,
                "identity base cannot be both a scalar and generated reaction family",
            ));
        }
        let len = u32::try_from(ordinals.len()).unwrap_or(u32::MAX);
        if ordinals.iter().copied().ne(0..len) {
            return Err(invalid_ownership(
                &base,
                "generated reaction ordinals must be contiguous starting at zero",
            ));
        }
    }
    Ok(())
}

/// Immutable target-neutral logical application structure.
#[derive(Clone, Eq, PartialEq)]
pub struct ApplicationTopology {
    /// Stable application identity.
    application_id: ApplicationId,
    /// Components keyed by stable identity.
    components: BTreeMap<ComponentInstanceId, ComponentInstance>,
    /// Reactors keyed by stable identity.
    reactors: BTreeMap<ReactorId, Reactor>,
    /// Actions keyed by stable identity.
    actions: BTreeMap<ActionId, Action>,
    /// Ports keyed by stable identity.
    ports: BTreeMap<PortId, Port>,
    /// Reactions keyed by stable identity.
    reactions: BTreeMap<ReactionId, Reaction>,
    /// Connections keyed by stable identity.
    connections: BTreeMap<BoundaryId, Connection>,
    /// Modes keyed by stable identity.
    modes: BTreeMap<ModeId, Mode>,
    /// Enclaves keyed by stable identity.
    enclaves: BTreeMap<StableEnclaveId, Enclave>,
    /// Placement groups keyed by stable identity.
    placement_groups: BTreeMap<PlacementGroupId, PlacementGroup>,
}

macro_rules! accessors {
    ($iter:ident, $get:ident, $field:ident, $id:ty, $item:ty) => {
        /// Iterates records in canonical stable-identity order.
        pub fn $iter(&self) -> impl Iterator<Item = (&$id, &$item)> {
            self.$field.iter()
        }

        /// Looks up a record by stable identity.
        pub fn $get(&self, id: &$id) -> Option<&$item> {
            self.$field.get(id)
        }
    };
}

impl ApplicationTopology {
    /// Returns the application identity.
    pub fn application_id(&self) -> &ApplicationId {
        &self.application_id
    }

    accessors!(
        components,
        component,
        components,
        ComponentInstanceId,
        ComponentInstance
    );
    accessors!(reactors, reactor, reactors, ReactorId, Reactor);
    accessors!(actions, action, actions, ActionId, Action);
    accessors!(ports, port, ports, PortId, Port);
    accessors!(reactions, reaction, reactions, ReactionId, Reaction);
    accessors!(connections, connection, connections, BoundaryId, Connection);
    accessors!(modes, mode, modes, ModeId, Mode);
    accessors!(enclaves, enclave, enclaves, StableEnclaveId, Enclave);
    accessors!(
        placement_groups,
        placement_group,
        placement_groups,
        PlacementGroupId,
        PlacementGroup
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id<T: std::str::FromStr>(value: &str) -> T
    where
        T::Err: fmt::Debug,
    {
        value.parse().unwrap()
    }

    fn base(
        component: &str,
        root: &str,
        enclave: &str,
    ) -> (ApplicationTopologyBuilder, ReactorId, StableEnclaveId) {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        let component: ComponentInstanceId = id(component);
        let root: ReactorId = id(root);
        let enclave: StableEnclaveId = id(enclave);
        builder
            .add_component(ComponentInstance::new(component.to_string(), "contract", 1).unwrap())
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                root.clone(),
                component,
                None,
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder.add_enclave(enclave.clone(), root.clone()).unwrap();
        (builder, root, enclave)
    }

    #[test]
    fn top_level_reactor_identity_is_independent_of_component_path() {
        let mut builder = ApplicationTopologyBuilder::new("application").unwrap();
        let component: ComponentInstanceId = id("component/encoded-root");
        let root: ReactorId = id("raw-root");
        let enclave: StableEnclaveId = id("raw-root");
        builder
            .add_component(ComponentInstance::new(component.to_string(), "contract", 1).unwrap())
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                root.clone(),
                component,
                None,
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder.add_enclave(enclave, root).unwrap();

        builder.finish().unwrap();
    }

    #[test]
    fn separate_action_and_port_positions_finalize_and_preserve_relations() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        let a0: ActionId = id("vehicle/root/a0");
        let a1: ActionId = id("vehicle/root/a1");
        let p0: PortId = id("vehicle/root/p0");
        let p1: PortId = id("vehicle/root/p1");
        builder
            .add_action(
                a0.clone(),
                reactor.clone(),
                ActionKind::Logical {
                    minimum_delay: None,
                },
                0,
                None,
            )
            .unwrap();
        builder
            .add_action(
                a1.clone(),
                reactor.clone(),
                ActionKind::Logical {
                    minimum_delay: None,
                },
                1,
                None,
            )
            .unwrap();
        builder
            .add_port(
                p0.clone(),
                reactor.clone(),
                PortDirection::Input,
                None,
                0,
                None,
            )
            .unwrap();
        builder
            .add_port(
                p1.clone(),
                reactor.clone(),
                PortDirection::Output,
                None,
                1,
                None,
            )
            .unwrap();
        let reaction_id: ReactionId = id("vehicle/root/reaction");
        let relations = [
            ReactionRelation::new(
                ReactionRelationTarget::Action(a1.clone()),
                super::super::ReactionRelationFlags::EFFECT,
                1,
            ),
            ReactionRelation::new(
                ReactionRelationTarget::Port(p0.clone()),
                super::super::ReactionRelationFlags::TRIGGER
                    | super::super::ReactionRelationFlags::USE,
                0,
            ),
            ReactionRelation::new(
                ReactionRelationTarget::Action(a0.clone()),
                super::super::ReactionRelationFlags::USE,
                0,
            ),
            ReactionRelation::new(
                ReactionRelationTarget::Port(p1.clone()),
                super::super::ReactionRelationFlags::EFFECT,
                1,
            ),
        ];
        builder
            .add_reaction(
                reaction_id.clone(),
                reactor,
                relations.clone(),
                ReactionOptions::default(),
            )
            .unwrap();

        let topology = builder.finish().unwrap();
        let reaction = topology.reaction(&reaction_id).unwrap();
        let expected = [
            relations[2].clone(),
            relations[0].clone(),
            relations[1].clone(),
            relations[3].clone(),
        ];
        assert_eq!(reaction.relations(), expected);
        assert!(reaction.relations()[2].flags().is_trigger());
        assert!(reaction.relations()[2].flags().is_use());
        assert_eq!(
            reaction
                .relations()
                .iter()
                .map(ReactionRelation::declaration_position)
                .collect::<Vec<_>>(),
            [0, 1, 0, 1]
        );
    }

    #[test]
    fn reaction_interleaving_and_modal_membership_order_are_canonical() {
        fn build(reverse: bool) -> ApplicationTopology {
            let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
            let active: ModeId = id("vehicle/root/active");
            let idle: ModeId = id("vehicle/root/idle");
            builder
                .add_mode(active.clone(), reactor.clone(), None, true)
                .unwrap();
            builder
                .add_mode(idle.clone(), reactor.clone(), None, false)
                .unwrap();

            let a0: ActionId = id("vehicle/root/a0");
            let a1: ActionId = id("vehicle/root/a1");
            let p0: PortId = id("vehicle/root/p0");
            let p1: PortId = id("vehicle/root/p1");
            builder
                .add_action(
                    a0.clone(),
                    reactor.clone(),
                    ActionKind::Logical {
                        minimum_delay: None,
                    },
                    0,
                    None,
                )
                .unwrap();
            builder
                .add_action(
                    a1.clone(),
                    reactor.clone(),
                    ActionKind::Logical {
                        minimum_delay: None,
                    },
                    1,
                    None,
                )
                .unwrap();
            builder
                .add_port(
                    p0.clone(),
                    reactor.clone(),
                    PortDirection::Input,
                    None,
                    0,
                    None,
                )
                .unwrap();
            builder
                .add_port(
                    p1.clone(),
                    reactor.clone(),
                    PortDirection::Output,
                    None,
                    1,
                    None,
                )
                .unwrap();

            let action0 = ReactionRelation::new(
                ReactionRelationTarget::Action(a0),
                super::super::ReactionRelationFlags::USE,
                0,
            );
            let action1 = ReactionRelation::new(
                ReactionRelationTarget::Action(a1),
                super::super::ReactionRelationFlags::EFFECT,
                1,
            );
            let port0 = ReactionRelation::new(
                ReactionRelationTarget::Port(p0),
                super::super::ReactionRelationFlags::TRIGGER,
                0,
            );
            let port1 = ReactionRelation::new(
                ReactionRelationTarget::Port(p1),
                super::super::ReactionRelationFlags::EFFECT,
                1,
            );
            let relations = if reverse {
                vec![port1, action1, port0, action0]
            } else {
                vec![action0, port0, action1, port1]
            };
            let options = if reverse {
                ReactionOptions {
                    enabled_modes: vec![idle.clone(), active.clone()],
                    reset_modes: vec![active, idle],
                    ..ReactionOptions::default()
                }
            } else {
                ReactionOptions {
                    enabled_modes: vec![active.clone(), idle.clone()],
                    reset_modes: vec![idle, active],
                    ..ReactionOptions::default()
                }
            };
            builder
                .add_reaction(id("vehicle/root/reaction"), reactor, relations, options)
                .unwrap();
            builder.finish().unwrap()
        }

        assert_eq!(build(false), build(true));
    }

    #[test]
    fn reaction_modal_membership_sets_reject_duplicates() {
        for reset_modes in [false, true] {
            let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
            let mode: ModeId = id("vehicle/root/mode");
            builder
                .add_mode(mode.clone(), reactor.clone(), None, true)
                .unwrap();
            let mut options = ReactionOptions::default();
            if reset_modes {
                options.reset_modes = vec![mode.clone(), mode];
            } else {
                options.enabled_modes = vec![mode.clone(), mode];
            }
            builder
                .add_reaction(id("vehicle/root/reaction"), reactor, [], options)
                .unwrap();
            assert!(matches!(
                builder.finish(),
                Err(TopologyBuildError::InvalidModalStructure { reason, .. })
                    if reason.contains("duplicate")
            ));
        }
    }

    #[test]
    fn child_reactor_rejects_a_different_component() {
        let (mut builder, root, enclave) = base("vehicle/a", "vehicle/a/root", "vehicle/a/root");
        let other: ComponentInstanceId = id("vehicle/b");
        builder
            .add_component(ComponentInstance::new("vehicle/b", "contract", 1).unwrap())
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                id("vehicle/a/root/child"),
                other,
                Some(root),
                None,
                enclave,
                None,
                None,
            ))
            .unwrap();
        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn placement_group_must_be_an_immediate_child_of_its_parent() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        let parent: PlacementGroupId = id("vehicle/b");
        builder.add_placement_group(parent.clone(), None).unwrap();
        builder
            .add_placement_group(id("vehicle/a"), Some(parent))
            .unwrap();
        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn cyclic_reactor_parents_are_rejected_directly() {
        let (mut builder, _, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
        let a: ReactorId = id("vehicle/root/a");
        let b: ReactorId = id("vehicle/root/a/b");
        builder
            .add_reactor(Reactor::new(
                a.clone(),
                id("vehicle"),
                Some(b.clone()),
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                b,
                id("vehicle"),
                Some(a),
                None,
                enclave,
                None,
                None,
            ))
            .unwrap();
        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn reactor_scope_mode_must_belong_to_its_structural_parent() {
        let (mut builder, root, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
        let mode: ModeId = id("vehicle/root/mode");
        builder
            .add_mode(mode.clone(), root.clone(), None, true)
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                id("vehicle/root/child"),
                id("vehicle"),
                Some(root),
                None,
                enclave,
                None,
                Some(mode),
            ))
            .unwrap();
        assert!(builder.finish().is_ok());

        let (mut child_owned, root, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
        let child: ReactorId = id("vehicle/root/child");
        let mode: ModeId = id("vehicle/root/child/mode");
        child_owned
            .add_mode(mode.clone(), child.clone(), None, true)
            .unwrap();
        child_owned
            .add_reactor(Reactor::new(
                child,
                id("vehicle"),
                Some(root),
                None,
                enclave,
                None,
                Some(mode),
            ))
            .unwrap();
        assert!(matches!(
            child_owned.finish(),
            Err(TopologyBuildError::InvalidModalStructure { .. })
        ));

        let (mut unrelated, root, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
        let sibling: ReactorId = id("vehicle/root/sibling");
        let mode: ModeId = id("vehicle/root/sibling/mode");
        unrelated
            .add_mode(mode.clone(), sibling.clone(), None, true)
            .unwrap();
        unrelated
            .add_reactor(Reactor::new(
                sibling,
                id("vehicle"),
                Some(root.clone()),
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        unrelated
            .add_reactor(Reactor::new(
                id("vehicle/root/child"),
                id("vehicle"),
                Some(root),
                None,
                enclave,
                None,
                Some(mode),
            ))
            .unwrap();
        assert!(matches!(
            unrelated.finish(),
            Err(TopologyBuildError::InvalidModalStructure { .. })
        ));
    }

    #[test]
    fn mode_paths_must_be_immediate_children_of_their_declared_parent() {
        let (mut root_case, root, _) = base("vehicle", "vehicle/root", "vehicle/root");
        root_case
            .add_mode(id("vehicle/root/deep/mode"), root, None, true)
            .unwrap();
        assert!(matches!(
            root_case.finish(),
            Err(TopologyBuildError::InvalidModalStructure { reason, .. })
                if reason.contains("vehicle/root")
        ));

        let (mut child_case, root, _) = base("vehicle", "vehicle/root", "vehicle/root");
        let parent: ModeId = id("vehicle/root/parent");
        child_case
            .add_mode(parent.clone(), root.clone(), None, true)
            .unwrap();
        child_case
            .add_mode(id("vehicle/root/wrong/child"), root, Some(parent), false)
            .unwrap();
        assert!(matches!(
            child_case.finish(),
            Err(TopologyBuildError::InvalidModalStructure { .. })
        ));
    }

    #[test]
    fn enclave_accepts_a_disconnected_parentless_reactor() {
        let (mut builder, _, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_reactor(Reactor::new(
                id("vehicle/other"),
                id("vehicle"),
                None,
                None,
                enclave,
                None,
                None,
            ))
            .unwrap();
        assert!(builder.finish().is_ok());
    }

    #[test]
    fn enclave_rejects_structural_ancestry_crossing_its_boundary() {
        let (mut builder, root, _) = base("vehicle", "vehicle/root", "vehicle/root");
        let nested: ReactorId = id("vehicle/root/nested");
        let enclave: StableEnclaveId = id("vehicle/root/nested");
        builder
            .add_reactor(Reactor::new(
                nested.clone(),
                id("vehicle"),
                Some(root.clone()),
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder.add_enclave(enclave.clone(), nested).unwrap();
        builder
            .add_reactor(Reactor::new(
                id("vehicle/root/wrong"),
                id("vehicle"),
                Some(root),
                None,
                enclave,
                None,
                None,
            ))
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { reason, .. })
                if reason.contains("crosses an enclave boundary")
        ));
    }

    #[test]
    fn enclave_rejects_disconnected_member_when_declared_root_is_nested() {
        let mut builder = ApplicationTopologyBuilder::new("application").unwrap();
        let component: ComponentInstanceId = id("vehicle");
        let parent: ReactorId = id("vehicle/a");
        let nested: ReactorId = id("vehicle/a/b");
        let enclave: StableEnclaveId = id("vehicle/a/b");
        builder
            .add_component(ComponentInstance::new(component.to_string(), "contract", 1).unwrap())
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                parent.clone(),
                component.clone(),
                None,
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                nested.clone(),
                component,
                Some(parent),
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder.add_enclave(enclave, nested).unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { reason, .. })
                if reason.contains("nested Enclave root")
        ));
    }

    #[test]
    fn connection_hierarchy_rejects_every_illegal_direction_shape() {
        fn finish(
            source_owner: &str,
            source_direction: PortDirection,
            target_owner: &str,
            target_direction: PortDirection,
        ) -> Result<ApplicationTopology, TopologyBuildError> {
            let (mut builder, root, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
            let left: ReactorId = id("vehicle/root/left");
            let right: ReactorId = id("vehicle/root/right");
            for child in [&left, &right] {
                builder
                    .add_reactor(Reactor::new(
                        child.clone(),
                        id("vehicle"),
                        Some(root.clone()),
                        None,
                        enclave.clone(),
                        None,
                        None,
                    ))
                    .unwrap();
            }
            let source_reactor: ReactorId = id(source_owner);
            let target_reactor: ReactorId = id(target_owner);
            let source: PortId = id(&format!("{source_owner}/source"));
            let target: PortId = id(&format!("{target_owner}/target"));
            builder
                .add_port(
                    source.clone(),
                    source_reactor,
                    source_direction,
                    None,
                    0,
                    None,
                )
                .unwrap();
            builder
                .add_port(
                    target.clone(),
                    target_reactor,
                    target_direction,
                    None,
                    u32::from(source_owner == target_owner),
                    None,
                )
                .unwrap();
            builder
                .add_connection(
                    id("boundary/invalid"),
                    source,
                    target,
                    ConnectionSemantics::Logical { after: None },
                )
                .unwrap();
            builder.finish()
        }

        let cases = [
            (
                "vehicle/root/left",
                PortDirection::Input,
                "vehicle/root/right",
                PortDirection::Input,
            ),
            (
                "vehicle/root",
                PortDirection::Output,
                "vehicle/root/left",
                PortDirection::Input,
            ),
            (
                "vehicle/root/left",
                PortDirection::Output,
                "vehicle/root/right",
                PortDirection::Output,
            ),
            (
                "vehicle/root",
                PortDirection::Input,
                "vehicle/root",
                PortDirection::Output,
            ),
        ];
        for (source_owner, source_direction, target_owner, target_direction) in cases {
            assert!(matches!(
                finish(
                    source_owner,
                    source_direction,
                    target_owner,
                    target_direction
                ),
                Err(TopologyBuildError::InvalidConnection { .. })
            ));
        }
    }

    #[test]
    fn nested_enclave_root_retains_its_structural_parent() {
        let (mut builder, root, _) = base("vehicle", "vehicle/root", "vehicle/root");
        let nested: ReactorId = id("vehicle/root/nested");
        let enclave: StableEnclaveId = id("vehicle/root/nested");
        builder
            .add_reactor(Reactor::new(
                nested.clone(),
                id("vehicle"),
                Some(root),
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder.add_enclave(enclave, nested).unwrap();
        assert!(builder.finish().is_ok());
    }

    #[test]
    fn reordered_input_produces_equal_canonical_topologies() {
        fn build(order: [&str; 3]) -> ApplicationTopology {
            let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
            for id in order {
                builder
                    .add_component(ComponentInstance::new(id, "sensor.v1", 1).unwrap())
                    .unwrap();
            }
            builder.finish().unwrap()
        }

        let forward = build(["vehicle/a", "vehicle/b", "vehicle/c"]);
        let reverse = build(["vehicle/c", "vehicle/b", "vehicle/a"]);
        assert_eq!(forward, reverse);
        assert_eq!(
            forward
                .components()
                .map(|(id, _)| id.to_string())
                .collect::<Vec<_>>(),
            ["vehicle/a", "vehicle/b", "vehicle/c"]
        );
    }

    #[test]
    fn bank_member_rejects_empty_and_out_of_range_values() {
        assert_eq!(
            crate::compiler::BankMember::new(0, 0),
            Err(crate::compiler::InvalidBankMember::Empty)
        );
        assert_eq!(
            crate::compiler::BankMember::new(3, 3),
            Err(crate::compiler::InvalidBankMember::IndexOutOfBounds { index: 3, total: 3 })
        );
    }

    #[test]
    fn reactor_and_port_bank_metadata_round_trip() {
        let (mut builder, root, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
        let mut reactors = Vec::new();
        let mut ports = Vec::new();
        for index in 0..2 {
            let reactor: ReactorId = id(&format!("vehicle/root/sensors/#b{index}"));
            builder
                .add_reactor(Reactor::new(
                    reactor.clone(),
                    id("vehicle"),
                    Some(root.clone()),
                    Some(crate::compiler::BankMember::new(index, 2).unwrap()),
                    enclave.clone(),
                    None,
                    None,
                ))
                .unwrap();
            reactors.push(reactor);
            let port: PortId = id(&format!("vehicle/root/readings/#b{index}"));
            builder
                .add_port(
                    port.clone(),
                    root.clone(),
                    PortDirection::Output,
                    Some(crate::compiler::BankMember::new(index, 2).unwrap()),
                    index,
                    None,
                )
                .unwrap();
            ports.push(port);
        }
        let topology = builder.finish().unwrap();
        for (index, reactor) in reactors.iter().enumerate() {
            let bank = topology.reactor(reactor).unwrap().bank().unwrap();
            assert_eq!((bank.index(), bank.total()), (index as u32, 2));
        }
        for (index, port) in ports.iter().enumerate() {
            let bank = topology.port(port).unwrap().bank().unwrap();
            assert_eq!((bank.index(), bank.total()), (index as u32, 2));
        }
    }

    #[test]
    fn action_positions_and_category_specific_timing_round_trip() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        let cases = [
            (
                "logical",
                ActionKind::Logical {
                    minimum_delay: Some(crate::runtime::Duration::milliseconds(1)),
                },
            ),
            (
                "physical",
                ActionKind::Physical {
                    minimum_delay: Some(crate::runtime::Duration::milliseconds(2)),
                },
            ),
            (
                "timer",
                ActionKind::Timer {
                    offset: Some(crate::runtime::Duration::milliseconds(3)),
                    period: Some(crate::runtime::Duration::milliseconds(4)),
                },
            ),
            ("startup", ActionKind::Startup),
            ("shutdown", ActionKind::Shutdown),
        ];
        let mut ids = Vec::new();
        for (position, (name, kind)) in cases.into_iter().enumerate() {
            let action: ActionId = id(&format!("vehicle/root/{name}"));
            builder
                .add_action(action.clone(), reactor.clone(), kind, position as u32, None)
                .unwrap();
            ids.push((action, kind, position as u32));
        }

        let topology = builder.finish().unwrap();
        for (id, kind, position) in ids {
            let action = topology.action(&id).unwrap();
            assert_eq!(action.kind(), kind);
            assert_eq!(action.declaration_position(), position);
        }
    }

    #[test]
    fn logical_and_physical_connection_semantics_round_trip() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        let source: PortId = id("vehicle/root/source");
        let target: PortId = id("vehicle/root/target");
        builder
            .add_port(
                source.clone(),
                reactor.clone(),
                PortDirection::Output,
                None,
                0,
                None,
            )
            .unwrap();
        builder
            .add_port(target.clone(), reactor, PortDirection::Input, None, 1, None)
            .unwrap();
        let logical: BoundaryId = id("vehicle/logical");
        let physical: BoundaryId = id("vehicle/physical");
        let logical_semantics = crate::compiler::ConnectionSemantics::Logical {
            after: Some(crate::runtime::Duration::milliseconds(5)),
        };
        let physical_semantics = crate::compiler::ConnectionSemantics::Physical {
            after: Some(crate::runtime::Duration::milliseconds(6)),
        };
        builder
            .add_connection(
                logical.clone(),
                source.clone(),
                target.clone(),
                logical_semantics,
            )
            .unwrap();
        builder
            .add_connection(physical.clone(), source, target, physical_semantics)
            .unwrap();

        let topology = builder.finish().unwrap();
        assert_eq!(
            topology.connection(&logical).unwrap().semantics(),
            logical_semantics
        );
        assert_eq!(
            topology.connection(&physical).unwrap().semantics(),
            physical_semantics
        );
    }

    #[test]
    fn named_reaction_typed_ordinal_is_logically_owned_by_its_reactor() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_reaction(
                id("vehicle/root/update/#g0"),
                reactor,
                [],
                ReactionOptions::default(),
            )
            .unwrap();

        assert!(builder.finish().is_ok());
    }

    #[test]
    fn bank_identity_index_must_match_metadata() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_port(
                id("vehicle/root/readings/#b1"),
                reactor,
                PortDirection::Output,
                Some(crate::compiler::BankMember::new(0, 2).unwrap()),
                0,
                None,
            )
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn bank_metadata_requires_a_typed_bank_suffix() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_port(
                id("vehicle/root/readings"),
                reactor,
                PortDirection::Output,
                Some(crate::compiler::BankMember::new(0, 1).unwrap()),
                0,
                None,
            )
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn typed_bank_suffix_requires_bank_metadata() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_port(
                id("vehicle/root/readings/#b0"),
                reactor,
                PortDirection::Output,
                None,
                0,
                None,
            )
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn bank_family_totals_must_be_consistent() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        for (index, total) in [(0, 2), (1, 3)] {
            builder
                .add_port(
                    id(&format!("vehicle/root/readings/#b{index}")),
                    reactor.clone(),
                    PortDirection::Output,
                    Some(crate::compiler::BankMember::new(index, total).unwrap()),
                    index,
                    None,
                )
                .unwrap();
        }

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn bank_family_must_contain_every_declared_member() {
        let (mut builder, root, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_reactor(Reactor::new(
                id("vehicle/root/sensors/#b0"),
                id("vehicle"),
                Some(root),
                Some(crate::compiler::BankMember::new(0, 2).unwrap()),
                enclave,
                None,
                None,
            ))
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn ordinary_entity_cannot_share_a_banked_declaration_base() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_port(
                id("vehicle/root/readings"),
                reactor.clone(),
                PortDirection::Output,
                None,
                0,
                None,
            )
            .unwrap();
        builder
            .add_port(
                id("vehicle/root/readings/#b0"),
                reactor,
                PortDirection::Output,
                Some(crate::compiler::BankMember::new(0, 1).unwrap()),
                1,
                None,
            )
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn action_rejects_typed_suffixes() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_action(
                id("vehicle/root/#g0"),
                reactor,
                ActionKind::Logical {
                    minimum_delay: None,
                },
                0,
                None,
            )
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn mode_rejects_typed_suffixes() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_mode(id("vehicle/root/#b0"), reactor, None, true)
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidModalStructure { .. })
        ));
    }

    #[test]
    fn reaction_rejects_bank_suffixes() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_reaction(
                id("vehicle/root/#b0"),
                reactor,
                [],
                ReactionOptions::default(),
            )
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn generated_reaction_ordinals_must_be_contiguous() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_reaction(
                id("vehicle/root/update/#g1"),
                reactor,
                [],
                ReactionOptions::default(),
            )
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn scalar_reaction_cannot_share_a_generated_family_base() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        for reaction in ["vehicle/root/update", "vehicle/root/update/#g0"] {
            builder
                .add_reaction(
                    id(reaction),
                    reactor.clone(),
                    [],
                    ReactionOptions::default(),
                )
                .unwrap();
        }

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { entity, reason })
                if entity == "vehicle/root/update"
                    && reason.contains("scalar and generated reaction family")
        ));
    }

    #[test]
    fn approved_identity_grammar_shapes_are_accepted() {
        let (mut builder, root, enclave) = base("vehicle", "vehicle/root", "vehicle/root");
        builder
            .add_reactor(Reactor::new(
                id("vehicle/root/sensors/#b0"),
                id("vehicle"),
                Some(root.clone()),
                Some(crate::compiler::BankMember::new(0, 1).unwrap()),
                enclave,
                None,
                None,
            ))
            .unwrap();
        for reaction in [
            "vehicle/root/update",
            "vehicle/root/#g0",
            "vehicle/root/refresh/#g0",
        ] {
            builder
                .add_reaction(id(reaction), root.clone(), [], ReactionOptions::default())
                .unwrap();
        }

        assert!(builder.finish().is_ok());
    }

    #[test]
    fn duplicate_action_declaration_positions_are_rejected() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        for name in ["first", "second"] {
            builder
                .add_action(
                    id(&format!("vehicle/root/{name}")),
                    reactor.clone(),
                    ActionKind::Logical {
                        minimum_delay: None,
                    },
                    0,
                    None,
                )
                .unwrap();
        }

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn gapped_action_declaration_positions_are_rejected() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        for (name, position) in [("first", 0), ("second", 2)] {
            builder
                .add_action(
                    id(&format!("vehicle/root/{name}")),
                    reactor.clone(),
                    ActionKind::Logical {
                        minimum_delay: None,
                    },
                    position,
                    None,
                )
                .unwrap();
        }

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn duplicate_port_declaration_positions_are_rejected() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        for name in ["first", "second"] {
            builder
                .add_port(
                    id(&format!("vehicle/root/{name}")),
                    reactor.clone(),
                    PortDirection::Output,
                    None,
                    0,
                    None,
                )
                .unwrap();
        }

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn gapped_port_declaration_positions_are_rejected() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        for (name, position) in [("first", 0), ("second", 2)] {
            builder
                .add_port(
                    id(&format!("vehicle/root/{name}")),
                    reactor.clone(),
                    PortDirection::Output,
                    None,
                    position,
                    None,
                )
                .unwrap();
        }

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn action_and_port_positions_form_independent_sequences() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        for (name, position) in [("a0", 0), ("a1", 1)] {
            builder
                .add_action(
                    id(&format!("vehicle/root/{name}")),
                    reactor.clone(),
                    ActionKind::Logical {
                        minimum_delay: None,
                    },
                    position,
                    None,
                )
                .unwrap();
        }
        for (name, position) in [("p0", 0), ("p1", 1)] {
            builder
                .add_port(
                    id(&format!("vehicle/root/{name}")),
                    reactor.clone(),
                    PortDirection::Output,
                    None,
                    position,
                    None,
                )
                .unwrap();
        }

        assert!(builder.finish().is_ok());
    }

    #[test]
    fn component_identity_rejects_terminal_and_internal_typed_segments() {
        for component in ["vehicle/#b0", "vehicle/#g0/sensor"] {
            let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
            builder
                .add_component(ComponentInstance::new(component, "sensor.v1", 1).unwrap())
                .unwrap();

            assert!(matches!(
                builder.finish(),
                Err(TopologyBuildError::InvalidOwnership { .. })
            ));
        }
    }

    #[test]
    fn root_placement_group_identity_rejects_typed_segments() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        builder
            .add_placement_group(id("placement/#b0"), None)
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn nested_placement_group_identity_rejects_internal_typed_segments() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        let parent: PlacementGroupId = id("placement/#g0");
        builder.add_placement_group(parent.clone(), None).unwrap();
        builder
            .add_placement_group(id("placement/#g0/child"), Some(parent))
            .unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    fn topology_with_connection(boundary: &str) -> ApplicationTopologyBuilder {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/root");
        let source: PortId = id("vehicle/root/source");
        let target: PortId = id("vehicle/root/target");
        builder
            .add_port(
                source.clone(),
                reactor.clone(),
                PortDirection::Output,
                None,
                0,
                None,
            )
            .unwrap();
        builder
            .add_port(target.clone(), reactor, PortDirection::Input, None, 1, None)
            .unwrap();
        builder
            .add_connection(
                id(boundary),
                source,
                target,
                ConnectionSemantics::Logical { after: None },
            )
            .unwrap();
        builder
    }

    #[test]
    fn boundary_identity_rejects_bank_segments() {
        assert!(matches!(
            topology_with_connection("vehicle/#b0").finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn boundary_identity_rejects_generated_segments() {
        assert!(matches!(
            topology_with_connection("vehicle/#g0/link").finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn enclave_identity_must_match_its_root_reactor_path() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        let component: ComponentInstanceId = id("vehicle");
        let root: ReactorId = id("vehicle/root");
        let enclave: StableEnclaveId = id("vehicle/enclave");
        builder
            .add_component(ComponentInstance::new("vehicle", "vehicle.v1", 1).unwrap())
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                root.clone(),
                component,
                None,
                None,
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder.add_enclave(enclave, root).unwrap();

        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn banked_enclave_root_uses_the_exact_reactor_path() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        let component: ComponentInstanceId = id("vehicle");
        let root: ReactorId = id("vehicle/sensor/#b0");
        let enclave: StableEnclaveId = id("vehicle/sensor/#b0");
        builder
            .add_component(ComponentInstance::new("vehicle", "sensor.v1", 1).unwrap())
            .unwrap();
        builder
            .add_reactor(Reactor::new(
                root.clone(),
                component,
                None,
                Some(crate::compiler::BankMember::new(0, 1).unwrap()),
                enclave.clone(),
                None,
                None,
            ))
            .unwrap();
        builder.add_enclave(enclave, root).unwrap();

        assert!(builder.finish().is_ok());
    }
}
