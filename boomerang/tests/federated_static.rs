#![cfg(feature = "federated")]
//! Proves the public `boomerang` API can build and execute a static in-memory
//! federation, route a logical message through the RTI, and deliver it at
//! `Tag::ZERO`.

use std::sync::{Arc, Mutex};

use boomerang::prelude::*;

#[derive(Clone)]
struct SinkState {
    values: Arc<Mutex<Vec<(Tag, u32)>>>,
}

#[reactor]
fn FederatedSource(#[output] out: u32) -> impl Reactor {
    builder
        .add_reaction(Some("emit"))
        .with_startup_trigger()
        .with_effect(out)
        .with_reaction_fn(|ctx, _state, (_startup, mut out)| {
            *out = Some(7);
            ctx.schedule_shutdown(None);
        })
        .finish()?;
}

#[reactor(state = SinkState)]
fn FederatedSink(#[input] input: u32) -> impl Reactor {
    builder
        .add_reaction(Some("keep_alive_until_message"))
        .with_startup_trigger()
        .with_reaction_fn(|ctx, _state, (_startup,)| {
            ctx.schedule_shutdown(Some(Duration::milliseconds(100)));
        })
        .finish()?;

    builder
        .add_reaction(Some("record"))
        .with_trigger(input)
        .with_reaction_fn(|ctx, state, (input,)| {
            if let Some(value) = *input {
                state.values.lock().unwrap().push((ctx.get_tag(), value));
                ctx.schedule_shutdown(None);
            }
        })
        .finish()?;
}

#[reactor]
fn StaticFederation(values: Arc<Mutex<Vec<(Tag, u32)>>>) -> impl Reactor {
    let source = builder.add_child_federate(FederatedSource(), "source", ())?;
    let sink = builder.add_child_federate(
        FederatedSink(),
        "sink",
        SinkState {
            values: Arc::clone(&values),
        },
    )?;

    builder.connect_port(source.out, sink.input, None, false)?;
}

#[test]
fn public_api_runs_static_in_memory_federation() {
    boomerang_util::test_tracing::init_with_directive("debug");
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut env_builder = EnvBuilder::new();
    env_builder
        .register_federated_codec::<u32, _>(boomerang::federated::SerdeJsonCodec)
        .unwrap();

    StaticFederation(Arc::clone(&values))
        .build("main", (), None, None, None, false, &mut env_builder)
        .unwrap();
    env_builder.validate_reactions().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = env_builder.into_runtime_parts(&config).unwrap();
    let _envs = execute_federation_in_memory(parts, config).unwrap();

    assert_eq!(*values.lock().unwrap(), vec![(Tag::ZERO, 7)]);
}
