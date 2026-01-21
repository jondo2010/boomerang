use crate::{
    reaction::{ConnectionReceiverReactionFn, EnclaveSenderReactionFn},
    reaction_closure, Action, Config, InputRef, OutputRef, Port, Reactor,
};

use super::*;

/// Create a dummy `Env` and `ReactionGraph` for testing.
pub fn create_dummy_env() -> (Env, ReactionGraph) {
    let mut env = Env::default();
    let reactor_key = env.reactors.insert(Reactor::new("dummy", ()).boxed());
    let reaction_key = env
        .reactions
        .insert(Reaction::new("dummy", reaction_closure!(), None));
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
pub fn create_enclave_pair() -> tinymap::TinyMap<EnclaveKey, Enclave> {
    let mut enclaves = tinymap::TinyMap::with_capacity(2);

    // receiver-side
    let key_b = enclaves.insert(Enclave::default());
    let enclave_b = &mut enclaves[key_b];

    let reactor_b = enclave_b.insert_reactor(Reactor::new("reactorB", false).boxed(), None);
    let port_b = enclave_b.insert_port(|key| Port::<u32>::new("portB", key).boxed());
    let action_b =
        enclave_b.insert_action(|key| Action::<u32>::new("actionB", key, None, true).boxed());

    // receiver-side has a reaction that reads the value from 'portB' and prints it.
    let reaction_output = enclave_b.insert_reaction(
        Reaction::new(
            "reactionOut",
            reaction_closure!(
            ctx, reactor, refs => {
                assert_eq!(ctx.get_elapsed_logical_time(), Duration::ZERO);
                let state = reactor.get_state_mut::<bool>().unwrap();
                *state = true;
                let port: InputRef<u32> = refs.ports.partition().unwrap();
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
            ConnectionReceiverReactionFn::<u32>::default(),
            None,
        ),
        reactor_b,
        std::iter::empty(),
        std::iter::once(port_b),
        std::iter::once(action_b),
    );

    // actionB triggers reactionB
    enclave_b.insert_action_trigger(action_b, (Level::from(0), reaction_b));

    let enclave_b_remote_context = enclave_b.create_send_context(key_b);
    let enclave_b_remote_action_ref = enclave_b.create_async_action_ref(action_b);

    // sender-side enclave
    let key_a = enclaves.insert(Enclave::default());
    let enclave_a = &mut enclaves[key_a];

    let reactor_a = enclave_a.insert_reactor(Reactor::new("reactorA", ()).boxed(), None);
    // sender-side has a startup reaction that sets the value of 'portA' to 42.
    let port_a = enclave_a.insert_port(|key| Port::<u32>::new("portA", key).boxed());

    let reaction_startup = enclave_a.insert_reaction(
        Reaction::new(
            "startup",
            reaction_closure!(
            ctx, _state, refs=> {
                assert_eq!(ctx.get_elapsed_logical_time(), Duration::ZERO);
                let mut port: OutputRef<u32> = refs.ports_mut.partition_mut().unwrap();
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

    // startup action triggers reactionStartup
    let startup_action =
        enclave_a.insert_action(|key| Action::<()>::new("startup", key, None, true).boxed());
    enclave_a.insert_startup_action(startup_action, Tag::ZERO);
    enclave_a.insert_action_trigger(startup_action, (Level::from(0), reaction_startup));

    // The sender-side has a reaction that is triggered by 'portA' and sends an async event to the receiver-side.
    let reaction_a = enclave_a.insert_reaction(
        Reaction::new(
            "reactionA",
            EnclaveSenderReactionFn::<u32>::new(
                enclave_b_remote_context,
                enclave_b_remote_action_ref,
                None,
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
    crate::crosslink_enclaves(&mut enclaves, key_a, key_b, None);

    enclaves
}

#[test]
fn test_enclave0() {
    tracing_subscriber::fmt::init();

    let enclaves = create_enclave_pair();
    assert_eq!(enclaves.len(), 2);

    for (key, enclave) in enclaves.iter() {
        enclave.validate();
        let name = enclave.env.reactors.values().next().unwrap().name();
        tracing::info!("Enclave {key}: {name}");
    }

    let config = Config::default()
        .with_fast_forward(true)
        .with_keep_alive(false)
        .with_timeout(Duration::seconds(3));

    let envs_out = crate::execute_enclaves(enclaves.into_iter(), config);

    for env in envs_out.values() {
        if let Some(state) = env
            .find_reactor_by_name("reactorB")
            .and_then(|r| r.get_state::<bool>())
        {
            assert!(*state, "Expected state to be true");
        }
    }
}
