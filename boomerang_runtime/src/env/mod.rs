//! Runtime environment data consumed by the scheduler.
//!
//! This module defines the resolved reactors, actions, ports, reactions, and graph metadata that
//! the runtime executes. Build-time lowering and construction of derived graph indexes belong in
//! `boomerang_builder`; the runtime treats [`ReactionGraph`] as ready-to-execute data.

use crate::{
    event::AsyncEvent, keepalive, ActionKey, AsyncActionRef, BaseAction, BasePort, BaseReactor,
    Duration, DynActionRef, PortKey, Reaction, ReactionKey, ReactorData, ReactorKey, SendContext,
    Tag,
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
        self.modes.iter().any(|m| *m == mode)
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
    /// All defined modes with their owning reactor
    pub modes: tinymap::TinyMap<ModeKey, ReactorKey>,
    /// Names for each mode
    pub mode_names: tinymap::TinySecondaryMap<ModeKey, String>,
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
    /// Mode names per reactor
    pub reactor_mode_names: tinymap::TinySecondaryMap<ReactorKey, Vec<(ModeKey, String)>>,
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

tinymap::key_type! { pub EnclaveKey }

/// Upstream enclave reference
#[derive(Debug)]
pub struct UpstreamRef {
    /// The upstream `SendContext`
    pub send_ctx: SendContext,
    /// Optional delay for this upstream connection
    pub delay: Option<Duration>,
}

/// Downstream enclave reference
#[derive(Debug)]
pub struct DownstreamRef {
    /// The downstream `SendContext`
    pub send_ctx: SendContext,
}

/// An Enclave is the self-contained runtime data fed into a single scheduler instance.
#[derive(Debug)]
pub struct Enclave {
    /// The runtime environment
    pub env: Env,
    /// The reaction graph
    pub graph: ReactionGraph,
    /// The event channel for injecting events into the scheduler
    pub event_tx: crate::Sender<AsyncEvent>,
    /// The event receiver for receiving events into the scheduler
    pub event_rx: crate::Receiver<AsyncEvent>,
    /// The receivers from upstream enclaves for for granted tag advances, and the upstream
    /// `SendContext`
    pub upstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, UpstreamRef>,
    /// The senders to downstream enclaves for granted tag advances
    pub downstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, DownstreamRef>,
    /// The shutdown channel for the scheduler
    pub shutdown_tx: keepalive::Sender,
    /// The shutdown receiver for the scheduler
    pub shutdown_rx: keepalive::Receiver,
}

impl Default for Enclave {
    fn default() -> Self {
        let (event_tx, event_rx) = kanal::bounded(2);
        let (shutdown_tx, shutdown_rx) = keepalive::channel();
        Self {
            env: Default::default(),
            graph: Default::default(),
            event_tx,
            event_rx,
            downstream_enclaves: Default::default(),
            upstream_enclaves: Default::default(),
            shutdown_tx,
            shutdown_rx,
        }
    }
}

impl Enclave {
    pub fn with_event_q_size(physical_event_q_size: usize) -> Self {
        let size = physical_event_q_size.max(1);
        let (event_tx, event_rx) = kanal::bounded(size);
        let (shutdown_tx, shutdown_rx) = keepalive::channel();
        Self {
            env: Default::default(),
            graph: Default::default(),
            event_tx,
            event_rx,
            downstream_enclaves: Default::default(),
            upstream_enclaves: Default::default(),
            shutdown_tx,
            shutdown_rx,
        }
    }

    pub fn insert_reactor(
        &mut self,
        reactor: Box<dyn BaseReactor>,
        bank_info: Option<BankInfo>,
    ) -> ReactorKey {
        let reactor_key = self.env.reactors.insert(reactor);
        let root_scope = self.graph.scopes.insert(ScopeInfo {
            parent: None,
            reactor: reactor_key,
            mode: None,
        });
        self.graph.reset_reactions.insert(root_scope, Vec::new());
        self.graph.startup_reactions.insert(root_scope, Vec::new());
        self.graph
            .shutdown_reactions_by_scope
            .insert(root_scope, Vec::new());
        self.graph
            .reactor_root_scopes
            .insert(reactor_key, root_scope);
        self.graph.reactor_bank_infos.insert(reactor_key, bank_info);
        self.graph.reactor_modes.insert(reactor_key, Vec::new());
        self.graph
            .reactor_mode_names
            .insert(reactor_key, Vec::new());
        self.graph.reactor_initial_modes.insert(reactor_key, None);
        reactor_key
    }

    pub fn insert_action<F>(&mut self, action_fn: F) -> ActionKey
    where
        F: FnOnce(ActionKey) -> Box<dyn BaseAction>,
    {
        let action_key = self.env.actions.insert_with_key(action_fn);
        self.graph.action_triggers.insert(action_key, vec![]);
        self.graph
            .action_is_logical
            .insert(action_key, self.env.actions[action_key].is_logical());
        action_key
    }

    pub fn insert_port<F>(&mut self, port_fn: F) -> PortKey
    where
        F: FnOnce(PortKey) -> Box<dyn BasePort>,
    {
        let port_key = self.env.ports.insert_with_key(port_fn);
        self.graph.port_triggers.insert(port_key, vec![]);
        port_key
    }

    pub fn insert_mode(&mut self, reactor_key: ReactorKey, name: &str, initial: bool) -> ModeKey {
        let mode_key = self.graph.modes.insert(reactor_key);
        let root_scope = self.graph.reactor_root_scopes[reactor_key];
        let mode_scope = self.graph.scopes.insert(ScopeInfo {
            parent: Some(root_scope),
            reactor: reactor_key,
            mode: Some(mode_key),
        });
        self.graph.reset_reactions.insert(mode_scope, Vec::new());
        self.graph.startup_reactions.insert(mode_scope, Vec::new());
        self.graph
            .shutdown_reactions_by_scope
            .insert(mode_scope, Vec::new());
        self.graph.mode_scopes.insert(mode_key, mode_scope);
        self.graph.mode_names.insert(mode_key, name.to_owned());
        self.graph
            .reactor_modes
            .get_mut(reactor_key)
            .expect("reactor not found")
            .push(mode_key);
        self.graph
            .reactor_mode_names
            .get_mut(reactor_key)
            .expect("reactor not found")
            .push((mode_key, name.to_owned()));
        if initial {
            self.graph
                .reactor_initial_modes
                .insert(reactor_key, Some(mode_key));
        }
        mode_key
    }

    pub fn insert_reaction(
        &mut self,
        reaction: Reaction,
        reactor_key: ReactorKey,
        use_ports: impl IntoIterator<Item = PortKey>,
        effect_ports: impl IntoIterator<Item = PortKey>,
        actions: impl IntoIterator<Item = ActionKey>,
        scope: ScopeKey,
        mode_filter: Option<ModeFilter>,
    ) -> ReactionKey {
        let reaction_key = self.env.reactions.insert(reaction);
        self.graph
            .reaction_use_ports
            .insert(reaction_key, use_ports.into_iter().collect());
        self.graph
            .reaction_effect_ports
            .insert(reaction_key, effect_ports.into_iter().collect());
        self.graph
            .reaction_actions
            .insert(reaction_key, actions.into_iter().collect());
        self.graph
            .reaction_reactors
            .insert(reaction_key, reactor_key);
        self.graph.reaction_scopes.insert(reaction_key, scope);
        self.graph.reaction_modes.insert(reaction_key, mode_filter);
        reaction_key
    }

    pub fn root_scope(&self, reactor_key: ReactorKey) -> ScopeKey {
        self.graph.reactor_root_scopes[reactor_key]
    }

    pub fn mode_scope(&self, mode_key: ModeKey) -> ScopeKey {
        self.graph.mode_scopes[mode_key]
    }

    pub fn set_reactor_scope_parent(&mut self, reactor_key: ReactorKey, parent: ScopeKey) {
        let root_scope = self.root_scope(reactor_key);
        self.graph.scopes[root_scope].parent = Some(parent);
    }

    pub fn insert_action_scope(&mut self, action_key: ActionKey, scope: ScopeKey) {
        self.graph.action_scopes.insert(action_key, scope);
    }

    pub fn insert_port_scope(&mut self, port_key: PortKey, scope: ScopeKey) {
        self.graph.port_scopes.insert(port_key, scope);
    }

    /// Insert an `ActionKey` that is triggered on startup.
    pub fn insert_startup_action(&mut self, action_key: ActionKey, tag: Tag) {
        self.graph.startup_actions.push((action_key, tag));
    }

    /// Insert a timer `ActionKey` that is scheduled from local time zero.
    pub fn insert_timer_startup_action(&mut self, action_key: ActionKey, tag: Tag) {
        self.graph.timer_startup_actions.push((action_key, tag));
    }

    /// Insert an `ActionKey` that is triggered on shutdown.
    pub fn insert_shutdown_action(&mut self, action_key: ActionKey) {
        self.graph.shutdown_actions.push(action_key);
    }

    /// Insert a `LevelReactionKey` that is triggered by a port.
    pub fn insert_port_trigger(&mut self, port_key: PortKey, trigger: LevelReactionKey) {
        let triggers = self
            .graph
            .port_triggers
            .get_mut(port_key)
            .expect("port not found");
        triggers.push(trigger);
    }

    /// Insert a `LevelReactionKey` that is triggered by an action.
    pub fn insert_action_trigger(&mut self, action_key: ActionKey, trigger: LevelReactionKey) {
        let triggers = self
            .graph
            .action_triggers
            .get_mut(action_key)
            .expect("action not found");
        triggers.push(trigger);
    }

    /// Insert a `LevelReactionKey` that is triggered when a mode scope is entered by reset.
    pub fn insert_reset_trigger(&mut self, scope: ScopeKey, trigger: LevelReactionKey) {
        let triggers = self
            .graph
            .reset_reactions
            .get_mut(scope)
            .expect("scope not found");
        triggers.push(trigger);
    }

    /// Insert a `LevelReactionKey` that is triggered when its scope starts up.
    pub fn insert_startup_trigger(
        &mut self,
        scope: ScopeKey,
        action: ActionKey,
        trigger: LevelReactionKey,
    ) {
        let triggers = self
            .graph
            .startup_reactions
            .get_mut(scope)
            .expect("scope not found");
        triggers.push(LifecycleReaction {
            reaction: trigger,
            action,
        });
    }

    /// Insert a `LevelReactionKey` that is triggered when its scope shuts down.
    pub fn insert_shutdown_trigger(
        &mut self,
        scope: ScopeKey,
        action: ActionKey,
        trigger: LevelReactionKey,
    ) {
        let triggers = self
            .graph
            .shutdown_reactions_by_scope
            .get_mut(scope)
            .expect("scope not found");
        triggers.push(LifecycleReaction {
            reaction: trigger,
            action,
        });
    }

    /// Create a [`SendContext`] for sending events into the scheduler.
    pub fn create_send_context(&self, key: EnclaveKey) -> SendContext {
        SendContext {
            enclave_key: key,
            async_tx: self.event_tx.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
        }
    }

    /// Create an [`AsyncActionRef`] for interacting with an action asynchronously.
    pub fn create_async_action_ref<T: ReactorData>(
        &self,
        action_key: ActionKey,
    ) -> AsyncActionRef<T> {
        AsyncActionRef::try_from(DynActionRef(self.env.actions[action_key].as_ref()))
            .expect("type mismatch creating AsyncActionRef")
    }

    /// Validate the lengths of the runtime data structures.
    pub fn validate(&self) {
        itertools::assert_equal(self.env.actions.keys(), self.graph.action_triggers.keys());
        itertools::assert_equal(self.env.actions.keys(), self.graph.action_scopes.keys());
        itertools::assert_equal(self.env.actions.keys(), self.graph.action_is_logical.keys());
        itertools::assert_equal(self.env.ports.keys(), self.graph.port_triggers.keys());
        itertools::assert_equal(self.env.ports.keys(), self.graph.port_scopes.keys());
        itertools::assert_equal(
            self.env.reactions.keys(),
            self.graph.reaction_use_ports.keys(),
        );
        itertools::assert_equal(
            self.env.reactions.keys(),
            self.graph.reaction_effect_ports.keys(),
        );
        itertools::assert_equal(
            self.env.reactions.keys(),
            self.graph.reaction_actions.keys(),
        );
        itertools::assert_equal(
            self.env.reactions.keys(),
            self.graph.reaction_reactors.keys(),
        );
        itertools::assert_equal(self.env.reactions.keys(), self.graph.reaction_scopes.keys());
        itertools::assert_equal(self.env.reactions.keys(), self.graph.reaction_modes.keys());
        itertools::assert_equal(
            self.env.reactors.keys(),
            self.graph.reactor_bank_infos.keys(),
        );
        itertools::assert_equal(
            self.env.reactors.keys(),
            self.graph.reactor_root_scopes.keys(),
        );
        itertools::assert_equal(self.env.reactors.keys(), self.graph.reactor_modes.keys());
        itertools::assert_equal(
            self.env.reactors.keys(),
            self.graph.reactor_mode_names.keys(),
        );
        itertools::assert_equal(
            self.env.reactors.keys(),
            self.graph.reactor_initial_modes.keys(),
        );
        itertools::assert_equal(self.graph.modes.keys(), self.graph.mode_names.keys());
        itertools::assert_equal(self.graph.modes.keys(), self.graph.mode_scopes.keys());
        itertools::assert_equal(self.graph.scopes.keys(), self.graph.reset_reactions.keys());
        itertools::assert_equal(
            self.graph.scopes.keys(),
            self.graph.startup_reactions.keys(),
        );
        itertools::assert_equal(
            self.graph.scopes.keys(),
            self.graph.shutdown_reactions_by_scope.keys(),
        );
    }
}

/// Cross-Link enclaves from upstream to downstream.
///
/// The downstream enclave will wait for granted tags from the upstream enclave before advancing.
pub fn crosslink_enclaves<E>(
    enclaves: &mut E,
    upstream_key: EnclaveKey,
    downstream_key: EnclaveKey,
    delay: Option<Duration>,
) where
    E: std::ops::Index<EnclaveKey, Output = Enclave> + std::ops::IndexMut<EnclaveKey>,
{
    let downstream_ctx = enclaves[downstream_key].create_send_context(downstream_key);

    let upstream_enclave = &mut enclaves[upstream_key];
    upstream_enclave.downstream_enclaves.insert(
        downstream_key,
        DownstreamRef {
            send_ctx: downstream_ctx,
        },
    );

    let upstream_ctx = enclaves[upstream_key].create_send_context(upstream_key);

    let downstream_enclave = &mut enclaves[downstream_key];
    downstream_enclave.upstream_enclaves.insert(
        upstream_key,
        UpstreamRef {
            send_ctx: upstream_ctx,
            delay,
        },
    );
}
