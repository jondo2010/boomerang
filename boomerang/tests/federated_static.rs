#![cfg(feature = "federated")]
//! Proves the public `boomerang` API can build and execute static federations,
//! route a logical message through the RTI, and deliver it at `Tag::ZERO`.

use std::{
    sync::{mpsc, Arc, Mutex},
    time::Duration as StdDuration,
};

use boomerang::prelude::*;
#[cfg(feature = "rerun")]
use rerun::external::arrow::array::Array as _;
#[cfg(feature = "rerun")]
use tracing_subscriber::prelude::*;

#[cfg(feature = "rerun")]
fn rerun_chunks(session: &boomerang::rerun::RerunSession) -> Vec<rerun::log::Chunk> {
    session
        .take_memory_snapshot_bounded()
        .expect("memory sink")
        .into_iter()
        .filter_map(|message| match message {
            rerun::log::LogMsg::ArrowMsg(_, message) => {
                Some(rerun::log::Chunk::from_chunk_record_batch(&message.batch).unwrap())
            }
            _ => None,
        })
        .collect()
}

#[cfg(feature = "rerun")]
fn text_component(chunk: &rerun::log::Chunk, suffix: &str) -> Option<String> {
    let descriptor = chunk
        .component_descriptors()
        .find(|descriptor| descriptor.component.as_str().ends_with(suffix))?;
    let values = chunk.component_batch_raw(descriptor.component, 0)?.ok()?;
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()?;
    (values.len() == 1).then(|| values.value(0).to_owned())
}

#[cfg(feature = "rerun")]
fn graph_edge(chunk: &rerun::log::Chunk) -> Option<(String, String)> {
    let descriptor = chunk.component_descriptors().find(|descriptor| {
        descriptor
            .component_type
            .as_ref()
            .is_some_and(|name| name.as_str() == "rerun.components.GraphEdge")
    })?;
    let values = chunk.component_batch_raw(descriptor.component, 0)?.ok()?;
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StructArray>()?;
    let first = values
        .column_by_name("first")?
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()?;
    let second = values
        .column_by_name("second")?
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()?;
    (values.len() == 1).then(|| (first.value(0).to_owned(), second.value(0).to_owned()))
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
        let subscriber = tracing_subscriber::registry().with(rerun.layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(
                target: "boomerang::trace",
                event = "port_write",
                enclave = %runtime::EnclaveKey::from(1),
                port_key = %runtime::PortKey::from(0),
                outcome = "test",
            );
            tracing::trace!(
                target: "boomerang::trace",
                event = "action_schedule",
                enclave = %runtime::EnclaveKey::from(0),
                action_key = %runtime::ActionKey::from(0),
                outcome = "test",
            );
            tracing::trace!(
                target: "boomerang::trace",
                event = "propagation_send",
                enclave = %runtime::EnclaveKey::from(1),
                destination = %runtime::EnclaveKey::from(0),
                action_key = %runtime::ActionKey::from(0),
                outcome = "test",
            );
        });
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
        assert!(
            paths.iter().any(|path| {
                path.starts_with("/federates/a/enclaves/")
                    && path
                        .contains("/reactors/main/reactors/a/reactors/source\\@ReactorKey\\(1\\)")
                    && path.contains("/reactions/")
            }),
            "registered paths: {paths:#?}"
        );
        assert!(paths.iter().any(|path| {
            path.starts_with("/federates/b/enclaves/")
                && path.contains("/reactors/main/reactors/b\\@ReactorKey\\(")
                && path.contains("/actions/")
        }));
        let aligned_port_event = paths
            .iter()
            .find(|path| path.ends_with("/ports/PortKey\\(0\\)/port_write"))
            .expect("unambiguous dynamic port event");
        assert!(aligned_port_event.starts_with("/federates/a/enclaves/EnclaveKey\\(1\\)/"));
        assert!(aligned_port_event.contains("/reactors/main/reactors/a/reactors/relay\\@"));
        let ambiguous_action_event = paths
            .iter()
            .find(|path| path.ends_with("/actions/ActionKey\\(0\\)/action_schedule"))
            .expect("ambiguous dynamic action event");
        assert_eq!(
            ambiguous_action_event,
            "/enclaves/EnclaveKey\\(0\\)/actions/ActionKey\\(0\\)/action_schedule"
        );
        assert!(paths
            .iter()
            .any(|path| path.starts_with("/propagation/sends/")));
        assert!(!paths
            .iter()
            .any(|path| { path.ends_with("/propagation_send") && path.contains("/actions/") }));

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
        let topology_paths = chunks
            .iter()
            .filter_map(|chunk| {
                let archetypes = chunk
                    .component_descriptors()
                    .filter_map(|descriptor| descriptor.archetype.as_ref())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                (!archetypes.is_empty()).then(|| (chunk.entity_path().to_string(), archetypes))
            })
            .collect::<Vec<_>>();
        assert!(topology_paths.iter().any(|(path, archetypes)| {
            path.ends_with("/topology")
                && archetypes
                    .iter()
                    .any(|name| name == "rerun.archetypes.GraphNodes")
        }));
        assert!(topology_paths.iter().any(|(path, archetypes)| {
            path.ends_with("/topology")
                && archetypes
                    .iter()
                    .any(|name| name == "rerun.archetypes.GraphEdges")
        }));
        assert!(!topology_paths.iter().any(|(path, _)| {
            path.ends_with("/topology/nodes") || path.ends_with("/topology/edges")
        }));
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
        assert_eq!(
            text_component(endpoint, ":boomerang.runtime.source").as_deref(),
            Some("/federates/a")
        );
        assert_eq!(
            text_component(endpoint, ":boomerang.runtime.target").as_deref(),
            Some("/federates/b")
        );
        assert_eq!(
            text_component(endpoint, ":boomerang.runtime.relation_kind").as_deref(),
            Some("federated_endpoint")
        );
        let ownership = chunks
            .iter()
            .find(|chunk| {
                text_component(chunk, ":boomerang.runtime.relation_kind").as_deref()
                    == Some("owns_port")
                    && text_component(chunk, ":boomerang.runtime.target").as_deref()
                        == Some(
                            "/federates/a/enclaves/EnclaveKey(0)/reactors/main/reactors/a/reactors/source@ReactorKey(1)/ports/PortKey(0)",
                        )
            })
            .expect("exact source port ownership relation");
        assert_eq!(
            text_component(ownership, ":boomerang.runtime.source").as_deref(),
            Some(
                "/federates/a/enclaves/EnclaveKey(0)/reactors/main/reactors/a/reactors/source@ReactorKey(1)"
            )
        );
        let trigger = chunks
            .iter()
            .find(|chunk| {
                text_component(chunk, ":boomerang.runtime.relation_kind").as_deref()
                    == Some("triggers")
                    && text_component(chunk, ":boomerang.runtime.source").as_deref()
                        == Some(
                            "/federates/a/enclaves/EnclaveKey(1)/reactors/main/reactors/a/reactors/relay@ReactorKey(0)/ports/PortKey(0)",
                        )
                    && text_component(chunk, ":boomerang.runtime.target").as_deref()
                        == Some(
                            "/federates/a/enclaves/EnclaveKey(1)/reactors/main/reactors/a/reactors/con_reactor_src@ReactorKey(2)/reactions/ReactionKey(3)",
                        )
            })
            .expect("exact relay port trigger relation");
        assert_eq!(
            text_component(trigger, ":boomerang.runtime.relation_kind").as_deref(),
            Some("triggers")
        );
        assert!(
            chunks
                .iter()
                .filter(|chunk| {
                    chunk.component_descriptors().any(|descriptor| {
                        descriptor.archetype.as_ref().is_some_and(|name| {
                            matches!(
                                name.as_str(),
                                "boomerang.RuntimeEntity"
                                    | "boomerang.RuntimeRelation"
                                    | "rerun.archetypes.GraphNodes"
                                    | "rerun.archetypes.GraphEdges"
                            )
                        })
                    })
                })
                .all(|chunk| chunk.timelines().is_empty()),
            "static registration must be timeless"
        );
        assert!(rerun.is_enabled());
        assert_eq!(rerun.error_count(), 0);
    }
    #[cfg(feature = "rerun")]
    let envs = {
        let subscriber = tracing_subscriber::registry().with(rerun.layer());
        tracing::subscriber::with_default(subscriber, || {
            execute_federation_in_memory(parts.into_federation().unwrap(), config).unwrap()
        })
    };
    #[cfg(not(feature = "rerun"))]
    let envs = execute_federation_in_memory(parts.into_federation().unwrap(), config).unwrap();
    let a_envs = &envs[&FederateId::new("a")];
    let b_envs = &envs[&FederateId::new("b")];
    assert_eq!(a_envs.keys().next(), b_envs.keys().next());

    assert_eq!(*values.lock().unwrap(), vec![(Tag::ZERO, 7)]);

    #[cfg(feature = "rerun")]
    {
        let runtime_chunks = rerun_chunks(&rerun);
        let runtime_chunks = runtime_chunks
            .iter()
            .filter(|chunk| !chunk.timelines().is_empty())
            .collect::<Vec<_>>();
        assert!(
            !runtime_chunks.is_empty(),
            "scheduler emitted no trace records"
        );
        assert!(runtime_chunks.iter().any(|chunk| {
            chunk
                .timelines()
                .values()
                .any(|timeline| timeline.name() == "logical")
        }));

        let scheduler_lanes = runtime_chunks
            .iter()
            .filter_map(|chunk| {
                let path = chunk.entity_path().to_string();
                let segments = path.split('/').collect::<Vec<_>>();
                let enclave = segments.iter().position(|segment| *segment == "enclaves")?;
                Some(segments[..=enclave + 1].join("/"))
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            scheduler_lanes
                .iter()
                .filter(|lane| lane.starts_with("/federates/a/enclaves/"))
                .count(),
            2,
            "expected both lowered scheduler lanes in federate A: {scheduler_lanes:?}"
        );
        assert_eq!(
            scheduler_lanes
                .iter()
                .filter(|lane| lane.starts_with("/federates/b/enclaves/"))
                .count(),
            1,
            "expected the lowered scheduler lane in federate B: {scheduler_lanes:?}"
        );
        let causal_links = runtime_chunks
            .iter()
            .filter(|chunk| {
                text_component(chunk, ":boomerang.trace.event").as_deref() == Some("causal_link")
            })
            .collect::<Vec<_>>();
        assert!(
            !causal_links.is_empty(),
            "the exercised federated propagation produced no exact causal link"
        );
        let records = runtime_chunks
            .iter()
            .filter_map(|chunk| {
                Some((
                    text_component(chunk, ":boomerang.trace.id")?,
                    text_component(chunk, ":boomerang.trace.event")?,
                ))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let links = causal_links
            .iter()
            .map(|link| {
                let source = text_component(link, ":boomerang.trace.source")
                    .expect("causal source adapter ID");
                let destination = text_component(link, ":boomerang.trace.destination")
                    .expect("causal destination adapter ID");
                let graph = runtime_chunks
                    .iter()
                    .find(|chunk| {
                        chunk.entity_path() == link.entity_path() && graph_edge(chunk).is_some()
                    })
                    .and_then(|chunk| graph_edge(chunk))
                    .expect("co-located built-in GraphEdges endpoint pair");
                assert_eq!(graph, (source.clone(), destination.clone()));
                assert!(
                    records.contains_key(&source),
                    "unknown causal source {source}"
                );
                assert!(
                    records.contains_key(&destination),
                    "unknown causal destination {destination}"
                );
                (source, destination)
            })
            .collect::<Vec<_>>();
        let receives = records
            .iter()
            .filter(|(_, event)| event.as_str() == "propagation_receive")
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert!(receives.iter().any(|receive| {
            let has_send = links.iter().any(|(source, destination)| {
                destination == *receive
                    && records.get(source).map(String::as_str) == Some("propagation_send")
            });
            let has_reaction = links.iter().any(|(source, destination)| {
                source == *receive
                    && records.get(destination).map(String::as_str) == Some("reaction_execute")
            });
            has_send && has_reaction
        }), "no complete propagation_send -> propagation_receive -> reaction_execute chain: records={records:?}, links={links:?}");
    }
}

#[test]
fn public_api_rejects_runtime_without_lowered_federation() {
    let parts = RuntimeAssembly::default();

    assert!(matches!(
        parts.into_federation(),
        Err(RuntimeExecutionError::ExpectedFederation)
    ));
}

#[cfg(feature = "rerun")]
#[test]
fn unsupported_rerun_grpc_config_rejection_does_not_change_federation_output() {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
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
    let rerun = boomerang::rerun::RerunSessionBuilder::new("federated-grpc-isolation")
        .sink(boomerang::rerun::SinkConfig::Grpc {
            url: "rerun+http://127.0.0.1:9/proxy".to_owned(),
            memory_limit_bytes: 64 * 1024,
        })
        .blueprint(boomerang::rerun::BlueprintConfig::None)
        .flush_timeout(StdDuration::from_millis(10))
        .build();
    assert!(matches!(
        rerun,
        Err(boomerang::rerun::RerunSessionBuildError::UnsupportedGrpc)
    ));

    let envs = execute_federation_in_memory(parts.into_federation().unwrap(), config).unwrap();
    let a_envs = &envs[&FederateId::new("a")];
    let b_envs = &envs[&FederateId::new("b")];
    assert_eq!(a_envs.keys().next(), b_envs.keys().next());
    assert_eq!(*values.lock().unwrap(), vec![(Tag::ZERO, 7)]);
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
