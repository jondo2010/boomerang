//! Deterministic structural inspection for target-neutral application topologies.

use std::{collections::BTreeMap, fmt};

use petgraph::prelude::DiGraphMap;

use super::{
    Action, ApplicationTopology, BankMember, Connection, Enclave, Mode, PlacementGroup, Port,
    PortId, Reaction, ReactionRelation, ReactionRelationTarget, Reactor, ReactorId, StablePath,
    StablePathSegment,
};
/// Returns the stable family path for a scalar declaration or validated bank member.
fn bank_family(path: &StablePath, bank: Option<BankMember>) -> StablePath {
    match bank {
        Some(member) => {
            assert!(
                matches!(path.segments().last(), Some(StablePathSegment::BankIndex(index)) if *index == member.index()),
                "validated bank member must end in its typed bank-index segment"
            );
            path.parent()
                .expect("validated bank member must have a stable family path")
        }
        None => path.clone(),
    }
}

/// One scalar declaration or explicit typed bank family.
struct DebugGroup<'a, Id> {
    /// Stable identities in canonical member order.
    members: Vec<&'a Id>,
    /// Whether the declarations carry typed bank metadata.
    bank: bool,
}

impl<'a, Id> DebugGroup<'a, Id> {
    /// Returns the first identity and the last identity for an explicit bank.
    fn range(&self) -> (&'a Id, Option<&'a Id>) {
        let first = self.members[0];
        let last = self.bank.then(|| self.members[self.members.len() - 1]);
        (first, last)
    }
}

/// Groups validated stable identities by scalar declaration or typed bank family.
fn grouped<'a, Id>(
    declarations: impl Iterator<Item = (&'a Id, &'a StablePath, Option<BankMember>)>,
) -> Vec<DebugGroup<'a, Id>> {
    let mut groups = BTreeMap::<StablePath, DebugGroup<Id>>::new();
    for (id, path, bank) in declarations {
        let is_bank = bank.is_some();
        let group = groups
            .entry(bank_family(path, bank))
            .or_insert_with(|| DebugGroup {
                members: Vec::new(),
                bank: is_bank,
            });
        debug_assert_eq!(group.bank, is_bank);
        group.members.push(id);
    }
    groups.into_values().collect()
}

impl ApplicationTopology {
    /// Returns stable reactor ranges grouped by explicit typed bank segments.
    ///
    /// Scalar declarations have no last identity. Bank declarations return their canonical first
    /// and last member identities without exposing any runtime or construction-order key. A
    /// singleton bank returns its sole identity as both first and last.
    pub fn reactors_debug_grouped(&self) -> Vec<(&ReactorId, Option<&ReactorId>)> {
        grouped(
            self.reactors()
                .map(|(id, reactor)| (id, id.path(), reactor.bank())),
        )
        .into_iter()
        .map(|group| group.range())
        .collect()
    }

    /// Returns stable port ranges grouped by explicit typed bank segments.
    ///
    /// A singleton bank returns its sole identity as both first and last.
    pub fn ports_debug_grouped(&self) -> Vec<(&PortId, Option<&PortId>)> {
        grouped(self.ports().map(|(id, port)| (id, id.path(), port.bank())))
            .into_iter()
            .map(|group| group.range())
            .collect()
    }

    /// Builds the structural parent graph using one stable representative per reactor bank.
    pub fn build_reactor_graph_grouped(&self) -> DiGraphMap<&ReactorId, ()> {
        let groups = grouped(
            self.reactors()
                .map(|(id, reactor)| (id, id.path(), reactor.bank())),
        );
        let mut representatives = BTreeMap::new();
        for group in &groups {
            let representative = group.members[0];
            for member in &group.members {
                representatives.insert(*member, representative);
            }
        }

        let mut graph = DiGraphMap::new();
        for group in groups {
            let representative = group.members[0];
            graph.add_node(representative);
            if let Some(parent) = self
                .reactor(representative)
                .expect("grouped reactor identity must resolve")
                .parent()
            {
                graph.add_edge(representatives[parent], representative, ());
            }
        }
        graph
    }
}

/// Formats one canonical stable identity using its public text representation.
struct Canonical<'a, T>(
    /// Stable identity rendered canonically.
    &'a T,
);

impl<T: fmt::Display> fmt::Debug for Canonical<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.to_string().fmt(formatter)
    }
}

/// Formats a scalar identity or a canonical first-to-last bank range.
struct CanonicalRange<'a, T> {
    /// First or only stable identity.
    first: &'a T,
    /// Last stable identity when this is a bank.
    last: Option<&'a T>,
}

impl<T: fmt::Display> fmt::Debug for CanonicalRange<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.last {
            Some(last) => format!("{}..{last}", self.first).fmt(formatter),
            None => self.first.to_string().fmt(formatter),
        }
    }
}

/// Formats validated bank metadata as a stable member marker.
fn bank_member(member: Option<BankMember>) -> Option<String> {
    member.map(|member| format!("#b{} of {}", member.index(), member.total()))
}

/// Structural reactor fields rendered through stable identities.
struct ReactorDebug<'a>(
    /// Structural reactor record.
    &'a Reactor,
);

impl fmt::Debug for ReactorDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Reactor")
            .field("component", &Canonical(self.0.component()))
            .field("parent", &self.0.parent().map(ToString::to_string))
            .field("bank", &bank_member(self.0.bank()))
            .field("enclave", &Canonical(self.0.enclave()))
            .field(
                "placement_group",
                &self.0.placement_group().map(ToString::to_string),
            )
            .field("scope_mode", &self.0.scope_mode().map(ToString::to_string))
            .finish()
    }
}

/// Renders a scalar directly and every bank member in canonical order.
struct GroupedValues<T> {
    /// Structural records in stable member order.
    values: Vec<T>,
    /// Whether the records form an explicit typed bank.
    bank: bool,
}

impl<T: fmt::Debug> fmt::Debug for GroupedValues<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.bank {
            if let [value] = self.values.as_slice() {
                return value.fmt(formatter);
            }
        }
        formatter.debug_list().entries(&self.values).finish()
    }
}

/// Structural action fields rendered through stable identities.
struct ActionDebug<'a>(
    /// Structural action record.
    &'a Action,
);

impl fmt::Debug for ActionDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Action")
            .field("reactor", &Canonical(self.0.reactor()))
            .field("kind", &self.0.kind())
            .field("declaration_position", &self.0.declaration_position())
            .field("mode", &self.0.mode().map(ToString::to_string))
            .finish()
    }
}

/// Structural port fields rendered through stable identities.
struct PortDebug<'a>(
    /// Structural port record.
    &'a Port,
);

impl fmt::Debug for PortDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Port")
            .field("reactor", &Canonical(self.0.reactor()))
            .field("direction", &self.0.direction())
            .field("bank", &bank_member(self.0.bank()))
            .field("declaration_position", &self.0.declaration_position())
            .field("mode", &self.0.mode().map(ToString::to_string))
            .finish()
    }
}

/// Stable reaction target and relation flags.
struct RelationDebug<'a>(
    /// Structural reaction relation.
    &'a ReactionRelation,
);

impl fmt::Debug for RelationDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let target = match self.0.target() {
            ReactionRelationTarget::Action(id) => format!("action:{id}"),
            ReactionRelationTarget::Port(id) => format!("port:{id}"),
        };
        let flags = self.0.flags();
        let mut enabled = Vec::new();
        if flags.is_trigger() {
            enabled.push("trigger");
        }
        if flags.is_use() {
            enabled.push("use");
        }
        if flags.is_effect() {
            enabled.push("effect");
        }
        formatter
            .debug_struct("ReactionRelation")
            .field("target", &target)
            .field("flags", &enabled)
            .field("declaration_position", &self.0.declaration_position())
            .finish()
    }
}

/// Structural reaction fields rendered through stable identities.
struct ReactionDebug<'a>(
    /// Structural reaction record.
    &'a Reaction,
);

impl fmt::Debug for ReactionDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let relations = self
            .0
            .relations()
            .iter()
            .map(RelationDebug)
            .collect::<Vec<_>>();
        let options = self.0.options();
        let transition = options
            .transition()
            .map(|transition| (transition.target().to_string(), transition.kind()));
        formatter
            .debug_struct("Reaction")
            .field("reactor", &Canonical(self.0.reactor()))
            .field("relations", &relations)
            .field("mode", &options.mode().map(ToString::to_string))
            .field(
                "enabled_modes",
                &options
                    .enabled_modes()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
            .field(
                "reset_modes",
                &options
                    .reset_modes()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
            .field("transition", &transition)
            .finish()
    }
}

/// Structural connection fields rendered through stable identities.
struct ConnectionDebug<'a>(
    /// Structural connection record.
    &'a Connection,
);

impl fmt::Debug for ConnectionDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Connection")
            .field("source", &Canonical(self.0.source()))
            .field("target", &Canonical(self.0.target()))
            .field("semantics", &self.0.semantics())
            .finish()
    }
}

/// Structural mode fields rendered through stable identities.
struct ModeDebug<'a>(
    /// Structural mode record.
    &'a Mode,
);

impl fmt::Debug for ModeDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Mode")
            .field("reactor", &Canonical(self.0.reactor()))
            .field("parent", &self.0.parent().map(ToString::to_string))
            .field("initial", &self.0.is_initial())
            .finish()
    }
}

/// Structural Enclave fields rendered through stable identities.
struct EnclaveDebug<'a>(
    /// Structural Enclave record.
    &'a Enclave,
);

impl fmt::Debug for EnclaveDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Enclave")
            .field("root", &Canonical(self.0.root()))
            .finish()
    }
}

/// Structural placement-group fields rendered through stable identities.
struct PlacementGroupDebug<'a>(
    /// Structural placement-group record.
    &'a PlacementGroup,
);

impl fmt::Debug for PlacementGroupDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlacementGroup")
            .field("parent", &self.0.parent().map(ToString::to_string))
            .finish()
    }
}

impl fmt::Debug for ApplicationTopology {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let components = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    self.components()
                        .map(|(id, component)| (Canonical(id), component.contract().as_str())),
                )
                .finish()
        });
        let reactors = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    grouped(
                        self.reactors()
                            .map(|(id, reactor)| (id, id.path(), reactor.bank())),
                    )
                    .into_iter()
                    .map(|group| {
                        let (first, last) = group.range();
                        (
                            CanonicalRange { first, last },
                            GroupedValues {
                                bank: group.bank,
                                values: group
                                    .members
                                    .into_iter()
                                    .map(|id| {
                                        ReactorDebug(
                                            self.reactor(id)
                                                .expect("grouped reactor identity must resolve"),
                                        )
                                    })
                                    .collect(),
                            },
                        )
                    }),
                )
                .finish()
        });
        let actions = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    self.actions()
                        .map(|(id, action)| (Canonical(id), ActionDebug(action))),
                )
                .finish()
        });
        let ports = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    grouped(self.ports().map(|(id, port)| (id, id.path(), port.bank())))
                        .into_iter()
                        .map(|group| {
                            let (first, last) = group.range();
                            (
                                CanonicalRange { first, last },
                                GroupedValues {
                                    bank: group.bank,
                                    values: group
                                        .members
                                        .into_iter()
                                        .map(|id| {
                                            PortDebug(
                                                self.port(id)
                                                    .expect("grouped port identity must resolve"),
                                            )
                                        })
                                        .collect(),
                                },
                            )
                        }),
                )
                .finish()
        });
        let reactions = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    self.reactions()
                        .map(|(id, reaction)| (Canonical(id), ReactionDebug(reaction))),
                )
                .finish()
        });
        let connections = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    self.connections()
                        .map(|(id, connection)| (Canonical(id), ConnectionDebug(connection))),
                )
                .finish()
        });
        let modes = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    self.modes()
                        .map(|(id, mode)| (Canonical(id), ModeDebug(mode))),
                )
                .finish()
        });
        let enclaves = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    self.enclaves()
                        .map(|(id, enclave)| (Canonical(id), EnclaveDebug(enclave))),
                )
                .finish()
        });
        let placement_groups = crate::runtime::fmt_utils::from_fn(|formatter| {
            formatter
                .debug_map()
                .entries(
                    self.placement_groups()
                        .map(|(id, group)| (Canonical(id), PlacementGroupDebug(group))),
                )
                .finish()
        });

        formatter
            .debug_struct("ApplicationTopology")
            .field("application_id", &self.application_id().as_str())
            .field("components", &components)
            .field("reactors", &reactors)
            .field("actions", &actions)
            .field("ports", &ports)
            .field("reactions", &reactions)
            .field("connections", &connections)
            .field("modes", &modes)
            .field("enclaves", &enclaves)
            .field("placement_groups", &placement_groups)
            .finish()
    }
}
