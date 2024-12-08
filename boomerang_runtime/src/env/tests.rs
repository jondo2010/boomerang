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
    let action_key0 = env
        .actions
        .insert(Action::<()>::new("action0", ActionKey::from(0), Default::default(), true).boxed());
    let action_key1 = env
        .actions
        .insert(Action::<()>::new("action1", ActionKey::from(1), Default::default(), true).boxed());
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
    let reactor_b = enclave_b.insert_reactor(Reactor::new("reactorB", false).boxed(), None);
    let port_b = enclave_b.insert_port(|key| Port::<u32>::new("portB", key).boxed());
    let action_b = enclave_b.insert_action(|key| {
        Action::<u32>::new("actionB", key, Some(Duration::from_secs(1)), true).boxed()
    });

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
    let reactor_a = enclave_a.insert_reactor(Reactor::new("reactorA", ()).boxed(), None);
    // sender-side has a startup reaction that sets the value of 'portA' to 42.
    let port_a = enclave_a.insert_port(|key| Port::<u32>::new("portA", key).boxed());

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

    enclave_a.insert_startup_reaction((Level::from(0), reaction_startup), None);

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
