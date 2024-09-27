use crate::{
    key_set::KeySetLimits, Action, ActionKey, BasePort, PortKey, Reaction, ReactionKey, Reactor,
    ReactorKey,
};

mod debug;
mod inner;

pub(crate) use inner::{ReactionTriggerCtx, Store};

/// Execution level
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Level(pub(crate) usize);

impl std::fmt::Display for Level {
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
pub struct Env {
    /// The runtime set of Reactors
    pub reactors: tinymap::TinyMap<ReactorKey, Reactor>,
    /// The runtime set of Actions
    pub actions: tinymap::TinyMap<ActionKey, Action>,
    /// The runtime set of Ports
    pub ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
    /// The runtime set of Reactions
    pub reactions: tinymap::TinyMap<ReactionKey, Reaction>,
}

impl Env {
    /// Get a reactor by it's name
    pub fn get_reactor_by_name(&self, name: &str) -> Option<&Reactor> {
        self.reactors
            .iter()
            .find(|(_, reactor)| reactor.name == name)
            .map(|(_, reactor)| reactor)
    }
}

/// Bank information for a multi-bank reactor
#[derive(Debug, Clone)]
pub struct BankInfo {
    /// The index of this reactor within the bank
    pub idx: usize,
    /// The total number of reactors in the bank
    pub total: usize,
}

/// Invariant data for the runtime, describing the resolved reaction graph and it's dependencies.
///
/// Maps of triggers for actions and ports. This data is statically resolved by the builder from the reaction graph.
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
    pub reaction_use_ports:
        tinymap::TinySecondaryMap<ReactionKey, tinymap::TinySecondarySet<PortKey>>,
    /// For each reaction, the set of 'effect' ports
    pub reaction_effect_ports:
        tinymap::TinySecondaryMap<ReactionKey, tinymap::TinySecondarySet<PortKey>>,
    /// For each reaction, the set of 'use/effect' actions
    pub reaction_actions:
        tinymap::TinySecondaryMap<ReactionKey, tinymap::TinySecondarySet<ActionKey>>,
    /// For each reaction, the reactor it belongs to
    pub reaction_reactors: tinymap::TinySecondaryMap<ReactionKey, ReactorKey>,
    /// Bank index for a multi-bank reactor
    pub reactor_bank_infos: tinymap::TinySecondaryMap<ReactorKey, Option<BankInfo>>,
}
