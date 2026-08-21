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
fn rerun_chunks(
    session: &boomerang::rerun::RerunSession,
    messages: &mut Vec<rerun::log::LogMsg>,
) -> Vec<rerun::log::Chunk> {
    messages.extend(session.take_memory_snapshot_bounded().expect("memory sink"));
    decode_chunks(messages, Some(rerun::StoreKind::Recording))
}

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
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecordingRowSemantics {
    entity_path: String,
    archetypes: Vec<String>,
    trace_fields: Vec<(String, String)>,
    timelines: Vec<(String, i64)>,
}

#[cfg(feature = "rerun")]
fn normalize_recording_rows(
    batches: impl IntoIterator<Item = Vec<RecordingRowSemantics>>,
) -> Vec<RecordingRowSemantics> {
    let mut rows = batches.into_iter().flatten().collect::<Vec<_>>();
    rows.sort();
    rows
}

#[cfg(feature = "rerun")]
fn normalized_present_archetypes<'a>(
    components: impl IntoIterator<Item = (Option<&'a str>, bool)>,
) -> Vec<String> {
    let mut archetypes = components
        .into_iter()
        .filter_map(|(archetype, present)| present.then_some(archetype).flatten())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    archetypes.sort();
    archetypes.dedup();
    archetypes
}

#[cfg(feature = "rerun")]
fn recording_semantics(chunks: &[rerun::log::Chunk]) -> Vec<RecordingRowSemantics> {
    normalize_recording_rows(chunks.iter().map(|chunk| {
        (0..chunk.num_rows())
            .map(|row| {
                let archetypes = normalized_present_archetypes(chunk.component_descriptors().map(
                    |descriptor| {
                        (
                            descriptor.archetype.as_ref().map(|name| name.as_str()),
                            chunk
                                .component_batch_raw(descriptor.component, row)
                                .is_some(),
                        )
                    },
                ));
                let trace_fields = [
                    ":boomerang.trace.event",
                    ":boomerang.trace.id",
                    ":boomerang.trace.source",
                    ":boomerang.trace.destination",
                ]
                .into_iter()
                .filter_map(|suffix| {
                    text_component_at(chunk, suffix, row).map(|value| (suffix.to_owned(), value))
                })
                .collect();
                let mut timelines = chunk
                    .timelines()
                    .values()
                    .filter_map(|column| {
                        column
                            .times_raw()
                            .get(row)
                            .copied()
                            .map(|time| (column.name().to_owned(), time))
                    })
                    .collect::<Vec<_>>();
                timelines.sort();
                RecordingRowSemantics {
                    entity_path: chunk.entity_path().to_string(),
                    archetypes,
                    trace_fields,
                    timelines,
                }
            })
            .collect()
    }))
}

#[cfg(feature = "rerun")]
fn assert_recording_row_normalization_is_independent_of_chunk_batching() {
    let schema_union = [
        Some("boomerang.TraceRecord"),
        Some("rerun.archetypes.GraphEdges"),
        None,
    ];
    assert_eq!(
        normalized_present_archetypes(schema_union.into_iter().zip([true, false, true])),
        vec!["boomerang.TraceRecord"]
    );
    assert_eq!(
        normalized_present_archetypes(schema_union.into_iter().zip([false, true, true])),
        vec!["rerun.archetypes.GraphEdges"]
    );

    let row = |event: &str, logical: i64| RecordingRowSemantics {
        entity_path: "/enclaves/e0/reactions/r0".to_owned(),
        archetypes: vec!["boomerang.TraceRecord".to_owned()],
        trace_fields: vec![(":boomerang.trace.event".to_owned(), event.to_owned())],
        timelines: vec![("logical".to_owned(), logical)],
    };
    let first = row("first", 1);
    let later = row("later", 2);
    let duplicate = later.clone();

    let split = normalize_recording_rows(vec![
        vec![first.clone()],
        vec![later.clone(), duplicate.clone()],
    ]);
    let merged = normalize_recording_rows(vec![vec![duplicate, first, later.clone()]]);

    assert_eq!(split, merged);
    assert_eq!(merged.len(), 3, "row multiplicity must be preserved");
    assert_eq!(
        merged
            .iter()
            .filter(|row| row.trace_fields[0].1 == "later")
            .count(),
        2,
        "later rows must not be ignored"
    );
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
fn text_component(chunk: &rerun::log::Chunk, suffix: &str) -> Option<String> {
    text_component_at(chunk, suffix, 0)
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
    let directory = tempfile::tempdir().unwrap();
    #[cfg(feature = "rerun")]
    let rrd_path = directory.path().join("federated-static.rrd");
    #[cfg(feature = "rerun")]
    let mut memory_messages = Vec::new();
    #[cfg(feature = "rerun")]
    let rerun = boomerang::rerun::RerunSessionBuilder::new("federated-static-test")
        .sink(boomerang::rerun::SinkConfig::Tee(vec![
            boomerang::rerun::SinkConfig::Memory,
            boomerang::rerun::SinkConfig::File(rrd_path.clone()),
        ]))
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
                federate = "a",
                enclave = %runtime::EnclaveKey::from(1),
                port_key = %runtime::PortKey::from(0),
                outcome = "test",
            );
            tracing::trace!(
                target: "boomerang::trace",
                event = "action_schedule",
                federate = "a",
                enclave = %runtime::EnclaveKey::from(0),
                action_key = %runtime::ActionKey::from(0),
                outcome = "test",
            );
        });
        let chunks = rerun_chunks(&rerun, &mut memory_messages);
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
        assert!(aligned_port_event
            .contains("/reactors/main/reactors/a/reactors/relay\\@ReactorKey\\(0\\)/ports/"));
        let aligned_action_event = paths
            .iter()
            .find(|path| path.ends_with("/actions/ActionKey\\(0\\)/action_schedule"))
            .expect("federate-qualified dynamic action event");
        assert!(aligned_action_event.starts_with("/federates/a/enclaves/EnclaveKey\\(0\\)/"));
        assert!(
            aligned_action_event
                .contains("/reactors/main/reactors/a/reactors/source\\@ReactorKey\\(1\\)/actions/"),
            "action path must be nested beneath the exact source Reactor: {aligned_action_event}"
        );

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
        let chunks = rerun_chunks(&rerun, &mut memory_messages);
        let runtime_chunks = chunks
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
            scheduler_lanes.len(),
            3,
            "expected exactly three distinct federated scheduler lanes: {scheduler_lanes:?}"
        );
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
        let in_process_send = runtime_chunks
            .iter()
            .find(|chunk| {
                text_component(chunk, ":boomerang.trace.event").as_deref()
                    == Some("propagation_send")
                    && text_component(chunk, ":boomerang.trace.federate").as_deref() == Some("a")
                    && text_component(chunk, ":boomerang.trace.destination_federate").is_none()
                    && text_component(chunk, ":boomerang.trace.outcome").as_deref()
                        == Some("accepted")
            })
            .expect("accepted in-process A-to-A propagation send");
        let serialized_send = runtime_chunks
            .iter()
            .find(|chunk| {
                text_component(chunk, ":boomerang.trace.event").as_deref()
                    == Some("propagation_send")
                    && text_component(chunk, ":boomerang.trace.federate").as_deref() == Some("a")
                    && text_component(chunk, ":boomerang.trace.destination_federate").as_deref()
                        == Some("b")
                    && text_component(chunk, ":boomerang.trace.outcome").as_deref()
                        == Some("accepted")
            })
            .expect("accepted serialized A-to-B propagation send");
        assert!(text_component(serialized_send, ":boomerang.trace.destination").is_none());

        let assert_chain = |send: &&rerun::log::Chunk, expected_federate: &str| {
            let send_id = text_component(send, ":boomerang.trace.id").unwrap();
            let receive_id = links
                .iter()
                .find_map(|(source, destination)| (source == &send_id).then_some(destination))
                .unwrap_or_else(|| panic!("{expected_federate} send has no exact receive edge"));
            let receive = runtime_chunks
                .iter()
                .find(|chunk| {
                    text_component(chunk, ":boomerang.trace.id").as_deref()
                        == Some(receive_id.as_str())
                        && text_component(chunk, ":boomerang.trace.event").as_deref()
                            == Some("propagation_receive")
                })
                .expect("send destination is a propagation receive");
            assert_eq!(
                text_component(receive, ":boomerang.trace.federate").as_deref(),
                Some(expected_federate)
            );
            if let Some(destination) = text_component(send, ":boomerang.trace.destination") {
                assert_eq!(
                    Some(destination),
                    text_component(receive, ":boomerang.trace.enclave")
                );
            }
            assert_eq!(
                text_component(send, ":boomerang.trace.action_key"),
                text_component(receive, ":boomerang.trace.action_key")
            );
            let reaction_id = links
                .iter()
                .find_map(|(source, destination)| (source == receive_id).then_some(destination))
                .unwrap_or_else(|| panic!("{expected_federate} receive has no reaction edge"));
            let reaction = runtime_chunks
                .iter()
                .find(|chunk| {
                    text_component(chunk, ":boomerang.trace.id").as_deref()
                        == Some(reaction_id.as_str())
                })
                .expect("reaction record exists");
            assert_eq!(
                text_component(reaction, ":boomerang.trace.event").as_deref(),
                Some("reaction_execute")
            );
            assert_eq!(
                text_component(reaction, ":boomerang.trace.federate").as_deref(),
                Some(expected_federate)
            );
            let reaction_path = reaction.entity_path().to_string();
            assert!(
                reaction_path.starts_with(&format!("/federates/{expected_federate}/enclaves/"))
                    && reaction_path.contains("/reactors/con_reactor_tgt\\@"),
                "receive linked to unexpected reaction path {reaction_path}"
            );
            receive_id.clone()
        };
        let local_receive = assert_chain(in_process_send, "a");
        let remote_receive = assert_chain(serialized_send, "b");
        assert_ne!(local_receive, remote_receive);

        let chunks = rerun_chunks(&rerun, &mut memory_messages);
        rerun.finish().unwrap();
        let bytes = std::fs::read(&rrd_path).unwrap();
        let manifests =
            rerun::external::re_log_encoding::RawRrdManifest::from_rrd_bytes(&bytes).unwrap();
        let manifest_kinds = manifests
            .iter()
            .map(|manifest| manifest.store_id.kind())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(manifest_kinds.contains(&rerun::StoreKind::Recording));
        assert!(manifest_kinds.contains(&rerun::StoreKind::Blueprint));

        let file = std::io::BufReader::new(std::fs::File::open(&rrd_path).unwrap());
        let file_messages = rerun::external::re_log_encoding::DecoderApp::decode_lazy(file)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let file_recording = decode_chunks(&file_messages, Some(rerun::StoreKind::Recording));
        assert_recording_row_normalization_is_independent_of_chunk_batching();
        assert_eq!(
            recording_semantics(&chunks),
            recording_semantics(&file_recording),
            "memory and finalized file must preserve identical deterministic recording semantics"
        );

        let has_archetype = |chunk: &&rerun::log::Chunk, expected: &str| {
            chunk.component_descriptors().any(|descriptor| {
                descriptor
                    .archetype
                    .as_ref()
                    .is_some_and(|name| name.as_str() == expected)
            })
        };
        let duration_paths = chunks
            .iter()
            .filter(|chunk| has_archetype(chunk, "rerun.archetypes.StateChange"))
            .map(|chunk| chunk.entity_path().to_string())
            .collect::<Vec<_>>();
        assert!(
            !duration_paths.is_empty(),
            "missing duration StateChange records"
        );
        let duration_events = ["/tag_process", "/reaction_execute", "/coordination_wait"];
        assert!(
            duration_paths.iter().all(|path| {
                duration_events
                    .iter()
                    .any(|duration_event| path.ends_with(duration_event))
            }),
            "StateChange outside duration-bearing trace paths: {duration_paths:#?}"
        );
        for duration_event in duration_events {
            assert!(
                duration_paths
                    .iter()
                    .any(|path| path.ends_with(duration_event)),
                "missing StateChange for duration-bearing event {duration_event}: {duration_paths:#?}"
            );
        }
        for event in [
            "action_schedule",
            "port_write",
            "propagation_send",
            "propagation_receive",
            "shutdown",
        ] {
            assert!(
                chunks.iter().any(|chunk| {
                    text_component(chunk, ":boomerang.trace.event").as_deref() == Some(event)
                        && has_archetype(&chunk, "boomerang.TraceRecord")
                }),
                "missing TraceRecord event {event}"
            );
        }
        assert!(chunks
            .iter()
            .any(|chunk| has_archetype(&chunk, "rerun.archetypes.GraphNodes")));
        assert!(chunks
            .iter()
            .any(|chunk| has_archetype(&chunk, "rerun.archetypes.GraphEdges")));
        for chunk in &runtime_chunks {
            let timelines = chunk.timelines();
            assert!(timelines
                .values()
                .any(|timeline| timeline.name() == "elapsed"));
            assert!(timelines
                .values()
                .any(|timeline| timeline.name() == "wall_clock"));
            if let Some(logical) = timelines
                .values()
                .find(|timeline| timeline.name() == "logical")
            {
                assert!(logical.times_raw().iter().all(|value| *value != i64::MAX));
            }
        }
        assert!(runtime_chunks.iter().any(|chunk| {
            chunk.timelines().values().any(|timeline| {
                timeline.name() == "logical"
                    && timeline.times_raw().iter().any(|value| *value != i64::MAX)
            })
        }));

        let blueprint = decode_chunks(&file_messages, Some(rerun::StoreKind::Blueprint));
        let view_classes = blueprint
            .iter()
            .filter_map(|chunk| {
                use rerun::external::re_sdk_types::blueprint::archetypes::ViewBlueprint;

                let name = chunk
                    .iter_component::<rerun::components::Name>(
                        ViewBlueprint::descriptor_display_name().component,
                    )
                    .next()?
                    .first()?
                    .to_string();
                let class = chunk
                    .iter_component::<rerun::blueprint::components::ViewClass>(
                        ViewBlueprint::descriptor_class_identifier().component,
                    )
                    .next()?
                    .first()?
                    .to_string();
                Some((name, class))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        for (name, class) in [
            ("Scheduler phases", "StateTimeline"),
            ("Event records", "Dataframe"),
            ("Ownership and propagation", "Graph"),
            ("Diagnostics", "TextLog"),
            ("Operational measures", "TimeSeries"),
        ] {
            assert_eq!(
                view_classes.get(name).map(String::as_str),
                Some(class),
                "decoded blueprint view classes: {view_classes:#?}"
            );
        }
        let selected_timeline = blueprint
            .iter()
            .find(|chunk| chunk.entity_path().to_string() == "/time_panel")
            .and_then(|chunk| {
                use rerun::external::re_sdk_types::blueprint::archetypes::TimePanelBlueprint;

                chunk
                    .iter_component::<rerun::blueprint::components::TimelineName>(
                        TimePanelBlueprint::descriptor_timeline().component,
                    )
                    .next()?
                    .first()
                    .map(|timeline| timeline.0 .0.to_string())
            });
        assert_eq!(selected_timeline.as_deref(), Some("elapsed"));
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
