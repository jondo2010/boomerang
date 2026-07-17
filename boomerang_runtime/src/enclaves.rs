//! Runtime data owned by an individual scheduler Enclave.

use crate::{
    env::{
        BankInfo, Env, LevelReactionKey, LifecycleReaction, Mode, ModeFilter, ModeKey,
        ReactionGraph, ScopeInfo, ScopeKey,
    },
    event::AsyncEvent,
    keepalive, ActionKey, AsyncActionRef, BaseAction, BasePort, BaseReactor, Duration,
    DynActionRef, PortKey, Reaction, ReactionKey, ReactorData, ReactorKey, SendContext, Tag,
};

tinymap::key_type! { pub EnclaveKey }

/// Reference to an upstream Enclave and its logical-time delay.
#[derive(Debug)]
pub struct UpstreamRef {
    /// Context used to send events to the upstream Enclave.
    pub send_ctx: SendContext,
    /// Optional delay applied to the upstream connection.
    pub delay: Option<Duration>,
}

/// Reference used to send events to a downstream Enclave.
#[derive(Debug)]
pub struct DownstreamRef {
    /// Context used to send events to the downstream Enclave.
    pub send_ctx: SendContext,
}

/// Self-contained runtime data consumed by one scheduler instance.
#[derive(Debug)]
pub struct Enclave {
    /// Resolved reactors, actions, ports, and reactions.
    pub env: Env,
    /// Resolved reaction graph and scheduling metadata.
    pub graph: ReactionGraph,
    /// Channel for injecting events into the scheduler.
    pub event_tx: crate::Sender<AsyncEvent>,
    /// Channel from which the scheduler receives injected events.
    pub event_rx: crate::Receiver<AsyncEvent>,
    /// Upstream Enclaves that constrain logical-time advancement.
    pub upstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, UpstreamRef>,
    /// Downstream Enclaves notified of granted logical-time advances.
    pub downstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, DownstreamRef>,
    /// Channel used to signal scheduler shutdown.
    pub shutdown_tx: keepalive::Sender,
    /// Channel used to observe scheduler shutdown.
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
    /// Create an empty Enclave with the requested physical event queue capacity.
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

    /// Insert a Reactor and initialize its root scheduling scope.
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
        self.graph.reactor_initial_modes.insert(reactor_key, None);
        reactor_key
    }

    /// Insert an Action and initialize its static graph entries.
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

    /// Insert a Port and initialize its static graph entries.
    pub fn insert_port<F>(&mut self, port_fn: F) -> PortKey
    where
        F: FnOnce(PortKey) -> Box<dyn BasePort>,
    {
        let port_key = self.env.ports.insert_with_key(port_fn);
        self.graph.port_triggers.insert(port_key, vec![]);
        port_key
    }

    /// Insert a Mode owned by the given Reactor.
    pub fn insert_mode(&mut self, reactor_key: ReactorKey, name: &str, initial: bool) -> ModeKey {
        let mode_key = self.graph.modes.insert(Mode {
            name: name.to_owned(),
            parent: reactor_key,
        });
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
        self.graph
            .reactor_modes
            .get_mut(reactor_key)
            .expect("reactor not found")
            .push(mode_key);
        if initial {
            self.graph
                .reactor_initial_modes
                .insert(reactor_key, Some(mode_key));
        }
        mode_key
    }

    /// Insert a Reaction and its resolved graph relationships.
    #[allow(clippy::too_many_arguments)]
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

    /// Return the root scheduling scope for a Reactor.
    pub fn root_scope(&self, reactor_key: ReactorKey) -> ScopeKey {
        self.graph.reactor_root_scopes[reactor_key]
    }

    /// Return the scheduling scope for a Mode.
    pub fn mode_scope(&self, mode_key: ModeKey) -> ScopeKey {
        self.graph.mode_scopes[mode_key]
    }

    /// Assign a Reactor's root scope to a parent scope.
    pub fn set_reactor_scope_parent(&mut self, reactor_key: ReactorKey, parent: ScopeKey) {
        let root_scope = self.root_scope(reactor_key);
        self.graph.scopes[root_scope].parent = Some(parent);
    }

    /// Assign an Action to a scheduling scope.
    pub fn insert_action_scope(&mut self, action_key: ActionKey, scope: ScopeKey) {
        self.graph.action_scopes.insert(action_key, scope);
    }

    /// Assign a Port to a scheduling scope.
    pub fn insert_port_scope(&mut self, port_key: PortKey, scope: ScopeKey) {
        self.graph.port_scopes.insert(port_key, scope);
    }

    /// Insert an Action that is triggered on startup.
    pub fn insert_startup_action(&mut self, action_key: ActionKey, tag: Tag) {
        self.graph.startup_actions.push((action_key, tag));
    }

    /// Insert a timer Action that is scheduled from local time zero.
    pub fn insert_timer_startup_action(&mut self, action_key: ActionKey, tag: Tag) {
        self.graph.timer_startup_actions.push((action_key, tag));
    }

    /// Insert an Action that is triggered on shutdown.
    pub fn insert_shutdown_action(&mut self, action_key: ActionKey) {
        self.graph.shutdown_actions.push(action_key);
    }

    /// Insert a Reaction that is triggered by a Port.
    pub fn insert_port_trigger(&mut self, port_key: PortKey, trigger: LevelReactionKey) {
        let triggers = self
            .graph
            .port_triggers
            .get_mut(port_key)
            .expect("port not found");
        triggers.push(trigger);
    }

    /// Insert a Reaction that is triggered by an Action.
    pub fn insert_action_trigger(&mut self, action_key: ActionKey, trigger: LevelReactionKey) {
        let triggers = self
            .graph
            .action_triggers
            .get_mut(action_key)
            .expect("action not found");
        triggers.push(trigger);
    }

    /// Insert a Reaction triggered when a Mode scope is entered by reset.
    pub fn insert_reset_trigger(&mut self, scope: ScopeKey, trigger: LevelReactionKey) {
        let triggers = self
            .graph
            .reset_reactions
            .get_mut(scope)
            .expect("scope not found");
        triggers.push(trigger);
    }

    /// Insert a Reaction triggered when its scope starts up.
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

    /// Insert a Reaction triggered when its scope shuts down.
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

    /// Create a context for sending events to this Enclave's scheduler.
    pub fn create_send_context(&self, key: EnclaveKey) -> SendContext {
        SendContext {
            enclave_key: key,
            async_tx: self.event_tx.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
        }
    }

    /// Create an asynchronous reference to a typed Action.
    pub fn create_async_action_ref<T: ReactorData>(
        &self,
        action_key: ActionKey,
    ) -> AsyncActionRef<T> {
        AsyncActionRef::try_from(DynActionRef(self.env.actions[action_key].as_ref()))
            .expect("type mismatch creating AsyncActionRef")
    }

    /// Validate that primary runtime maps and their secondary indexes have matching keys.
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
            self.graph.reactor_initial_modes.keys(),
        );
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

/// Cross-link an upstream Enclave to a downstream Enclave.
///
/// The downstream Enclave waits for granted tags from the upstream Enclave before advancing.
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
