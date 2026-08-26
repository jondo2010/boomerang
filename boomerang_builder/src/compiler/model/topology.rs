use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use super::{
    action::{Action, ActionKind},
    component::ComponentInstance,
    connection::Connection,
    enclave::Enclave,
    mode::Mode,
    placement_group::PlacementGroup,
    port::{Port, PortDirection},
    reaction::{Reaction, ReactionOptions, ReactionRelation, ReactionRelationTarget},
    reactor::Reactor,
};
use crate::compiler::{
    ActionId, ApplicationId, BoundaryId, ComponentInstanceId, InvalidStableId, ModeId,
    PlacementGroupId, PortId, ReactionId, ReactorId, StableEnclaveId, StablePath,
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
    pub fn add_reactor(
        &mut self,
        id: ReactorId,
        component: ComponentInstanceId,
        parent: Option<ReactorId>,
        enclave: StableEnclaveId,
        placement_group: Option<PlacementGroupId>,
        scope_mode: Option<ModeId>,
    ) -> Result<(), TopologyBuildError> {
        let reactor = Reactor {
            id: id.clone(),
            component,
            parent,
            enclave,
            placement_group,
            scope_mode,
        };
        insert_unique!(self.reactors, id, reactor, "reactor")
    }

    /// Stages one action declaration.
    pub fn add_action(
        &mut self,
        id: ActionId,
        reactor: ReactorId,
        kind: ActionKind,
        mode: Option<ModeId>,
    ) -> Result<(), TopologyBuildError> {
        let action = Action {
            id: id.clone(),
            reactor,
            kind,
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
        mode: Option<ModeId>,
    ) -> Result<(), TopologyBuildError> {
        let port = Port {
            id: id.clone(),
            reactor,
            direction,
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
    ) -> Result<(), TopologyBuildError> {
        let connection = Connection {
            id: id.clone(),
            source,
            target,
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

fn validate_child(
    child: &(impl std::ops::Deref<Target = StablePath> + fmt::Display),
    owner: &StablePath,
    kind: &'static str,
) -> Result<(), TopologyBuildError> {
    if child.parent().as_ref() == Some(owner) {
        Ok(())
    } else {
        Err(invalid_ownership(
            child,
            format!("{kind} path is not an immediate child of declared owner '{owner}'"),
        ))
    }
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
    validate_connections(&builder.connections, &builder.ports)?;
    validate_enclaves(&builder.enclaves, &builder.reactors)?;
    validate_reactions(
        &builder.reactions,
        &builder.reactors,
        &builder.actions,
        &builder.ports,
        &builder.modes,
    )
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
        if mode.id.path().parent().as_ref() != Some(expected_parent) {
            return Err(invalid_modal(
                &mode.id,
                format!(
                    "mode path is not an immediate child of declared owner '{expected_parent}'"
                ),
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
    for reactor in reactors.values() {
        require(components, &reactor.component, &reactor.id, "component")?;
        let owner = reactor
            .parent
            .as_ref()
            .map(|id| id.path())
            .unwrap_or(reactor.component.path());
        validate_child(&reactor.id, owner, "reactor")?;
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

    for reactor in reactors.values() {
        let enclave = require(enclaves, &reactor.enclave, &reactor.id, "enclave")?;
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
            cursor = current.parent.as_ref().ok_or_else(|| {
                invalid_ownership(&reactor.id, "reactor is disconnected from its enclave root")
            })?;
        }
    }
    Ok(())
}

fn validate_placement_groups(
    groups: &BTreeMap<PlacementGroupId, PlacementGroup>,
) -> Result<(), TopologyBuildError> {
    for group in groups.values() {
        if let Some(parent) = &group.parent {
            validate_child(&group.id, parent.path(), "placement group")?;
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
    for action in actions.values() {
        validate_child(&action.id, action.reactor.path(), "action")?;
        require(reactors, &action.reactor, &action.id, "reactor")?;
        validate_mode_owner(action.mode.as_ref(), &action.reactor, modes, &action.id)?;
    }
    Ok(())
}

fn validate_ports(
    ports: &BTreeMap<PortId, Port>,
    reactors: &BTreeMap<ReactorId, Reactor>,
    modes: &BTreeMap<ModeId, Mode>,
) -> Result<(), TopologyBuildError> {
    for port in ports.values() {
        validate_child(&port.id, port.reactor.path(), "port")?;
        require(reactors, &port.reactor, &port.id, "reactor")?;
        validate_mode_owner(port.mode.as_ref(), &port.reactor, modes, &port.id)?;
    }
    Ok(())
}

fn validate_connections(
    connections: &BTreeMap<BoundaryId, Connection>,
    ports: &BTreeMap<PortId, Port>,
) -> Result<(), TopologyBuildError> {
    for connection in connections.values() {
        let source = require(ports, &connection.source, &connection.id, "port")?;
        let target = require(ports, &connection.target, &connection.id, "port")?;
        if source.direction != PortDirection::Output || target.direction != PortDirection::Input {
            return Err(TopologyBuildError::InvalidConnection {
                connection: connection.id.clone(),
                reason: "source must be output and target must be input",
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
    for reaction in reactions.values() {
        validate_child(&reaction.id, reaction.reactor.path(), "reaction")?;
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

impl fmt::Debug for ApplicationTopology {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationTopology")
            .field("application_id", &self.application_id)
            .field(
                "components",
                &self
                    .components
                    .keys()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
            .field(
                "reactors",
                &self
                    .reactors
                    .keys()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
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
            .add_component(ComponentInstance::new(component.to_string(), "contract").unwrap())
            .unwrap();
        builder
            .add_reactor(root.clone(), component, None, enclave.clone(), None, None)
            .unwrap();
        builder.add_enclave(enclave.clone(), root.clone()).unwrap();
        (builder, root, enclave)
    }

    #[test]
    fn separate_action_and_port_positions_finalize_and_preserve_relations() {
        let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/enclave");
        let a0: ActionId = id("vehicle/root/a0");
        let a1: ActionId = id("vehicle/root/a1");
        let p0: PortId = id("vehicle/root/p0");
        let p1: PortId = id("vehicle/root/p1");
        builder
            .add_action(a0.clone(), reactor.clone(), ActionKind::Logical, None)
            .unwrap();
        builder
            .add_action(a1.clone(), reactor.clone(), ActionKind::Logical, None)
            .unwrap();
        builder
            .add_port(p0.clone(), reactor.clone(), PortDirection::Input, None)
            .unwrap();
        builder
            .add_port(p1.clone(), reactor.clone(), PortDirection::Output, None)
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
            let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/enclave");
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
                .add_action(a0.clone(), reactor.clone(), ActionKind::Logical, None)
                .unwrap();
            builder
                .add_action(a1.clone(), reactor.clone(), ActionKind::Logical, None)
                .unwrap();
            builder
                .add_port(p0.clone(), reactor.clone(), PortDirection::Input, None)
                .unwrap();
            builder
                .add_port(p1.clone(), reactor.clone(), PortDirection::Output, None)
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
            let (mut builder, reactor, _) = base("vehicle", "vehicle/root", "vehicle/enclave");
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
        let (mut builder, root, enclave) = base("vehicle/a", "vehicle/a/root", "vehicle/enclave");
        let other: ComponentInstanceId = id("vehicle/b");
        builder
            .add_component(ComponentInstance::new("vehicle/b", "contract").unwrap())
            .unwrap();
        builder
            .add_reactor(
                id("vehicle/a/root/child"),
                other,
                Some(root),
                enclave,
                None,
                None,
            )
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
        let (mut builder, _, enclave) = base("vehicle", "vehicle/root", "vehicle/enclave");
        let a: ReactorId = id("vehicle/root/a");
        let b: ReactorId = id("vehicle/root/a/b");
        builder
            .add_reactor(
                a.clone(),
                id("vehicle"),
                Some(b.clone()),
                enclave.clone(),
                None,
                None,
            )
            .unwrap();
        builder
            .add_reactor(b, id("vehicle"), Some(a), enclave, None, None)
            .unwrap();
        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn reactor_scope_mode_must_belong_to_its_structural_parent() {
        let (mut builder, root, enclave) = base("vehicle", "vehicle/root", "vehicle/enclave");
        let mode: ModeId = id("vehicle/root/mode");
        builder
            .add_mode(mode.clone(), root.clone(), None, true)
            .unwrap();
        builder
            .add_reactor(
                id("vehicle/root/child"),
                id("vehicle"),
                Some(root),
                enclave,
                None,
                Some(mode),
            )
            .unwrap();
        assert!(builder.finish().is_ok());

        let (mut child_owned, root, enclave) = base("vehicle", "vehicle/root", "vehicle/enclave");
        let child: ReactorId = id("vehicle/root/child");
        let mode: ModeId = id("vehicle/root/child/mode");
        child_owned
            .add_mode(mode.clone(), child.clone(), None, true)
            .unwrap();
        child_owned
            .add_reactor(child, id("vehicle"), Some(root), enclave, None, Some(mode))
            .unwrap();
        assert!(matches!(
            child_owned.finish(),
            Err(TopologyBuildError::InvalidModalStructure { .. })
        ));

        let (mut unrelated, root, enclave) = base("vehicle", "vehicle/root", "vehicle/enclave");
        let sibling: ReactorId = id("vehicle/root/sibling");
        let mode: ModeId = id("vehicle/root/sibling/mode");
        unrelated
            .add_mode(mode.clone(), sibling.clone(), None, true)
            .unwrap();
        unrelated
            .add_reactor(
                sibling,
                id("vehicle"),
                Some(root.clone()),
                enclave.clone(),
                None,
                None,
            )
            .unwrap();
        unrelated
            .add_reactor(
                id("vehicle/root/child"),
                id("vehicle"),
                Some(root),
                enclave,
                None,
                Some(mode),
            )
            .unwrap();
        assert!(matches!(
            unrelated.finish(),
            Err(TopologyBuildError::InvalidModalStructure { .. })
        ));
    }

    #[test]
    fn mode_paths_must_be_immediate_children_of_their_declared_parent() {
        let (mut root_case, root, _) = base("vehicle", "vehicle/root", "vehicle/enclave");
        root_case
            .add_mode(id("vehicle/root/deep/mode"), root, None, true)
            .unwrap();
        assert!(matches!(
            root_case.finish(),
            Err(TopologyBuildError::InvalidModalStructure { reason, .. })
                if reason.contains("vehicle/root")
        ));

        let (mut child_case, root, _) = base("vehicle", "vehicle/root", "vehicle/enclave");
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
    fn enclave_rejects_a_disconnected_parentless_reactor() {
        let (mut builder, _, enclave) = base("vehicle", "vehicle/root", "vehicle/enclave");
        builder
            .add_reactor(
                id("vehicle/other"),
                id("vehicle"),
                None,
                enclave,
                None,
                None,
            )
            .unwrap();
        assert!(matches!(
            builder.finish(),
            Err(TopologyBuildError::InvalidOwnership { .. })
        ));
    }

    #[test]
    fn nested_enclave_root_retains_its_structural_parent() {
        let (mut builder, root, _) = base("vehicle", "vehicle/root", "vehicle/outer");
        let nested: ReactorId = id("vehicle/root/nested");
        let enclave: StableEnclaveId = id("vehicle/nested");
        builder
            .add_reactor(
                nested.clone(),
                id("vehicle"),
                Some(root),
                enclave.clone(),
                None,
                None,
            )
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
                    .add_component(ComponentInstance::new(id, "sensor.v1").unwrap())
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
}
