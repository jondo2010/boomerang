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
    ctx.add_reaction(Some("emit"))
        .with_startup_trigger()
        .with_effect(out)
        .with_reaction_fn(|ctx, _state, (_startup, mut out)| {
            *out = Some(7);
            ctx.schedule_shutdown(None);
        })
        .finish()?;
}

#[reactor]
fn FederatedRelay(#[input] input: u32, #[output] out: u32) -> impl Reactor {
    ctx.add_reaction(Some("keep_alive_until_message"))
        .with_startup_trigger()
        .with_reaction_fn(|ctx, _state, (_startup,)| {
            ctx.schedule_shutdown(Some(Duration::milliseconds(100)));
        })
        .finish()?;

    ctx.add_reaction(Some("relay"))
        .with_trigger(input)
        .with_effect(out)
        .with_reaction_fn(|_ctx, _state, (input, mut out)| {
            if let Some(value) = *input {
                *out = Some(value);
            }
        })
        .finish()?;
}

fn federate_a() -> impl Reactor<(), Ports = FederatedRelayPorts> {
    |name: &str,
     state: (),
     parent: Option<AssemblyReactorKey>,
     scope_mode: Option<AssemblyModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let source = ctx.add_child_reactor(FederatedSource(), "source", (), false)?;
        let relay = ctx.add_child_reactor(FederatedRelay(), "relay", (), true)?;
        ctx.connect_port(source.out, relay.input, None, false)?;
        ctx.finish()?;
        Ok(relay)
    }
}

#[reactor(state = SinkState)]
fn FederatedSink(#[input] input: u32) -> impl Reactor {
    ctx.add_reaction(Some("keep_alive_until_message"))
        .with_startup_trigger()
        .with_reaction_fn(|ctx, _state, (_startup,)| {
            ctx.schedule_shutdown(Some(Duration::milliseconds(100)));
        })
        .finish()?;

    ctx.add_reaction(Some("record"))
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
    let source = ctx.add_child_reactor_with_placement(
        federate_a(),
        "a",
        (),
        ReactorPlacement::federate("a"),
    )?;
    let sink = ctx.add_child_federate(
        FederatedSink(),
        "b",
        SinkState {
            values: Arc::clone(&values),
        },
    )?;

    ctx.connect_port(source.out, sink.input, None, false)?;
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
        .build(
            "main",
            (),
            None,
            None,
            None,
            ReactorPlacement::Local,
            &mut assembly,
        )
        .unwrap();
    assembly.validate_reactions().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = assembly.into_runtime_assembly(&config).unwrap();
    let federation = parts.federation().unwrap();
    assert_eq!(federation.federates().len(), 2);
    assert_eq!(
        federation.federates()[&FederateId::new("a")]
            .enclaves()
            .len(),
        2
    );
    assert_eq!(
        federation.federates()[&FederateId::new("b")]
            .enclaves()
            .len(),
        1
    );
    assert_eq!(federation.topology().topology().edges.len(), 1);
    let _envs = execute_federation_in_memory(parts.into_federation().unwrap(), config).unwrap();

    assert_eq!(*values.lock().unwrap(), vec![(Tag::ZERO, 7)]);
}

#[test]
fn public_api_rejects_runtime_without_lowered_federation() {
    let parts = RuntimeAssembly::default();

    assert!(matches!(
        parts.into_federation(),
        Err(RuntimeExecutionError::ExpectedFederation)
    ));
}

#[test]
fn public_api_federates_own_local_enclave_maps() {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    assembly
        .register_federated_codec::<u32, _>(boomerang::federated::SerdeJsonCodec)
        .unwrap();
    StaticFederation(values)
        .build(
            "main",
            (),
            None,
            None,
            None,
            ReactorPlacement::Local,
            &mut assembly,
        )
        .unwrap();
    assembly.validate_reactions().unwrap();

    let federation = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap()
        .into_federation()
        .unwrap();
    let (topology, federates) = federation.into_parts();
    assert_eq!(topology.topology().edges.len(), 1);
    assert_eq!(federates.len(), 2);

    let a = &federates[&FederateId::new("a")];
    let b = &federates[&FederateId::new("b")];
    assert_eq!(a.id(), &FederateId::new("a"));
    assert_eq!(a.enclaves().len(), 2);
    assert_eq!(a.bridge().routes().count(), 0);
    assert_eq!(b.id(), &FederateId::new("b"));
    assert_eq!(b.enclaves().len(), 1);
    assert_eq!(b.bridge().routes().count(), 1);
    assert_eq!(a.enclaves().keys().next(), b.enclaves().keys().next());
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
            .build(
                "main",
                (),
                None,
                None,
                None,
                ReactorPlacement::Local,
                &mut assembly,
            )
            .unwrap();
        assembly.validate_reactions().unwrap();

        let config = runtime::Config::default().with_fast_forward(true);
        let parts = assembly.into_runtime_assembly(&config).unwrap();
        let _envs = execute_federation_over_tcp(
            parts.into_federation().unwrap(),
            config,
            TcpStaticFederationConfig::default(),
        )
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
