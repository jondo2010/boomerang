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
fn decode_chunks(
    messages: &[rerun::log::LogMsg],
    store_kind: Option<rerun::StoreKind>,
) -> Vec<rerun::log::Chunk> {
    messages
        .iter()
        .filter_map(|message| match message {
            rerun::log::LogMsg::ArrowMsg(store_id, message)
                if store_kind.is_none_or(|kind| store_id.kind() == kind) =>
            {
                Some(rerun::log::Chunk::from_chunk_record_batch(&message.batch).unwrap())
            }
            _ => None,
        })
        .collect()
}

#[cfg(feature = "rerun")]
fn rows(chunks: &[rerun::log::Chunk]) -> impl Iterator<Item = (&rerun::log::Chunk, usize)> {
    chunks
        .iter()
        .flat_map(|chunk| (0..chunk.num_rows()).map(move |row| (chunk, row)))
}

#[cfg(feature = "rerun")]
fn text_component_at(chunk: &rerun::log::Chunk, suffix: &str, row: usize) -> Option<String> {
    let descriptor = chunk
        .component_descriptors()
        .find(|descriptor| descriptor.component.as_str().ends_with(suffix))?;
    let values = chunk.component_batch_raw(descriptor.component, row)?.ok()?;
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()?;
    (values.len() == 1).then(|| values.value(0).to_owned())
}

#[cfg(feature = "rerun")]
fn text_components_at(chunk: &rerun::log::Chunk, suffix: &str, row: usize) -> Vec<String> {
    let Some(descriptor) = chunk
        .component_descriptors()
        .find(|descriptor| descriptor.component.as_str().ends_with(suffix))
    else {
        return Vec::new();
    };
    let Some(Ok(values)) = chunk.component_batch_raw(descriptor.component, row) else {
        return Vec::new();
    };
    let Some(values) = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()
    else {
        return Vec::new();
    };
    values.iter().flatten().map(str::to_owned).collect()
}

#[cfg(feature = "rerun")]
fn graph_edge_at(chunk: &rerun::log::Chunk, row: usize) -> Option<(String, String)> {
    let descriptor = chunk.component_descriptors().find(|descriptor| {
        descriptor
            .component_type
            .as_ref()
            .is_some_and(|name| name.as_str() == "rerun.components.GraphEdge")
    })?;
    let values = chunk.component_batch_raw(descriptor.component, row)?.ok()?;
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

#[cfg(feature = "rerun")]
fn state_change_at(chunk: &rerun::log::Chunk, row: usize) -> Option<Option<String>> {
    let descriptor = chunk.component_descriptors().find(|descriptor| {
        descriptor
            .archetype
            .as_ref()
            .is_some_and(|archetype| archetype.as_str() == "rerun.archetypes.StateChange")
    })?;
    let values = chunk.component_batch_raw(descriptor.component, row)?.ok()?;
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()?;
    Some(
        (values.len() == 1 && !values.is_null(0) && !values.value(0).is_empty())
            .then(|| values.value(0).to_owned()),
    )
}

#[cfg(feature = "rerun")]
fn scalar_at(chunk: &rerun::log::Chunk, row: usize) -> Option<f64> {
    let descriptor = chunk.component_descriptors().find(|descriptor| {
        descriptor
            .archetype
            .as_ref()
            .is_some_and(|archetype| archetype.as_str() == "rerun.archetypes.Scalars")
    })?;
    let values = chunk.component_batch_raw(descriptor.component, row)?.ok()?;
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::Float64Array>()?;
    (values.len() == 1).then(|| values.value(0))
}

#[cfg(feature = "rerun")]
fn uint_component_at(chunk: &rerun::log::Chunk, suffix: &str, row: usize) -> Option<u64> {
    let descriptor = chunk
        .component_descriptors()
        .find(|descriptor| descriptor.component.as_str().ends_with(suffix))?;
    let values = chunk.component_batch_raw(descriptor.component, row)?.ok()?;
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::UInt64Array>()?;
    (values.len() == 1).then(|| values.value(0))
}

#[cfg(feature = "rerun")]
fn bool_component_at(chunk: &rerun::log::Chunk, suffix: &str, row: usize) -> Option<bool> {
    let descriptor = chunk
        .component_descriptors()
        .find(|descriptor| descriptor.component.as_str().ends_with(suffix))?;
    let values = chunk.component_batch_raw(descriptor.component, row)?.ok()?;
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::BooleanArray>()?;
    (values.len() == 1).then(|| values.value(0))
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
fn public_api_runs_static_federation_with_finalized_rrd_trace() {
    boomerang_util::test_tracing::init_with_directive("warn");
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    #[cfg(feature = "rerun")]
    let directory = tempfile::tempdir().unwrap();
    #[cfg(feature = "rerun")]
    let rrd_path = directory.path().join("federated-static.rrd");
    #[cfg(feature = "rerun")]
    let rerun = boomerang::rerun::RerunSessionBuilder::new("federated-static-test")
        .sink(boomerang::rerun::SinkConfig::File(rrd_path.clone()))
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
    assert_eq!(federation.graph().endpoint_ids().count(), 1);

    #[cfg(feature = "rerun")]
    let envs = {
        let subscriber = tracing_subscriber::registry().with(rerun.layer());
        tracing::subscriber::with_default(subscriber, || {
            execute_federation_in_memory(parts.into_federation().unwrap(), config).unwrap()
        })
    };
    #[cfg(not(feature = "rerun"))]
    let envs = execute_federation_in_memory(parts.into_federation().unwrap(), config).unwrap();

    assert_eq!(
        envs[&FederateId::new("a")].keys().next(),
        envs[&FederateId::new("b")].keys().next()
    );
    assert_eq!(*values.lock().unwrap(), vec![(Tag::ZERO, 7)]);

    #[cfg(feature = "rerun")]
    {
        assert_eq!(rerun.error_count(), 0);
        rerun.finish().unwrap();

        let file = std::io::BufReader::new(std::fs::File::open(&rrd_path).unwrap());
        let messages = rerun::external::re_log_encoding::DecoderApp::decode_lazy(file)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let chunks = decode_chunks(&messages, Some(rerun::StoreKind::Recording));

        let static_topology = chunks.iter().filter(|chunk| {
            chunk.entity_path().to_string().ends_with("/topology")
                || chunk.component_descriptors().any(|descriptor| {
                    descriptor
                        .component
                        .as_str()
                        .starts_with("boomerang.runtime.")
                })
        });
        let mut static_topology_chunks = 0;
        let mut graph_labels =
            std::collections::HashMap::<String, std::collections::HashSet<String>>::new();
        for chunk in static_topology {
            static_topology_chunks += 1;
            assert!(chunk.is_static(), "{} must be static", chunk.entity_path());
            assert!(chunk.timelines().is_empty());
            for label in (0..chunk.num_rows())
                .flat_map(|row| text_components_at(chunk, "GraphNodes:labels", row))
            {
                assert!(
                    !label.contains('/'),
                    "graph label exposes entity path: {label}"
                );
                assert!(label.len() <= 64, "graph label is too long: {label}");
            }
            for row in 0..chunk.num_rows() {
                let ids = text_components_at(chunk, "GraphNodes:node_ids", row);
                let labels = text_components_at(chunk, "GraphNodes:labels", row);
                for (id, label) in ids.into_iter().zip(labels) {
                    graph_labels.entry(label).or_default().insert(id);
                }
            }
        }
        assert!(static_topology_chunks > 0);
        assert!(graph_labels.values().all(|ids| ids.len() == 1));

        for (label, kind, reactor, child) in [
            (
                "source output",
                "owns_port",
                "/reactors/main/reactors/a/reactors/source@",
                "/ports/",
            ),
            (
                "source startup",
                "owns_action",
                "/reactors/main/reactors/a/reactors/source@",
                "/actions/",
            ),
            (
                "relay input",
                "owns_port",
                "/reactors/main/reactors/a/reactors/relay@",
                "/ports/",
            ),
            (
                "relay startup",
                "owns_action",
                "/reactors/main/reactors/a/reactors/relay@",
                "/actions/",
            ),
            (
                "sink startup",
                "owns_action",
                "/federates/b/enclaves/EnclaveKey(0)/reactors/main/reactors/b@",
                "/actions/",
            ),
            (
                "sink inbound adapter",
                "owns_port",
                "/federates/b/enclaves/EnclaveKey(0)/reactors/main/reactors/con_reactor_tgt@",
                "/ports/",
            ),
        ] {
            assert!(
                rows(&chunks).any(|(chunk, row)| {
                    text_component_at(chunk, ":boomerang.runtime.relation_kind", row).as_deref()
                        == Some(kind)
                        && text_component_at(chunk, ":boomerang.runtime.source", row)
                            .is_some_and(|source| source.contains(reactor))
                        && text_component_at(chunk, ":boomerang.runtime.target", row).is_some_and(
                            |target| target.contains(reactor) && target.contains(child),
                        )
                }),
                "decoded {label} {kind} relation for {reactor}"
            );
        }

        let runtime_rows = rows(&chunks)
            .filter(|(chunk, row)| {
                text_component_at(chunk, ":boomerang.trace.event", *row).is_some()
            })
            .collect::<Vec<_>>();
        assert!(runtime_rows.iter().any(|(chunk, row)| {
            chunk.timelines().values().any(|timeline| {
                timeline.name() == "logical" && timeline.times_raw().get(*row).is_some()
            })
        }));

        let (send, send_row) = runtime_rows
            .iter()
            .copied()
            .find(|(chunk, row)| {
                text_component_at(chunk, ":boomerang.trace.event", *row).as_deref()
                    == Some("propagation_send")
                    && text_component_at(chunk, ":boomerang.trace.federate", *row).as_deref()
                        == Some("a")
                    && text_component_at(chunk, ":boomerang.trace.destination_federate", *row)
                        .as_deref()
                        == Some("b")
            })
            .expect("serialized A-to-B propagation send");
        assert!(
            text_component_at(send, ":boomerang.trace.destination", send_row).is_none(),
            "serialized send is intentionally ambiguous until its causal edge is decoded"
        );
        let propagation_size =
            uint_component_at(send, ":boomerang.trace.value_size", send_row).unwrap();
        assert_eq!(
            scalar_at(send, send_row),
            Some(propagation_size as f64),
            "propagation value-size measure is co-located with its typed event"
        );

        let (action, action_row) = runtime_rows
            .iter()
            .copied()
            .find(|(chunk, row)| {
                text_component_at(chunk, ":boomerang.trace.event", *row).as_deref()
                    == Some("action_schedule")
                    && uint_component_at(chunk, ":boomerang.trace.value_size", *row).is_some()
            })
            .expect("scheduled action with a value-size fact");
        let action_size =
            uint_component_at(action, ":boomerang.trace.value_size", action_row).unwrap();
        assert_eq!(
            scalar_at(action, action_row),
            Some(action_size as f64),
            "action value-size measure is co-located with its typed event"
        );

        let (terminal, terminal_row) = runtime_rows
            .iter()
            .copied()
            .find(|(chunk, row)| {
                bool_component_at(chunk, ":boomerang.trace.terminal", *row).is_some()
            })
            .expect("terminal lifecycle fact");
        let terminal_value =
            bool_component_at(terminal, ":boomerang.trace.terminal", terminal_row).unwrap();
        assert_eq!(
            scalar_at(terminal, terminal_row),
            Some(if terminal_value { 1.0 } else { 0.0 }),
            "terminal measure is co-located with its typed event"
        );

        let links = runtime_rows
            .iter()
            .copied()
            .filter(|(chunk, row)| {
                text_component_at(chunk, ":boomerang.trace.event", *row).as_deref()
                    == Some("causal_link")
            })
            .map(|(chunk, row)| {
                let source = text_component_at(chunk, ":boomerang.trace.source", row).unwrap();
                let destination =
                    text_component_at(chunk, ":boomerang.trace.destination", row).unwrap();
                assert_eq!(
                    graph_edge_at(chunk, row),
                    Some((source.clone(), destination.clone()))
                );
                (source, destination)
            })
            .collect::<Vec<_>>();
        let exact_destination = |source: &str| {
            let destinations = links
                .iter()
                .filter_map(|(candidate, destination)| {
                    (candidate == source).then_some(destination.as_str())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                destinations.len(),
                1,
                "{source} must have one exact causal destination"
            );
            destinations[0]
        };

        let send_id = text_component_at(send, ":boomerang.trace.id", send_row).unwrap();
        let receive_id = exact_destination(&send_id);
        let (receive, receive_row) = runtime_rows
            .iter()
            .copied()
            .find(|(chunk, row)| {
                text_component_at(chunk, ":boomerang.trace.id", *row).as_deref() == Some(receive_id)
                    && text_component_at(chunk, ":boomerang.trace.event", *row).as_deref()
                        == Some("propagation_receive")
            })
            .expect("causal destination is the remote receive");
        assert_eq!(
            text_component_at(receive, ":boomerang.trace.federate", receive_row).as_deref(),
            Some("b")
        );

        let reaction_id = exact_destination(receive_id);
        let (reaction, reaction_row) = runtime_rows
            .iter()
            .copied()
            .find(|(chunk, row)| {
                text_component_at(chunk, ":boomerang.trace.id", *row).as_deref()
                    == Some(reaction_id)
                    && text_component_at(chunk, ":boomerang.trace.event", *row).as_deref()
                        == Some("reaction_execute")
            })
            .expect("remote receive causally triggers one reaction");
        assert_eq!(
            text_component_at(reaction, ":boomerang.trace.federate", reaction_row).as_deref(),
            Some("b")
        );
        let duration_ns = uint_component_at(reaction, ":boomerang.trace.duration_ns", reaction_row)
            .expect("reaction duration component");
        let duration_measure =
            scalar_at(reaction, reaction_row).expect("reaction operational duration scalar");
        assert!(duration_measure.is_finite() && duration_measure >= 0.0);
        assert_eq!(duration_measure, duration_ns as f64);

        let mut reaction_states = rows(&chunks)
            .filter_map(|(chunk, row)| {
                (chunk.entity_path() == reaction.entity_path()).then(|| {
                    let state = state_change_at(chunk, row)?;
                    let log_time = chunk
                        .timelines()
                        .values()
                        .find(|timeline| timeline.name() == "log_time")?
                        .times_raw()
                        .get(row)
                        .copied()?;
                    assert!(chunk.timelines().values().all(|timeline| {
                        timeline.name() != "logical" || timeline.times_raw().get(row).is_none()
                    }));
                    Some((log_time, state))
                })?
            })
            .collect::<Vec<_>>();
        reaction_states.sort_by_key(|(log_time, _)| *log_time);
        let reaction_states = reaction_states
            .into_iter()
            .map(|(_, state)| state)
            .collect::<Vec<_>>();
        assert!(reaction_states
            .windows(2)
            .any(|states| { states == [Some("executing reaction".to_owned()), None] }));

        let blueprint = decode_chunks(&messages, Some(rerun::StoreKind::Blueprint));
        for (name, class) in [
            ("Scheduler phase spans (wall clock)", "StateTimeline"),
            ("Ownership and propagation", "Graph"),
            ("Logical phases and measures", "TimeSeries"),
        ] {
            assert!(rows(&blueprint).any(|(chunk, row)| {
                text_component_at(chunk, "ViewBlueprint:display_name", row).as_deref() == Some(name)
                    && text_component_at(chunk, "ViewBlueprint:class_identifier", row).as_deref()
                        == Some(class)
            }));
        }
        assert!(rows(&blueprint).any(|(chunk, row)| {
            chunk.entity_path().to_string() == "/time_panel"
                && text_component_at(chunk, "TimePanelBlueprint:timeline", row).as_deref()
                    == Some("logical")
        }));
        let queries = rows(&blueprint)
            .flat_map(|(chunk, row)| text_components_at(chunk, "ViewContents:query", row))
            .collect::<Vec<_>>();
        assert!(
            queries.iter().any(|query| query.ends_with("/enclaves/**")),
            "default blueprint must include the complete enclave subtree: {queries:?}"
        );
        assert!(queries.iter().any(|query| query == "/propagation/**"));
        assert!(
            queries.iter().all(|query| !query.contains("/**/")),
            "Rerun only supports recursive wildcards as the final path segment: {queries:?}"
        );
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
