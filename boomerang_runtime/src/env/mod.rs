use crate::{
    event::AsyncEvent, keepalive, ActionKey, AsyncActionRef, BaseAction, BasePort, BaseReactor, PortKey, Reaction, ReactionKey, ReactorData, ReactorKey, SendContext, Tag
};

mod debug;

/// Execution level
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
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
    /// Global startup reactions
    pub startup_reactions: Vec<LevelReactionKey>,
    /// Global shutdown reactions
    pub shutdown_reactions: Vec<LevelReactionKey>,
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

/// An Enclave is the self-contained runtime data fed into a single scheduler instance.
pub struct Enclave {
    /// The runtime environment
    pub env: Env,
    /// The reaction graph
    pub graph: ReactionGraph,
    /// The event channel for injecting events into the scheduler
    pub event_tx: crate::Sender<AsyncEvent>,
    /// The event receiver for receiving events into the scheduler
    pub event_rx: crate::Receiver<AsyncEvent>,
    /// The senders to downstream enclaves for granted tag advances
    pub downstream_tx: Vec<crate::Sender<Tag>>,
    /// The receivers from upstream enclaves for for granted tag advances
    pub upstream_rx: Vec<crate::Receiver<Tag>>,
    /// The shutdown channel for the scheduler
    pub shutdown_tx: keepalive::Sender,
    /// The shutdown receiver for the scheduler
    pub shutdown_rx: keepalive::Receiver,
}

impl Default for Enclave {
    fn default() -> Self {
        let (event_tx, event_rx) = crossbeam_channel::bounded(1);
        let (shutdown_tx, shutdown_rx) = keepalive::channel();
        Self {
            env: Default::default(),
            graph: Default::default(),
            event_tx,
            event_rx,
            downstream_tx: vec![],
            upstream_rx: vec![],
            shutdown_tx,
            shutdown_rx,
        }
    }
}

impl Enclave {
    pub fn insert_reactor(&mut self, reactor: Box<dyn BaseReactor>) -> ReactorKey {
        let reactor_key = self.env.reactors.insert(reactor);
        self.graph.reactor_bank_infos.insert(reactor_key, None);
        reactor_key
    }

    pub fn insert_action(&mut self, action: Box<dyn BaseAction>) -> ActionKey {
        let action_key = self.env.actions.insert(action);
        self.graph.action_triggers.insert(action_key, vec![]);
        action_key
    }

    pub fn insert_port(&mut self, port: Box<dyn BasePort>) -> PortKey {
        let port_key = self.env.ports.insert(port);
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

    pub fn insert_startup_reaction(&mut self, level_reaction_key: LevelReactionKey) {
        self.graph.startup_reactions.push(level_reaction_key);
    }

    pub fn insert_shutdown_reaction(&mut self, level_reaction_key: LevelReactionKey) {
        self.graph.shutdown_reactions.push(level_reaction_key);
    }

    pub fn insert_port_trigger(&mut self, port_key: PortKey, trigger: LevelReactionKey) {
        let triggers = self
            .graph
            .port_triggers
            .get_mut(port_key)
            .expect("port not found");
        triggers.push(trigger);
    }

    pub fn insert_action_trigger(&mut self, action_key: ActionKey, trigger: LevelReactionKey) {
        let triggers = self
            .graph
            .action_triggers
            .get_mut(action_key)
            .expect("action not found");
        triggers.push(trigger);
    }

    /// Link this enclave to an upstream enclave.
    pub fn link_upstream(&mut self, upstream: &mut Self) {
        let (tag_tx, tag_rx) = crossbeam_channel::bounded(1);
        upstream.downstream_tx.push(tag_tx);
        self.upstream_rx.push(tag_rx);
    }

    /// Create a [`SendContext`] for sending events into the scheduler.
    pub fn create_send_context(&self) -> SendContext {
        SendContext {
            async_tx: self.event_tx.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
        }
    }

    /// Create an [`AsyncActionRef`] for interacting with an action asynchronously.
    pub fn create_async_action_ref<T: ReactorData>(&self, action_key: ActionKey) -> AsyncActionRef<T> {
        self.env.actions[action_key].as_ref().into()
    }

    fn validate(&self) {
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

#[cfg(test)]
pub mod tests {
    use std::time::Duration;

    use tinymap::DefaultKey;

    use crate::{
        reaction::{EnclaveReceiverReactionFn, EnclaveSenderReactionFn},
        reaction_closure, Action, BaseReactor, Config, Context, InputRef, OutputRef, Port, Reactor,
        Scheduler,
    };

    use super::*;

    /// An empty reaction function for testing.
    pub fn dummy_reaction_fn<'a>(
        _context: &'a mut Context,
        _reactor: &'a mut dyn BaseReactor,
        _ref_ports: crate::refs::Refs<'a, dyn BasePort>,
        _mut_ports: crate::refs::RefsMut<'a, dyn BasePort>,
        _actions: crate::refs::RefsMut<'a, dyn BaseAction>,
    ) {
    }

    /// Create a dummy `Env` and `ReactionGraph` for testing.
    pub fn create_dummy_env() -> (Env, ReactionGraph) {
        let mut env = Env::default();
        let reactor_key = env.reactors.insert(Reactor::new("dummy", ()).boxed());
        let reaction_key =
            env.reactions
                .insert(Reaction::new("dummy", Box::new(dummy_reaction_fn), None));
        let action_key0 = env.actions.insert(
            Action::<()>::new("action0", ActionKey::from(0), Default::default(), true).boxed(),
        );
        let action_key1 = env.actions.insert(
            Action::<()>::new("action1", ActionKey::from(1), Default::default(), true).boxed(),
        );
        let port_key0 = env
            .ports
            .insert(Port::<u32>::new("port0", PortKey::from(0)).boxed());
        let port_key1 = env
            .ports
            .insert(Port::<u32>::new("port1", PortKey::from(1)).boxed());

        let mut reaction_graph = ReactionGraph::default();
        reaction_graph
            .reaction_use_ports
            .insert(reaction_key, std::iter::once(port_key0).collect());
        reaction_graph
            .reaction_effect_ports
            .insert(reaction_key, std::iter::once(port_key1).collect());
        reaction_graph.reaction_actions.insert(
            reaction_key,
            [action_key0, action_key1].into_iter().collect(),
        );
        reaction_graph
            .reaction_reactors
            .insert(reaction_key, reactor_key);

        (env, reaction_graph)
    }

    /// Create a test pair of `Env` and `ReactionGraph` with an Enclave connection between them.
    ///
    /// In the builder/logically: The top-level reactor has a `Connection` between two ports 'portA' and 'portB'.
    pub fn create_enclave_pair() -> tinymap::TinyMap<DefaultKey, Enclave> {
        let mut enclaves = tinymap::TinyMap::default();

        // receiver-side
        let mut enclave_b = Enclave::default();
        let reactor_b = enclave_b.insert_reactor(Reactor::new("reactorB", false).boxed());
        let port_b = enclave_b.insert_port(Port::<u32>::new("portB", PortKey::from(0)).boxed());
        let action_b = enclave_b.insert_action(
            Action::<u32>::new(
                "actionB",
                ActionKey::from(0),
                Some(Duration::from_secs(1)),
                true,
            )
            .boxed(),
        );

        // receiver-side has a reaction that reads the value from 'portB' and prints it.
        let reaction_output = enclave_b.insert_reaction(
            Reaction::new(
                "reactionOut",
                reaction_closure!(
                _ctx, reactor, ref_ports, _mut_ports, _actions => {
                    let state = reactor.get_state_mut::<bool>().unwrap();
                    *state = true;
                    let port: InputRef<u32> = ref_ports.partition().unwrap();
                    tracing::info!("portB value: {:?}", *port);
                }),
                None,
            ),
            reactor_b,
            std::iter::once(port_b),
            std::iter::empty(),
            std::iter::empty(),
        );

        // portB triggers reactionOutput
        enclave_b.insert_port_trigger(port_b, (Level::from(1), reaction_output));

        // receiver-side has an Action 'actionB' that triggers a reaction which effects 'portB' (writes the value from the action to the port).
        let reaction_b = enclave_b.insert_reaction(
            Reaction::new(
                "reactionB",
                EnclaveReceiverReactionFn::<u32>::default(),
                None,
            ),
            reactor_b,
            std::iter::empty(),
            std::iter::once(port_b),
            std::iter::once(action_b),
        );

        // actionB triggers reactionB
        enclave_b.insert_action_trigger(action_b, (Level::from(0), reaction_b));

        // sender-side enclave
        let mut enclave_a = Enclave::default();
        let reactor_a = enclave_a.insert_reactor(Reactor::new("reactorA", ()).boxed());
        // sender-side has a startup reaction that sets the value of 'portA' to 42.
        let port_a = enclave_a.insert_port(Port::<u32>::new("portA", PortKey::from(0)).boxed());

        let reaction_startup = enclave_a.insert_reaction(
            Reaction::new(
                "startup",
                reaction_closure!(
                _ctx, _state, _ref_ports, mut_ports, _actions => {
                    let mut port: OutputRef<u32> = mut_ports.partition_mut().unwrap();
                    *port = Some(42);
                }),
                None,
            ),
            reactor_a,
            std::iter::empty(),
            // portA is effected by reactionStartup
            std::iter::once(port_a),
            std::iter::empty(),
        );

        enclave_a.insert_startup_reaction((Level::from(0), reaction_startup));

        // The sender-side has a reaction that is triggered by 'portA' and sends an async event to the receiver-side.
        let reaction_a = enclave_a.insert_reaction(
            Reaction::new(
                "reactionA",
                EnclaveSenderReactionFn::<u32>::new(
                    enclave_b.create_send_context(),
                    enclave_b.create_async_action_ref(action_b),
                    Some(Duration::from_millis(500)),
                ),
                None,
            ),
            reactor_a,
            // reactionA uses portA
            std::iter::once(port_a),
            std::iter::empty(),
            std::iter::empty(),
        );

        // portA triggers reactionA
        enclave_a.insert_port_trigger(port_a, (Level::from(1), reaction_a));

        // link the two enclaves
        enclave_b.link_upstream(&mut enclave_a);

        enclaves.insert(enclave_a);
        enclaves.insert(enclave_b);

        enclaves
    }

    #[test]
    #[cfg(feature = "parallel")]
    fn test_enclave0() {
        use rayon::iter::{ParallelBridge, ParallelIterator};

        tracing_subscriber::fmt()
            .with_thread_ids(true)
            .with_max_level(tracing::Level::TRACE)
            .compact()
            .init();

        let enclaves = create_enclave_pair();
        assert_eq!(enclaves.len(), 2);

        for enclave in enclaves.values() {
            enclave.validate();
        }

        let config = Config::default()
            .with_fast_forward(false)
            .with_timeout(Duration::from_secs(3));

        rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build_global()
            .unwrap();

        let envs_out = enclaves
            .into_iter()
            .par_bridge()
            .map(|(reactor_key, enclave)| {
                let mut sched = Scheduler::new(enclave, config.clone());

                tracing::info!("Starting scheduler for reactor {reactor_key:?}");
                sched.event_loop();
                let env = sched.into_env();

                (reactor_key, env)
            });

        envs_out.for_each(|(reactor_key, env)| {
            if let Some(state) = env
                .find_reactor_by_name("reactorB")
                .and_then(|r| r.get_state::<bool>())
            {
                assert!(*state, "Expected state to be true");
            }
        });
    }
}
