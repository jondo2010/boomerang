use crate::{
    event::AsyncEvent, keepalive, ActionKey, AsyncActionRef, BaseAction, BasePort, BaseReactor,
    Duration, DynActionRef, PortKey, Reaction, ReactionKey, ReactorData, ReactorKey, SendContext,
    Tag,
};

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

/// `Env` stores the resolved runtime state of all the reactors.
///
/// The reactor heirarchy has been flattened and build by the builder methods.
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
    /// For each Action, a set of Reactions it triggers
    pub action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
    /// For each Port, a set of Reactions it triggers
    pub port_triggers: tinymap::TinySecondaryMap<PortKey, Vec<LevelReactionKey>>,
    /// Global startup actions
    pub startup_actions: Vec<(ActionKey, Tag)>,
    /// Global shutdown actions
    pub shutdown_actions: Vec<ActionKey>,
    /// For each reaction, the ordered 'use' ports in declaration order
    pub reaction_use_ports: tinymap::TinySecondaryMap<ReactionKey, Vec<PortKey>>,
    /// For each reaction, the ordered 'effect' ports in declaration order
    pub reaction_effect_ports: tinymap::TinySecondaryMap<ReactionKey, Vec<PortKey>>,
    /// For each reaction, the ordered 'use/effect' actions in declaration order
    pub reaction_actions: tinymap::TinySecondaryMap<ReactionKey, Vec<ActionKey>>,
    /// For each reaction, the reactor it belongs to
    pub reaction_reactors: tinymap::TinySecondaryMap<ReactionKey, ReactorKey>,
    /// Bank index for a multi-bank reactor
    pub reactor_bank_infos: tinymap::TinySecondaryMap<ReactorKey, Option<BankInfo>>,
}

impl ReactionGraph {
    /// Get an iterator over all the shutdown reactions
    pub fn shutdown_reactions(&self) -> impl Iterator<Item = LevelReactionKey> + '_ {
        self.shutdown_actions
            .iter()
            .flat_map(|&action_key| self.action_triggers[action_key].iter().copied())
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
    pub fn insert_reactor(
        &mut self,
        reactor: Box<dyn BaseReactor>,
        bank_info: Option<BankInfo>,
    ) -> ReactorKey {
        let reactor_key = self.env.reactors.insert(reactor);
        self.graph.reactor_bank_infos.insert(reactor_key, bank_info);
        reactor_key
    }

    pub fn insert_action<F>(&mut self, action_fn: F) -> ActionKey
    where
        F: FnOnce(ActionKey) -> Box<dyn BaseAction>,
    {
        let action_key = self.env.actions.insert_with_key(action_fn);
        self.graph.action_triggers.insert(action_key, vec![]);
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

    pub fn insert_reaction(
        &mut self,
        reaction: Reaction,
        reactor_key: ReactorKey,
        use_ports: impl IntoIterator<Item = PortKey>,
        effect_ports: impl IntoIterator<Item = PortKey>,
        actions: impl IntoIterator<Item = ActionKey>,
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
        reaction_key
    }

    /// Insert an `ActionKey` that is triggered on startup.
    pub fn insert_startup_action(&mut self, action_key: ActionKey, tag: Tag) {
        self.graph.startup_actions.push((action_key, tag));
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
        itertools::assert_equal(self.env.ports.keys(), self.graph.port_triggers.keys());
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
        itertools::assert_equal(
            self.env.reactors.keys(),
            self.graph.reactor_bank_infos.keys(),
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
