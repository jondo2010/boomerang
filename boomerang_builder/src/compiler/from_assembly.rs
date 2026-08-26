use std::collections::{BTreeMap, HashMap};

use slotmap::SecondaryMap;

use super::{
    ActionId, ActionKind, ApplicationTopology, ApplicationTopologyBuilder, BankMember, BoundaryId,
    ComponentInstance, ComponentInstanceId, ConnectionSemantics, ContractId, PlacementGroupId,
    PortDirection, PortId, ReactionId, ReactionOptions, ReactionRelation, ReactionRelationFlags,
    ReactionRelationTarget, Reactor, ReactorId, StableEnclaveId, StablePath, StablePathSegment,
};
use crate::{
    ActionType, Assembly, AssemblyActionKey, AssemblyError, AssemblyPortKey, AssemblyReactorKey,
    PortType, TimerSpec, TriggerMode,
};

impl Assembly {
    /// Projects this assembly into an immutable non-modal application topology.
    pub fn application_topology(&self) -> Result<ApplicationTopology, AssemblyError> {
        project_application_topology(self)
    }
}

fn projection_error(error: impl std::fmt::Display) -> AssemblyError {
    AssemblyError::InternalError(format!("application topology projection: {error}"))
}

fn checked_u32(value: usize, what: &str) -> Result<u32, AssemblyError> {
    value
        .try_into()
        .map_err(|_| projection_error(format!("{what} exceeds u32")))
}

fn bank_member(
    bank: Option<&crate::runtime::BankInfo>,
) -> Result<Option<BankMember>, AssemblyError> {
    bank.map(|bank| {
        BankMember::new(
            checked_u32(bank.idx, "bank index")?,
            checked_u32(bank.total, "bank total")?,
        )
        .map_err(projection_error)
    })
    .transpose()
}

fn reactor_path(assembly: &Assembly, key: AssemblyReactorKey) -> Result<StablePath, AssemblyError> {
    let mut ancestry = Vec::new();
    let mut cursor = Some(key);
    while let Some(reactor_key) = cursor {
        let reactor = &assembly.reactor_specs[reactor_key];
        ancestry.push(reactor_key);
        cursor = reactor.parent_reactor_key;
    }
    let mut segments = Vec::new();
    for reactor_key in ancestry.into_iter().rev() {
        let reactor = &assembly.reactor_specs[reactor_key];
        segments.push(StablePathSegment::Name(reactor.name().into()));
        if let Some(bank) = reactor.bank_info() {
            segments.push(StablePathSegment::BankIndex(checked_u32(
                bank.idx,
                "reactor bank index",
            )?));
        }
    }
    StablePath::from_segments(segments).map_err(projection_error)
}

fn name_only_path(
    category: &'static str,
    encoded_names: impl IntoIterator<Item = String>,
) -> Result<StablePath, AssemblyError> {
    StablePath::from_segments(
        std::iter::once(StablePathSegment::Name(category.into())).chain(
            encoded_names
                .into_iter()
                .map(|name| StablePathSegment::Name(name.into_boxed_str())),
        ),
    )
    .map_err(projection_error)
}

fn root_of(assembly: &Assembly, mut key: AssemblyReactorKey) -> AssemblyReactorKey {
    while let Some(parent) = assembly.reactor_specs[key].parent_reactor_key {
        key = parent;
    }
    key
}

fn relation_flags(mode: TriggerMode) -> ReactionRelationFlags {
    let mut flags = ReactionRelationFlags::TRIGGER;
    if !mode.is_triggers() {
        flags = ReactionRelationFlags::USE;
        if !mode.is_uses() {
            flags = ReactionRelationFlags::EFFECT;
        }
    } else {
        if mode.is_uses() {
            flags |= ReactionRelationFlags::USE;
        }
        if mode.is_effects() {
            flags |= ReactionRelationFlags::EFFECT;
        }
    }
    flags
}

fn project_application_topology(assembly: &Assembly) -> Result<ApplicationTopology, AssemblyError> {
    if assembly.reactor_specs.is_empty() {
        return Err(projection_error("assembly has no reactor roots"));
    }

    let mut reactor_paths = SecondaryMap::new();
    for reactor_key in assembly.reactor_specs.keys() {
        reactor_paths.insert(reactor_key, reactor_path(assembly, reactor_key)?);
    }
    let mut roots = assembly
        .reactor_specs
        .keys()
        .filter(|key| assembly.reactor_specs[*key].parent_reactor_key.is_none())
        .collect::<Vec<_>>();
    roots.sort_by_key(|key| reactor_paths[*key].clone());

    let application_path = StablePath::from_segments(
        std::iter::once(StablePathSegment::Name("application".into())).chain(roots.iter().map(
            |root| StablePathSegment::Name(reactor_paths[*root].to_string().into_boxed_str()),
        )),
    )
    .map_err(projection_error)?;
    let mut topology =
        ApplicationTopologyBuilder::new(application_path.to_string()).map_err(projection_error)?;

    let contract = ContractId::new("boomerang_builder::Assembly").map_err(projection_error)?;
    let mut components = SecondaryMap::new();
    for root in &roots {
        let component = ComponentInstanceId::from_path(name_only_path(
            "component",
            [reactor_paths[*root].to_string()],
        )?);
        topology
            .add_component(ComponentInstance::from_ids(
                component.clone(),
                contract.clone(),
            ))
            .map_err(projection_error)?;
        components.insert(*root, component);
    }

    let partition_map = assembly.build_partition_map();
    let mut partitions = BTreeMap::new();
    for partition in partition_map.values() {
        partitions.insert(reactor_paths[*partition].clone(), *partition);
    }
    let mut placement_groups = SecondaryMap::new();
    for (path, partition) in &partitions {
        let reactor = ReactorId::from_path(path.clone());
        let enclave = StableEnclaveId::from_path(path.clone());
        let placement =
            PlacementGroupId::from_path(name_only_path("placement", [path.to_string()])?);
        topology
            .add_enclave(enclave, reactor)
            .map_err(projection_error)?;
        topology
            .add_placement_group(placement.clone(), None)
            .map_err(projection_error)?;
        placement_groups.insert(*partition, placement);
    }

    for (key, spec) in &assembly.reactor_specs {
        let id = ReactorId::from_path(reactor_paths[key].clone());
        let parent = spec
            .parent_reactor_key
            .map(|parent| ReactorId::from_path(reactor_paths[parent].clone()));
        let root = root_of(assembly, key);
        let partition = partition_map[key];
        topology
            .add_reactor(Reactor::new(
                id,
                components[root].clone(),
                parent,
                bank_member(spec.bank_info())?,
                StableEnclaveId::from_path(reactor_paths[partition].clone()),
                Some(placement_groups[partition].clone()),
                None,
            ))
            .map_err(projection_error)?;
    }

    let mut action_ids = SecondaryMap::<AssemblyActionKey, ActionId>::new();
    let mut action_positions = HashMap::<AssemblyReactorKey, u32>::new();
    for (key, spec) in &assembly.action_specs {
        let reactor_key = spec.reactor_key();
        let position = action_positions.entry(reactor_key).or_default();
        let id = ActionId::from_path(
            reactor_paths[reactor_key]
                .append_name(spec.name())
                .map_err(projection_error)?,
        );
        let kind = match spec.r#type() {
            ActionType::Timer(timer)
                if timer == &TimerSpec::STARTUP && spec.name() == "__startup" =>
            {
                ActionKind::Startup
            }
            ActionType::Timer(timer) => ActionKind::Timer {
                offset: timer.offset,
                period: timer.period,
            },
            ActionType::Standard {
                is_logical,
                min_delay,
                ..
            } if *is_logical => ActionKind::Logical {
                minimum_delay: *min_delay,
            },
            ActionType::Standard { min_delay, .. } => ActionKind::Physical {
                minimum_delay: *min_delay,
            },
            ActionType::Shutdown => ActionKind::Shutdown,
        };
        topology
            .add_action(
                id.clone(),
                ReactorId::from_path(reactor_paths[reactor_key].clone()),
                kind,
                *position,
                None,
            )
            .map_err(projection_error)?;
        action_ids.insert(key, id);
        *position = position
            .checked_add(1)
            .ok_or_else(|| projection_error("action declaration position exceeds u32"))?;
    }

    let mut port_ids = SecondaryMap::<AssemblyPortKey, PortId>::new();
    let mut port_positions = HashMap::<AssemblyReactorKey, u32>::new();
    for (key, spec) in &assembly.port_specs {
        let reactor_key = spec.get_reactor_key();
        let position = port_positions.entry(reactor_key).or_default();
        let mut path = reactor_paths[reactor_key]
            .append_name(spec.name())
            .map_err(projection_error)?;
        if let Some(bank) = spec.bank_info() {
            path = path.append_bank_index(checked_u32(bank.idx, "port bank index")?);
        }
        let id = PortId::from_path(path);
        let direction = match spec.port_type() {
            PortType::Input => PortDirection::Input,
            PortType::Output => PortDirection::Output,
        };
        topology
            .add_port(
                id.clone(),
                ReactorId::from_path(reactor_paths[reactor_key].clone()),
                direction,
                bank_member(spec.bank_info())?,
                *position,
                None,
            )
            .map_err(projection_error)?;
        port_ids.insert(key, id);
        *position = position
            .checked_add(1)
            .ok_or_else(|| projection_error("port declaration position exceeds u32"))?;
    }

    let mut reaction_ordinals = HashMap::<(AssemblyReactorKey, Option<String>), u32>::new();
    for (_, spec) in &assembly.reaction_specs {
        let family = (spec.reactor_key, spec.name().map(str::to_owned));
        let ordinal = reaction_ordinals.entry(family).or_default();
        let owner = &reactor_paths[spec.reactor_key];
        let path = if let Some(name) = spec.name() {
            owner
                .append_name(name)
                .map_err(projection_error)?
                .append_generated_ordinal(*ordinal)
        } else {
            owner.append_generated_ordinal(*ordinal)
        };
        let id = ReactionId::from_path(path);
        let action_relations = spec
            .action_order
            .iter()
            .enumerate()
            .map(|(position, target)| {
                Ok(ReactionRelation::new(
                    ReactionRelationTarget::Action(action_ids[*target].clone()),
                    relation_flags(spec.action_relations[*target]),
                    checked_u32(position, "action relation position")?,
                ))
            });
        let port_relations = spec
            .port_order
            .iter()
            .enumerate()
            .map(|(position, target)| {
                Ok(ReactionRelation::new(
                    ReactionRelationTarget::Port(port_ids[*target].clone()),
                    relation_flags(spec.port_relations[*target]),
                    checked_u32(position, "port relation position")?,
                ))
            });
        let relations = action_relations
            .chain(port_relations)
            .collect::<Result<Vec<_>, AssemblyError>>()?;
        topology
            .add_reaction(
                id.clone(),
                ReactorId::from_path(owner.clone()),
                relations,
                ReactionOptions::default(),
            )
            .map_err(projection_error)?;
        *ordinal = ordinal
            .checked_add(1)
            .ok_or_else(|| projection_error("reaction ordinal exceeds u32"))?;
    }

    let mut connection_ordinals = HashMap::<(AssemblyPortKey, AssemblyPortKey), u32>::new();
    for connection in &assembly.connection_specs {
        let source_key = connection.source_key();
        let target_key = connection.target_key();
        let ordinal = connection_ordinals
            .entry((source_key, target_key))
            .or_default();
        let source = port_ids[source_key].clone();
        let target = port_ids[target_key].clone();
        let id = BoundaryId::from_path(name_only_path(
            "boundary",
            [
                source.to_canonical_string(),
                target.to_canonical_string(),
                format!("c{ordinal}"),
            ],
        )?);
        let semantics = if connection.physical() {
            ConnectionSemantics::Physical {
                after: connection.after(),
            }
        } else {
            ConnectionSemantics::Logical {
                after: connection.after(),
            }
        };
        topology
            .add_connection(id, source, target, semantics)
            .map_err(projection_error)?;
        *ordinal = ordinal
            .checked_add(1)
            .ok_or_else(|| projection_error("connection ordinal exceeds u32"))?;
    }

    topology.finish().map_err(projection_error)
}
