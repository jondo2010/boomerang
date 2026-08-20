#![cfg(feature = "federated")]
//! Proves the public `boomerang` API can build and execute static federations,
//! route a logical message through the RTI, and deliver it at `Tag::ZERO`.

use std::{
    sync::{mpsc, Arc, Mutex},
    time::Duration as StdDuration,
};

use boomerang::prelude::*;

#[cfg(feature = "rerun")]
fn rerun_chunks(session: &boomerang::rerun::RerunSession) -> Vec<rerun::log::Chunk> {
    session
        .memory_sink()
        .take()
        .into_iter()
        .filter_map(|message| match message {
            rerun::log::LogMsg::ArrowMsg(_, message) => {
                Some(rerun::log::Chunk::from_chunk_record_batch(&message.batch).unwrap())
            }
            _ => None,
        })
        .collect()
}

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
    #[cfg(feature = "rerun")]
    let rerun = boomerang::rerun::RerunSessionBuilder::new("federated-static-test")
        .build()
        .unwrap();
    assembly
        .register_federated_codec::<u32, _>(boomerang::federated::SerdeJsonCodec)
        .unwrap();

    let _ = StaticFederation(Arc::clone(&values))
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
    #[cfg(feature = "rerun")]
    rerun.register_runtime(&parts);
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
    assert_eq!(federation.graph().federate_ids().count(), 2);
    assert_eq!(federation.graph().endpoint_ids().count(), 1);
    #[cfg(feature = "rerun")]
    {
        let chunks = rerun_chunks(&rerun);
        let paths = chunks
            .iter()
            .map(|chunk| chunk.entity_path().to_string())
            .collect::<Vec<_>>();

        assert!(paths.iter().any(|path| path == "/federates/a"));
        assert!(paths.iter().any(|path| path == "/federates/b"));
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.ends_with("/scheduler"))
                .count(),
            3,
            "one scheduler entity per lowered Enclave"
        );
        assert!(paths.iter().any(|path| {
            path.starts_with("/federates/a/enclaves/")
                && path.contains("/reactors/main/reactors/a/reactors/source/reactions/")
        }));
        assert!(paths.iter().any(|path| {
            path.starts_with("/federates/b/enclaves/")
                && path.contains("/reactors/main/reactors/b/actions/")
        }));

        let component_suffixes = chunks
            .iter()
            .flat_map(|chunk| chunk.component_descriptors())
            .map(|descriptor| descriptor.component.to_string())
            .collect::<Vec<_>>();
        for required in [
            ":boomerang.runtime.display_name",
            ":boomerang.runtime.stable_key",
            ":boomerang.runtime.kind",
            ":boomerang.runtime.owner_key",
            ":boomerang.runtime.type",
            ":boomerang.runtime.action_timing",
            ":boomerang.runtime.reaction_level",
            ":boomerang.runtime.source",
            ":boomerang.runtime.target",
            ":boomerang.runtime.relation_kind",
        ] {
            assert!(
                component_suffixes
                    .iter()
                    .any(|component| component.ends_with(required)),
                "missing static component {required}"
            );
        }
        assert!(chunks
            .iter()
            .any(
                |chunk| chunk.component_descriptors().any(|descriptor| descriptor
                    .archetype
                    .as_ref()
                    .is_some_and(|name| name.as_str() == "rerun.archetypes.GraphNodes"))
            ));
        assert!(chunks
            .iter()
            .any(
                |chunk| chunk.component_descriptors().any(|descriptor| descriptor
                    .archetype
                    .as_ref()
                    .is_some_and(|name| name.as_str() == "rerun.archetypes.GraphEdges"))
            ));
        assert!(paths
            .iter()
            .any(|path| path.starts_with("/federation/topology/ownership/")));
        let endpoint = chunks
            .iter()
            .find(|chunk| {
                chunk
                    .entity_path()
                    .to_string()
                    .starts_with("/federation/topology/endpoints/")
            })
            .expect("lowered federated endpoint relation");
        for required in [
            ":boomerang.runtime.stable_key",
            ":boomerang.runtime.delay_ns",
            ":boomerang.runtime.source",
            ":boomerang.runtime.target",
        ] {
            assert!(endpoint
                .component_descriptors()
                .any(|descriptor| descriptor.component.as_str().ends_with(required)));
        }
        assert!(
            chunks
                .iter()
                .filter(|chunk| {
                    let path = chunk.entity_path().to_string();
                    path.starts_with("/federates/") || path.starts_with("/federation/")
                })
                .all(|chunk| chunk.timelines().is_empty()),
            "static registration must be timeless"
        );
        assert!(rerun.is_enabled());
        assert_eq!(rerun.error_count(), 0);
    }
    let envs = execute_federation_in_memory(parts.into_federation().unwrap(), config).unwrap();
    let a_envs = &envs[&FederateId::new("a")];
    let b_envs = &envs[&FederateId::new("b")];
    assert_eq!(a_envs.keys().next(), b_envs.keys().next());

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
    let (graph, federates) = federation.into_parts();
    assert_eq!(graph.endpoint_ids().count(), 1);
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
