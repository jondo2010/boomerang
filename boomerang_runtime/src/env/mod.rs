//! Runtime environment data consumed by the scheduler.
//!
//! This module defines the resolved reactors, actions, ports, reactions, and graph metadata that
//! the runtime executes. Build-time lowering and construction of derived graph indexes belong in
//! `boomerang_builder`; the runtime treats [`ReactionGraph`] as ready-to-execute data.

use crate::{
    ActionKey, BaseAction, BasePort, BaseReactor, PortKey, Reaction, ReactionKey, ReactorKey, Tag,
};
use core::range::Range;

mod debug;
#[cfg(test)]
pub mod tests;

/// Execution level
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Level(pub(crate) usize);

impl std::fmt::Debug for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "L{}", self.0)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LifecycleReaction {
    pub reaction: LevelReactionKey,
    pub action: ActionKey,
}

tinymap::key_type! { pub ModeKey }
tinymap::key_type! { pub ScopeKey }

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Mode {
    pub name: String,
    pub parent: ReactorKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScopeInfo {
    pub parent: Option<ScopeKey>,
    pub reactor: ReactorKey,
    pub mode: Option<ModeKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransitionKind {
    Reset,
    History,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ModeFilter {
    modes: Vec<ModeKey>,
}

impl ModeFilter {
    pub fn new(modes: Vec<ModeKey>) -> Self {
        Self { modes }
    }

    pub fn allows(&self, current: Option<ModeKey>) -> bool {
        let Some(mode) = current else {
            return false;
        };
        self.modes.contains(&mode)
    }

    pub fn modes(&self) -> &[ModeKey] {
        &self.modes
    }
}

/// Flattened scheduler lookup tables derived from [`ReactionGraph`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ModalScheduleIndex {
    #[cfg_attr(feature = "serde", serde(with = "range_map_serde"))]
    pub scope_descendant_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
    pub scope_descendants: Vec<ScopeKey>,
    #[cfg_attr(feature = "serde", serde(with = "range_map_serde"))]
    pub scope_logical_action_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
    pub scope_logical_actions: Vec<ActionKey>,
    #[cfg_attr(feature = "serde", serde(with = "range_map_serde"))]
    pub scope_timer_startup_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
    pub scope_timer_startups: Vec<(ActionKey, Tag)>,
    #[cfg_attr(feature = "serde", serde(with = "range_map_serde"))]
    pub scope_reset_reaction_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
    pub scope_reset_reactions: Vec<LevelReactionKey>,
    #[cfg_attr(feature = "serde", serde(with = "range_map_serde"))]
    pub scope_startup_reaction_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
    pub scope_startup_reactions: Vec<LifecycleReaction>,
    pub all_shutdown_reactions: Vec<LifecycleReaction>,
    pub all_shutdown_actions_unique: Vec<ActionKey>,
}

impl Default for ModalScheduleIndex {
    fn default() -> Self {
        Self {
            scope_descendant_ranges: tinymap::TinySecondaryMap::new(),
            scope_descendants: Vec::new(),
            scope_logical_action_ranges: tinymap::TinySecondaryMap::new(),
            scope_logical_actions: Vec::new(),
            scope_timer_startup_ranges: tinymap::TinySecondaryMap::new(),
            scope_timer_startups: Vec::new(),
            scope_reset_reaction_ranges: tinymap::TinySecondaryMap::new(),
            scope_reset_reactions: Vec::new(),
            scope_startup_reaction_ranges: tinymap::TinySecondaryMap::new(),
            scope_startup_reactions: Vec::new(),
            all_shutdown_reactions: Vec::new(),
            all_shutdown_actions_unique: Vec::new(),
        }
    }
}

impl ModalScheduleIndex {
    pub fn scope_descendants(&self, scope: ScopeKey) -> &[ScopeKey] {
        &self.scope_descendants[self.scope_descendant_ranges[scope]]
    }

    pub fn scope_logical_actions(&self, scope: ScopeKey) -> &[ActionKey] {
        &self.scope_logical_actions[self.scope_logical_action_ranges[scope]]
    }

    pub fn scope_timer_startups(&self, scope: ScopeKey) -> &[(ActionKey, Tag)] {
        &self.scope_timer_startups[self.scope_timer_startup_ranges[scope]]
    }

    pub fn scope_reset_reactions(&self, scope: ScopeKey) -> &[LevelReactionKey] {
        &self.scope_reset_reactions[self.scope_reset_reaction_ranges[scope]]
    }

    pub fn scope_startup_reactions(&self, scope: ScopeKey) -> &[LifecycleReaction] {
        &self.scope_startup_reactions[self.scope_startup_reaction_ranges[scope]]
    }
}

#[cfg(feature = "serde")]
mod range_map_serde {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct SerializableRange {
        start: usize,
        end: usize,
    }

    pub fn serialize<K, S>(
        map: &tinymap::TinySecondaryMap<K, Range<usize>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        K: tinymap::Key + serde::Serialize,
        S: serde::Serializer,
    {
        let serializable = map
            .iter()
            .map(|(key, range)| {
                (
                    key,
                    SerializableRange {
                        start: range.start,
                        end: range.end,
                    },
                )
            })
            .collect::<tinymap::TinySecondaryMap<K, SerializableRange>>();
        serde::Serialize::serialize(&serializable, serializer)
    }

    pub fn deserialize<'de, K, D>(
        deserializer: D,
    ) -> Result<tinymap::TinySecondaryMap<K, Range<usize>>, D::Error>
    where
        K: tinymap::Key + serde::Deserialize<'de>,
        D: serde::Deserializer<'de>,
    {
        let serializable =
            <tinymap::TinySecondaryMap<K, SerializableRange> as serde::Deserialize>::deserialize(
                deserializer,
            )?;
        Ok(serializable
            .into_iter()
            .map(|(key, range)| {
                (
                    key,
                    Range {
                        start: range.start,
                        end: range.end,
                    },
                )
            })
            .collect())
    }
}

/// `Env` stores the resolved runtime state of all the reactors.
///
/// The reactor hierarchy has been flattened and built by the builder methods.
#[derive(Default)]
pub struct Env {
    /// The runtime set of Reactors
    pub reactors: tinymap::TinyMap<ReactorKey, Box<dyn BaseReactor>>,
    /// The runtime set of Actions
    pub actions: tinymap::TinyMap<ActionKey, Box<dyn BaseAction>>,
    /// The runtime set of Ports
    pub ports: tinymap::TinyMap<PortKey, Box<dyn BasePort>>,
    /// The runtime set of Reactions
    pub reactions: tinymap::TinyMap<ReactionKey, Reaction>,
}

impl Env {
    /// Get a reactor by it's name
    pub fn find_reactor_by_name(&self, name: &str) -> Option<&dyn BaseReactor> {
        self.reactors
            .iter()
            .find(|(_, reactor)| reactor.name() == name)
            .map(|(_, reactor)| reactor.as_ref())
    }
}

/// Bank information for a multi-bank port/reactor
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BankInfo {
    /// The index of this port/reactor within the bank
    pub idx: usize,
    /// The total number of ports/reactors in the bank
    pub total: usize,
}

/// Invariant data for the runtime, describing the resolved reaction graph and it's dependencies.
///
/// Maps of triggers for actions and ports. This data is statically resolved by the builder from the
/// reaction graph.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Default)]
pub struct ReactionGraph {
    /// All static execution scopes. Each reactor has a root scope, and each mode has a child scope.
    pub scopes: tinymap::TinyMap<ScopeKey, ScopeInfo>,
    /// Root scope per reactor.
    pub reactor_root_scopes: tinymap::TinySecondaryMap<ReactorKey, ScopeKey>,
    /// Scope per mode.
    pub mode_scopes: tinymap::TinySecondaryMap<ModeKey, ScopeKey>,
    /// All defined modes.
    pub modes: tinymap::TinyMap<ModeKey, Mode>,
    /// For each Action, a set of Reactions it triggers
    pub action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
    /// For each Port, a set of Reactions it triggers
    pub port_triggers: tinymap::TinySecondaryMap<PortKey, Vec<LevelReactionKey>>,
    /// Global startup actions
    pub startup_actions: Vec<(ActionKey, Tag)>,
    /// Timer startup actions. This excludes reactor lifecycle startup actions.
    pub timer_startup_actions: Vec<(ActionKey, Tag)>,
    /// Global shutdown actions
    pub shutdown_actions: Vec<ActionKey>,
    /// Whether each action uses logical-time scheduling.
    pub action_is_logical: tinymap::TinySecondaryMap<ActionKey, bool>,
    /// For each reaction, the ordered 'use' ports in declaration order
    pub reaction_use_ports: tinymap::TinySecondaryMap<ReactionKey, Vec<PortKey>>,
    /// For each reaction, the ordered 'effect' ports in declaration order
    pub reaction_effect_ports: tinymap::TinySecondaryMap<ReactionKey, Vec<PortKey>>,
    /// For each reaction, the ordered 'use/effect' actions in declaration order
    pub reaction_actions: tinymap::TinySecondaryMap<ReactionKey, Vec<ActionKey>>,
    /// For each reaction, the reactor it belongs to
    pub reaction_reactors: tinymap::TinySecondaryMap<ReactionKey, ReactorKey>,
    /// Static scope per reaction.
    pub reaction_scopes: tinymap::TinySecondaryMap<ReactionKey, ScopeKey>,
    /// Static scope per action.
    pub action_scopes: tinymap::TinySecondaryMap<ActionKey, ScopeKey>,
    /// Static scope per port. Ports are currently always reactor-root scoped.
    pub port_scopes: tinymap::TinySecondaryMap<PortKey, ScopeKey>,
    /// Bank index for a multi-bank reactor
    pub reactor_bank_infos: tinymap::TinySecondaryMap<ReactorKey, Option<BankInfo>>,
    /// All known modes per reactor
    pub reactor_modes: tinymap::TinySecondaryMap<ReactorKey, Vec<ModeKey>>,
    /// Initial mode per reactor (if any)
    pub reactor_initial_modes: tinymap::TinySecondaryMap<ReactorKey, Option<ModeKey>>,
    /// Mode filter per reaction (None means always enabled)
    pub reaction_modes: tinymap::TinySecondaryMap<ReactionKey, Option<ModeFilter>>,
    /// Reset-triggered reactions by their static owning scope.
    pub reset_reactions: tinymap::TinySecondaryMap<ScopeKey, Vec<LevelReactionKey>>,
    /// Startup-triggered reactions by their static owning scope.
    pub startup_reactions: tinymap::TinySecondaryMap<ScopeKey, Vec<LifecycleReaction>>,
    /// Shutdown-triggered reactions by their static owning scope.
    pub shutdown_reactions_by_scope: tinymap::TinySecondaryMap<ScopeKey, Vec<LifecycleReaction>>,
    /// Flattened static lookup tables for modal scheduler operations.
    pub modal_schedule_index: ModalScheduleIndex,
}

impl ReactionGraph {
    /// Get an iterator over all the shutdown reactions
    pub fn shutdown_reactions(&self) -> impl Iterator<Item = LevelReactionKey> + '_ {
        self.shutdown_reactions_by_scope
            .values()
            .flat_map(|reactions| reactions.iter().map(|reaction| reaction.reaction))
    }

    pub fn is_shutdown_reaction(&self, reaction_key: ReactionKey) -> bool {
        self.shutdown_reactions_by_scope
            .values()
            .flatten()
            .any(|reaction| reaction.reaction.1 == reaction_key)
    }
}
