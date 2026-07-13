#![cfg(feature = "federated")]
//! Proves the public `boomerang` API can build and execute static federations,
//! route a logical message through the RTI, and deliver it at `Tag::ZERO`.

use std::{
    sync::{mpsc, Arc, Mutex},
    time::Duration as StdDuration,
};

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
    let mut assembly = Assembly::new();
    assembly
        .register_federated_codec::<u32, _>(boomerang::federated::SerdeJsonCodec)
        .unwrap();

    StaticFederation(Arc::clone(&values))
        .build("main", (), None, None, None, false, &mut assembly)
        .unwrap();
    assembly.validate_reactions().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = assembly.into_runtime_parts(&config).unwrap();
    let _envs = execute_federation_in_memory(parts, config).unwrap();

    assert_eq!(*values.lock().unwrap(), vec![(Tag::ZERO, 7)]);
}

#[test]
#[ignore = "localhost TCP integration test; run with `cargo test -p boomerang --features federated tcp_static -- --ignored`"]
fn public_api_runs_tcp_static_federation() {
    boomerang_util::test_tracing::init_with_directive("debug");
    let values = run_with_wall_timeout("public TCP static federation", || {
        let values = Arc::new(Mutex::new(Vec::new()));
        let mut assembly = Assembly::new();
        assembly
            .register_federated_codec::<u32, _>(boomerang::federated::SerdeJsonCodec)
            .unwrap();

        StaticFederation(Arc::clone(&values))
            .build("main", (), None, None, None, false, &mut assembly)
            .unwrap();
        assembly.validate_reactions().unwrap();

        let config = runtime::Config::default().with_fast_forward(true);
        let parts = assembly.into_runtime_parts(&config).unwrap();
        let _envs =
            execute_federation_over_tcp(parts, config, TcpStaticFederationConfig::default())
                .unwrap();

        let recorded = values.lock().unwrap().clone();
        recorded
    });

    assert_eq!(values, vec![(Tag::ZERO, 7)]);
}

fn run_with_wall_timeout<T: Send + 'static>(
    label: &'static str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = tx.send(result);
    });

    match rx.recv_timeout(StdDuration::from_secs(5)) {
        Ok(Ok(value)) => value,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => panic!("{label} timed out"),
    }
}
