use crate::{
    key_set::KeySetLimits, Action, ActionKey, BasePort, PortKey, Reaction, ReactionKey, Reactor,
    ReactorKey,
};

mod debug;

/// Execution level
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Level(pub(crate) usize);

impl std::fmt::Debug for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "L{}", self.0)
    }
}

impl From<usize> for Level {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl std::ops::Add<usize> for Level {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl std::ops::AddAssign<usize> for Level {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl std::ops::Sub<usize> for Level {
    type Output = Self;

    fn sub(self, rhs: usize) -> Self::Output {
        Self(self.0 - rhs)
    }
}

/// A paired `ReactionKey` with it's execution `Level`.
pub type LevelReactionKey = (Level, ReactionKey);

/// `Env` stores the resolved runtime state of all the reactors.
///
/// The reactor heirarchy has been flattened and build by the builder methods.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Env {
    /// The runtime set of Reactors
    pub reactors: tinymap::TinyMap<ReactorKey, Reactor>,
    /// The runtime set of Actions
    pub actions: tinymap::TinyMap<ActionKey, Action>,
    /// The runtime set of Ports
    #[serde(skip)]
    pub ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
    /// The runtime set of Reactions
    #[serde(skip)]
    pub reactions: tinymap::TinyMap<ReactionKey, Reaction>,
}

impl Env {
    /// Get a reactor by it's name
    pub fn find_reactor_by_name(&self, name: &str) -> Option<&Reactor> {
        self.reactors
            .iter()
            .find(|(_, reactor)| reactor.get_name() == name)
            .map(|(_, reactor)| reactor)
    }
}

/// Bank information for a multi-bank reactor
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BankInfo {
    /// The index of this reactor within the bank
    pub idx: usize,
    /// The total number of reactors in the bank
    pub total: usize,
}

/// Invariant data for the runtime, describing the resolved reaction graph and it's dependencies.
///
/// Maps of triggers for actions and ports. This data is statically resolved by the builder from the reaction graph.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReactionGraph {
    /// For each Action, a set of Reactions triggered by it.
    pub action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
    /// Global port triggers
    pub port_triggers: tinymap::TinySecondaryMap<PortKey, Vec<LevelReactionKey>>,
    /// Global startup reactions
    pub startup_reactions: Vec<LevelReactionKey>,
    /// Global shutdown reactions
    pub shutdown_reactions: Vec<LevelReactionKey>,
    /// The maximum level of any reaction, and the total number of reactions. This is used to allocate the reaction set.
    pub reaction_set_limits: KeySetLimits,
    /// For each reaction, the set of 'use' ports
    pub reaction_use_ports: tinymap::TinySecondaryMap<ReactionKey, tinymap::KeySet<PortKey>>,
    /// For each reaction, the set of 'effect' ports
    pub reaction_effect_ports: tinymap::TinySecondaryMap<ReactionKey, tinymap::KeySet<PortKey>>,
    /// For each reaction, the set of 'use/effect' actions
    pub reaction_actions: tinymap::TinySecondaryMap<ReactionKey, tinymap::KeySet<ActionKey>>,
    /// For each reaction, the reactor it belongs to
    pub reaction_reactors: tinymap::TinySecondaryMap<ReactionKey, ReactorKey>,
    /// Bank index for a multi-bank reactor
    pub reactor_bank_infos: tinymap::TinySecondaryMap<ReactorKey, Option<BankInfo>>,
}

#[cfg(test)]
pub mod tests {
    use itertools::Itertools;

    use crate::{Context, Port, ReactionSetLimits, ReactorState};

    use super::*;

    /// An empty reaction function for testing.
    pub fn dummy_reaction_fn<'a>(
        _context: &'a mut Context,
        _state: &'a mut dyn ReactorState,
        _ref_ports: crate::refs::Refs<'a, dyn BasePort>,
        _mut_ports: crate::refs::RefsMut<'a, dyn BasePort>,
        _actions: crate::refs::RefsMut<'a, Action>,
    ) {
    }

    /// Create a dummy `Env` and `ReactionGraph` for testing.
    pub fn create_dummy_env() -> (Env, ReactionGraph) {
        let env = Env {
            reactors: [Reactor::new("dummy", Box::new(()))].into_iter().collect(),
            reactions: [Reaction::new("dummy", Box::new(dummy_reaction_fn), None)]
                .into_iter()
                .collect(),
            actions: [
                Action::new_logical::<()>("action0", ActionKey::from(0), Default::default()),
                Action::new_logical::<()>("action1", ActionKey::from(1), Default::default()),
            ]
            .into_iter()
            .collect(),
            ports: [
                Port::<u32>::new("port0", PortKey::from(0)).boxed(),
                Port::<u32>::new("port1", PortKey::from(1)).boxed(),
            ]
            .into_iter()
            .collect(),
        };

        let reactor_key = env.reactors.keys().next().unwrap();
        let reaction_key = env.reactions.keys().next().unwrap();
        let action_keys = env.actions.keys().collect_vec();
        let port_keys = env.ports.keys().collect_vec();

        let reaction_graph = ReactionGraph {
            action_triggers: tinymap::TinySecondaryMap::new(),
            port_triggers: tinymap::TinySecondaryMap::new(),
            startup_reactions: Vec::new(),
            shutdown_reactions: Vec::new(),
            reaction_set_limits: ReactionSetLimits {
                max_level: 0.into(),
                num_keys: 0,
            },
            reaction_use_ports: [(reaction_key, std::iter::once(port_keys[0]).collect())]
                .into_iter()
                .collect(),
            reaction_effect_ports: [(reaction_key, std::iter::once(port_keys[1]).collect())]
                .into_iter()
                .collect(),
            reaction_actions: [(reaction_key, action_keys.into_iter().collect())]
                .into_iter()
                .collect(),
            reaction_reactors: [(reaction_key, reactor_key)].into_iter().collect(),
            reactor_bank_infos: tinymap::TinySecondaryMap::new(),
        };
        (env, reaction_graph)
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serialize_roundtrip() {
        let (env, reaction_graph) = create_dummy_env();

        let serialized_env = serde_json::to_string_pretty(&env).unwrap();
        println!("{serialized_env}");
        //let deserialized_env: Env = serde_json::from_str(&serialized_env).unwrap();
        //assert_eq!(env, deserialized_env);

        let serialized_reaction_graph = serde_json::to_string_pretty(&reaction_graph).unwrap();
        println!("{serialized_reaction_graph}");
        let deserialized_reaction_graph: ReactionGraph =
            serde_json::from_str(&serialized_reaction_graph).unwrap();
        //assert_eq!(reaction_graph, deserialized_reaction_graph);
    }
}
