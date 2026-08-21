#![cfg(feature = "rerun")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use boomerang::rerun::{BlueprintConfig, FlushDriver, RerunSessionBuilder, SinkConfig};
use rerun::external::arrow::array::Array as _;
use rerun::external::re_log_encoding::Decodable as _;
use tracing_subscriber::prelude::*;

fn decode_finalized_rrd(
    path: &std::path::Path,
    store_kind: rerun::StoreKind,
) -> Vec<rerun::log::Chunk> {
    let file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    rerun::external::re_log_encoding::DecoderApp::decode_lazy(file)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .filter_map(|message| match message {
            rerun::log::LogMsg::ArrowMsg(store_id, message) if store_id.kind() == store_kind => {
                Some(rerun::log::Chunk::from_chunk_record_batch(&message.batch).unwrap())
            }
            _ => None,
        })
        .collect()
}

fn rows(chunks: &[rerun::log::Chunk]) -> impl Iterator<Item = (&rerun::log::Chunk, usize)> {
    chunks
        .iter()
        .flat_map(|chunk| (0..chunk.num_rows()).map(move |row| (chunk, row)))
}

fn row_has_archetype(chunk: &rerun::log::Chunk, row: usize, expected: &str) -> bool {
    chunk.component_descriptors().any(|descriptor| {
        descriptor
            .archetype
            .as_ref()
            .is_some_and(|archetype| archetype.as_str() == expected)
            && chunk
                .component_batch_raw(descriptor.component, row)
                .is_some_and(|batch| batch.is_ok())
    })
}

fn component_names_at(
    chunk: &rerun::log::Chunk,
    row: usize,
    expected_archetype: &str,
) -> Vec<String> {
    let mut names = chunk
        .component_descriptors()
        .filter(|descriptor| {
            descriptor
                .archetype
                .as_ref()
                .is_some_and(|archetype| archetype.as_str() == expected_archetype)
                && chunk
                    .component_batch_raw(descriptor.component, row)
                    .is_some_and(|batch| batch.is_ok())
        })
        .map(|descriptor| descriptor.component.as_str().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn uint_component_at(chunk: &rerun::log::Chunk, row: usize, suffix: &str) -> u64 {
    let descriptor = chunk
        .component_descriptors()
        .find(|descriptor| descriptor.component.as_str().ends_with(suffix))
        .unwrap_or_else(|| panic!("missing component {suffix}"));
    let values = chunk
        .component_batch_raw(descriptor.component, row)
        .unwrap_or_else(|| panic!("missing component batch {suffix}"))
        .unwrap();
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::UInt64Array>()
        .unwrap_or_else(|| panic!("component {suffix} is not u64"));
    assert_eq!(values.len(), 1);
    values.value(0)
}

fn optional_text_component_at(
    chunk: &rerun::log::Chunk,
    row: usize,
    suffix: &str,
) -> Option<String> {
    let descriptor = chunk
        .component_descriptors()
        .find(|descriptor| descriptor.component.as_str().ends_with(suffix))?;
    let values = chunk.component_batch_raw(descriptor.component, row)?.ok()?;
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()?;
    (values.len() == 1).then(|| values.value(0).to_owned())
}

fn text_component_at(chunk: &rerun::log::Chunk, row: usize, suffix: &str) -> String {
    optional_text_component_at(chunk, row, suffix)
        .unwrap_or_else(|| panic!("missing text component {suffix} at row {row}"))
}

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
        .column(0)
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()?;
    let second = values
        .column(1)
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()?;
    (values.len() == 1).then(|| (first.value(0).to_owned(), second.value(0).to_owned()))
}

#[test]
fn trace_annotations_round_trip_through_finalized_rrd() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("trace-annotations.rrd");
    let session = RerunSessionBuilder::new("trace-annotations")
        .source_id("closed-loop")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let reaction = tracing::trace_span!(
            target: "boomerang::trace",
            "reaction_execute",
            event = "reaction_execute",
            enclave = "e0",
            reactor = "r0",
            reaction_key = "ReactionKey(0)",
            reaction = "react",
            logical_ns = 11_u64,
            microstep = 2_u64,
            level = 3_u64,
            state = "begin",
        );
        let _entered = reaction.enter();
        tracing::trace!(
            target: "boomerang::trace",
            event = "action_schedule",
            enclave = "e0",
            action_key = "ActionKey(0)",
            action = "tick",
            logical_ns = 11_u64,
            microstep = 2_u64,
            destination_logical_ns = 12_u64,
            destination_microstep = 0_u64,
            value_type = "()",
            value_size = 0_u64,
            outcome = "scheduled",
        );
        drop(_entered);
        tracing::trace!(
            target: "boomerang::trace",
            event = "shutdown",
            enclave = "e0",
            logical_ns = u64::MAX,
            microstep = 0_u64,
            state = "complete",
            outcome = "success",
        );
    });
    session.finish().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let footer_size = rerun::external::re_log_encoding::StreamFooter::ENCODED_SIZE_BYTES;
    rerun::external::re_log_encoding::StreamFooter::from_rrd_bytes(
        &bytes[bytes.len() - footer_size..],
    )
    .expect("finalized RRD footer");

    let chunks = decode_finalized_rrd(&path, rerun::StoreKind::Recording);
    assert!(rows(&chunks).all(|(chunk, row)| !row_has_archetype(
        chunk,
        row,
        "boomerang.TraceRecord"
    )));

    let (reaction, reaction_row) = rows(&chunks)
        .find(|(chunk, row)| row_has_archetype(chunk, *row, "boomerang.ReactionExecution"))
        .expect("closed reaction span in finalized RRD");
    assert_eq!(
        component_names_at(reaction, reaction_row, "boomerang.ReactionExecution"),
        [
            "boomerang.trace.duration_ns",
            "boomerang.trace.enclave",
            "boomerang.trace.event",
            "boomerang.trace.id",
            "boomerang.trace.level",
            "boomerang.trace.logical_ns",
            "boomerang.trace.microstep",
            "boomerang.trace.reaction",
            "boomerang.trace.reaction_key",
            "boomerang.trace.reactor",
            "boomerang.trace.state",
        ]
        .map(|name| format!("boomerang.ReactionExecution:{name}"))
    );
    for timeline in ["elapsed", "wall_clock", "logical"] {
        assert!(reaction.timelines().values().any(|column| {
            column.name() == timeline && column.times_raw().get(reaction_row).is_some()
        }));
    }
    let reaction_id = text_component_at(reaction, reaction_row, ":boomerang.trace.id");
    assert_eq!(
        text_component_at(reaction, reaction_row, ":boomerang.trace.event"),
        "reaction_execute"
    );

    let (action, action_row) = rows(&chunks)
        .find(|(chunk, row)| row_has_archetype(chunk, *row, "boomerang.ActionScheduled"))
        .expect("child action event in finalized RRD");
    assert_eq!(
        component_names_at(action, action_row, "boomerang.ActionScheduled"),
        [
            "boomerang.trace.action",
            "boomerang.trace.action_key",
            "boomerang.trace.destination_logical_ns",
            "boomerang.trace.destination_microstep",
            "boomerang.trace.enclave",
            "boomerang.trace.event",
            "boomerang.trace.id",
            "boomerang.trace.logical_ns",
            "boomerang.trace.microstep",
            "boomerang.trace.outcome",
            "boomerang.trace.parent_id",
            "boomerang.trace.value_size",
            "boomerang.trace.value_type",
        ]
        .map(|name| format!("boomerang.ActionScheduled:{name}"))
    );
    assert_eq!(
        text_component_at(action, action_row, ":boomerang.trace.parent_id"),
        reaction_id
    );
    for timeline in ["elapsed", "wall_clock", "logical"] {
        assert!(action.timelines().values().any(|column| {
            column.name() == timeline && column.times_raw().get(action_row).is_some()
        }));
    }

    let mut state_changes = rows(&chunks)
        .filter_map(|(chunk, row)| {
            (chunk.entity_path() == reaction.entity_path())
                .then(|| {
                    let elapsed = chunk
                        .timelines()
                        .values()
                        .find(|timeline| timeline.name() == "elapsed")?
                        .times_raw()
                        .get(row)
                        .copied()?;
                    Some((elapsed, state_change_at(chunk, row)?))
                })
                .flatten()
        })
        .collect::<Vec<_>>();
    state_changes.sort_by_key(|(elapsed, _)| *elapsed);
    assert_eq!(
        state_changes
            .into_iter()
            .map(|(_, state)| state)
            .collect::<Vec<_>>(),
        vec![Some("executing reaction".to_owned()), None],
        "reaction span must set then reset its state lane"
    );

    let (unrepresentable, unrepresentable_row) = rows(&chunks)
        .find(|(chunk, row)| row_has_archetype(chunk, *row, "boomerang.Shutdown"))
        .expect("u64::MAX shutdown in finalized RRD");
    assert_eq!(
        uint_component_at(
            unrepresentable,
            unrepresentable_row,
            ":boomerang.trace.logical_ns"
        ),
        u64::MAX
    );
    assert!(unrepresentable
        .timelines()
        .values()
        .all(|timeline| timeline.name() != "logical"
            || timeline.times_raw().get(unrepresentable_row).is_none()));
}

#[test]
fn causal_edges_require_a_unique_complete_route_and_tag() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("causality.rrd");
    let session = RerunSessionBuilder::new("causality")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..2 {
            let send = tracing::trace_span!(
                target: "boomerang::trace",
                "propagation_send",
                event = "propagation_send",
                kind = "logical",
                federate = "a",
                enclave = "e0",
                destination_federate = "b",
                action_key = "ActionKey(0)",
                action = "inbound",
                logical_ns = 7_u64,
                microstep = 0_u64,
                value_type = "u32",
                value_size = 4_u64,
                outcome = "accepted",
            );
            let _entered = send.enter();
        }
        tracing::trace!(
            target: "boomerang::trace",
            event = "async_ingress",
            kind = "logical",
            federate = "b",
            enclave = "con_reactor_tgt",
            action_key = "ActionKey(0)",
            action = "inbound",
            logical_ns = 7_u64,
            microstep = 0_u64,
            destination_logical_ns = 7_u64,
            destination_microstep = 0_u64,
            value_type = "u32",
            value_size = 4_u64,
            outcome = "accepted",
        );

        let send = tracing::trace_span!(
            target: "boomerang::trace",
            "propagation_send",
            event = "propagation_send",
            kind = "logical",
            federate = "a",
            enclave = "e0",
            destination_federate = "b",
            action_key = "ActionKey(0)",
            action = "inbound",
            logical_ns = 8_u64,
            microstep = 0_u64,
            value_type = "u32",
            value_size = 4_u64,
            outcome = "accepted",
        );
        let _entered = send.enter();
        drop(_entered);
        drop(send);
        tracing::trace!(
            target: "boomerang::trace",
            event = "async_ingress",
            kind = "logical",
            federate = "b",
            enclave = "con_reactor_tgt",
            action_key = "ActionKey(0)",
            action = "inbound",
            logical_ns = 8_u64,
            microstep = 0_u64,
            destination_logical_ns = 8_u64,
            destination_microstep = 0_u64,
            value_type = "u32",
            value_size = 4_u64,
            outcome = "accepted",
        );
        let reaction = tracing::trace_span!(
            target: "boomerang::trace",
            "reaction_execute",
            event = "reaction_execute",
            federate = "b",
            enclave = "con_reactor_tgt",
            reactor = "target",
            reaction_key = "con_reactor_tgt",
            reaction = "deliver",
            logical_ns = 8_u64,
            microstep = 0_u64,
            level = 0_u64,
            state = "begin",
        );
        let _entered = reaction.enter();
        drop(_entered);
        drop(reaction);

        for _ in 0..17 {
            let send = tracing::trace_span!(
                target: "boomerang::trace",
                "propagation_send",
                event = "propagation_send",
                kind = "logical",
                federate = "a",
                enclave = "e0",
                destination_federate = "b",
                action_key = "ActionKey(0)",
                action = "inbound",
                logical_ns = 9_u64,
                microstep = 0_u64,
                value_type = "u32",
                value_size = 4_u64,
                outcome = "accepted",
            );
            let _entered = send.enter();
        }
        tracing::trace!(
            target: "boomerang::trace",
            event = "async_ingress",
            kind = "logical",
            federate = "b",
            enclave = "con_reactor_tgt",
            action_key = "ActionKey(0)",
            action = "inbound",
            logical_ns = 9_u64,
            microstep = 0_u64,
            destination_logical_ns = 9_u64,
            destination_microstep = 0_u64,
            value_type = "u32",
            value_size = 4_u64,
            outcome = "accepted",
        );

        for outcome in ["accepted", "failed"] {
            let send = tracing::trace_span!(
                target: "boomerang::trace",
                "propagation_send",
                event = "propagation_send",
                kind = "logical",
                federate = "a",
                enclave = "e0",
                destination_federate = "b",
                action_key = "ActionKey(0)",
                action = "inbound",
                logical_ns = 10_u64,
                microstep = 0_u64,
                value_type = "u32",
                value_size = 4_u64,
                outcome,
            );
            let _entered = send.enter();
        }
        tracing::trace!(
            target: "boomerang::trace",
            event = "async_ingress",
            kind = "logical",
            federate = "b",
            enclave = "con_reactor_tgt",
            action_key = "ActionKey(0)",
            action = "inbound",
            logical_ns = 10_u64,
            microstep = 0_u64,
            destination_logical_ns = 10_u64,
            destination_microstep = 0_u64,
            value_type = "u32",
            value_size = 4_u64,
            outcome = "accepted",
        );
    });
    session.finish().unwrap();

    let chunks = decode_finalized_rrd(&path, rerun::StoreKind::Recording);
    let send_ids = |logical_ns| {
        rows(&chunks)
            .filter(|(chunk, row)| {
                row_has_archetype(chunk, *row, "boomerang.PropagationSerializedSend")
                    && uint_component_at(chunk, *row, ":boomerang.trace.logical_ns") == logical_ns
            })
            .map(|(chunk, row)| text_component_at(chunk, row, ":boomerang.trace.id"))
            .collect::<Vec<_>>()
    };
    let ambiguous = send_ids(7);
    assert_eq!(ambiguous.len(), 2);
    let ambiguous_ingress = rows(&chunks)
        .find(|(chunk, row)| {
            row_has_archetype(chunk, *row, "boomerang.LogicalIngress")
                && uint_component_at(chunk, *row, ":boomerang.trace.logical_ns") == 7
        })
        .map(|(chunk, row)| text_component_at(chunk, row, ":boomerang.trace.id"))
        .expect("ambiguous-tag ingress remains neutral");

    let links = rows(&chunks)
        .filter(|(chunk, row)| row_has_archetype(chunk, *row, "boomerang.CausalLink"))
        .map(|(chunk, row)| {
            let source = text_component_at(chunk, row, ":boomerang.trace.source");
            let destination = text_component_at(chunk, row, ":boomerang.trace.destination");
            assert_eq!(
                graph_edge_at(chunk, row),
                Some((source.clone(), destination.clone()))
            );
            (source, destination)
        })
        .collect::<Vec<_>>();
    assert!(links.iter().all(|(source, _)| !ambiguous.contains(source)));
    let ambiguous_ids = ambiguous
        .iter()
        .chain(std::iter::once(&ambiguous_ingress))
        .collect::<Vec<_>>();
    assert!(rows(&chunks)
        .filter_map(|(chunk, row)| graph_edge_at(chunk, row))
        .all(|(source, destination)| {
            ambiguous_ids
                .iter()
                .all(|id| id.as_str() != source.as_str() && id.as_str() != destination.as_str())
        }));
    assert!(!rows(&chunks).any(|(chunk, row)| {
        row_has_archetype(chunk, row, "boomerang.PropagationReceive")
            && uint_component_at(chunk, row, ":boomerang.trace.logical_ns") == 7
    }));

    let unique_sends = send_ids(8);
    assert_eq!(unique_sends.len(), 1, "one unique complete-tag send");
    let unique_send = &unique_sends[0];
    let receive_edges = links
        .iter()
        .filter_map(|(source, destination)| (source == unique_send).then_some(destination))
        .collect::<Vec<_>>();
    assert_eq!(receive_edges.len(), 1, "one send-to-receive edge");
    let receive = receive_edges[0];
    let reaction_edges = links
        .iter()
        .filter_map(|(source, destination)| (source == receive).then_some(destination))
        .collect::<Vec<_>>();
    assert!(
        reaction_edges.is_empty(),
        "unregistered synthetic topology must not infer receive-to-reaction causality"
    );

    for ambiguous_tag in [9, 10] {
        assert!(!rows(&chunks).any(|(chunk, row)| {
            row_has_archetype(chunk, row, "boomerang.PropagationReceive")
                && uint_component_at(chunk, row, ":boomerang.trace.logical_ns") == ambiguous_tag
        }));
        let ambiguous_ids = rows(&chunks)
            .filter(|(chunk, row)| {
                (row_has_archetype(chunk, *row, "boomerang.PropagationSerializedSend")
                    || row_has_archetype(chunk, *row, "boomerang.LogicalIngress"))
                    && uint_component_at(chunk, *row, ":boomerang.trace.logical_ns")
                        == ambiguous_tag
            })
            .map(|(chunk, row)| text_component_at(chunk, row, ":boomerang.trace.id"))
            .collect::<Vec<_>>();
        assert_eq!(
            ambiguous_ids.len(),
            if ambiguous_tag == 9 { 18 } else { 3 },
            "all sends and the ingress for tag {ambiguous_tag} must exist in this recording"
        );
        assert!(rows(&chunks)
            .filter_map(|(chunk, row)| graph_edge_at(chunk, row))
            .all(|(source, destination)| ambiguous_ids
                .iter()
                .all(|id| id != &source && id != &destination)));
    }
}

#[test]
fn local_and_serialized_aliases_promote_one_receive_without_false_edges() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("alias-causality.rrd");
    let session = RerunSessionBuilder::new("alias-causality")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let local = tracing::trace_span!(
            target: "boomerang::trace",
            "propagation_send",
            event = "propagation_send",
            kind = "logical",
            federate = "b",
            enclave = "source-local",
            destination = "e1",
            action_key = "ActionKey(0)",
            action = "inbound",
            logical_ns = 12_u64,
            microstep = 0_u64,
            value_type = "u32",
            value_size = 4_u64,
            outcome = "accepted",
        );
        let _entered = local.enter();
        drop(_entered);
        drop(local);

        let serialized = tracing::trace_span!(
            target: "boomerang::trace",
            "propagation_send",
            event = "propagation_send",
            kind = "logical",
            federate = "a",
            enclave = "source-serialized",
            destination_federate = "b",
            action_key = "ActionKey(0)",
            action = "inbound",
            logical_ns = 12_u64,
            microstep = 0_u64,
            value_type = "u32",
            value_size = 4_u64,
            outcome = "accepted",
        );
        let _entered = serialized.enter();
        drop(_entered);
        drop(serialized);

        tracing::trace!(
            target: "boomerang::trace",
            event = "async_ingress",
            kind = "logical",
            federate = "b",
            enclave = "e1",
            action_key = "ActionKey(0)",
            action = "inbound",
            logical_ns = 12_u64,
            microstep = 0_u64,
            destination_logical_ns = 12_u64,
            destination_microstep = 0_u64,
            value_type = "u32",
            value_size = 4_u64,
            outcome = "accepted",
        );
    });
    session.finish().unwrap();

    let chunks = decode_finalized_rrd(&path, rerun::StoreKind::Recording);
    let send_ids = rows(&chunks)
        .filter(|(chunk, row)| {
            row_has_archetype(chunk, *row, "boomerang.PropagationLogicalSend")
                || row_has_archetype(chunk, *row, "boomerang.PropagationSerializedSend")
        })
        .map(|(chunk, row)| text_component_at(chunk, row, ":boomerang.trace.id"))
        .collect::<Vec<_>>();
    assert_eq!(send_ids.len(), 2, "both send aliases are recorded");
    let ingress_id = rows(&chunks)
        .find(|(chunk, row)| row_has_archetype(chunk, *row, "boomerang.LogicalIngress"))
        .map(|(chunk, row)| text_component_at(chunk, row, ":boomerang.trace.id"))
        .expect("one ingress record");
    let receives = rows(&chunks)
        .filter(|(chunk, row)| row_has_archetype(chunk, *row, "boomerang.PropagationReceive"))
        .map(|(chunk, row)| text_component_at(chunk, row, ":boomerang.trace.id"))
        .collect::<Vec<_>>();
    assert_eq!(
        receives,
        std::slice::from_ref(&ingress_id),
        "one promoted receive"
    );

    let receive_edges = rows(&chunks)
        .filter_map(|(chunk, row)| graph_edge_at(chunk, row))
        .filter(|(_, destination)| destination == &ingress_id)
        .collect::<Vec<_>>();
    assert_eq!(receive_edges.len(), 1, "one exact send-to-receive edge");
    assert!(send_ids.contains(&receive_edges[0].0));
    assert_eq!(
        rows(&chunks)
            .filter_map(|(chunk, row)| graph_edge_at(chunk, row))
            .filter(|(source, destination)| {
                send_ids.contains(source) || source == &ingress_id || destination == &ingress_id
            })
            .count(),
        1,
        "no duplicate or false edge may mention the aliased propagation"
    );
}

#[test]
fn span_duration_counts_only_entered_time() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("entered-duration.rrd");
    let session = RerunSessionBuilder::new("entered-duration")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    let unentered_elapsed = tracing::subscriber::with_default(subscriber, || {
        let span = tracing::trace_span!(
            target: "boomerang::trace",
            "tag_process",
            event = "tag_process",
            enclave = "e0",
            logical_ns = 13_u64,
            microstep = 0_u64,
            terminal = false,
            state = "processing",
        );
        let unentered = Instant::now();
        std::thread::sleep(Duration::from_millis(200));
        let unentered_elapsed = unentered.elapsed();
        for _ in 0..2 {
            let _entered = span.enter();
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(span);
        assert!(unentered_elapsed >= Duration::from_millis(200));
        unentered_elapsed
    });
    session.finish().unwrap();

    let chunks = decode_finalized_rrd(&path, rerun::StoreKind::Recording);
    let (span, row) = rows(&chunks)
        .find(|(chunk, row)| row_has_archetype(chunk, *row, "boomerang.TagProcessing"))
        .expect("closed duration span");
    let duration =
        Duration::from_nanos(uint_component_at(span, row, ":boomerang.trace.duration_ns"));
    assert!(duration >= Duration::from_millis(5));
    assert!(
        duration < unentered_elapsed,
        "duration {duration:?} includes substantial unentered time"
    );
}

#[test]
fn explicit_root_records_do_not_inherit_active_adapter_context() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("explicit-root.rrd");
    let session = RerunSessionBuilder::new("explicit-root")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let outer = tracing::trace_span!(
            target: "boomerang::trace",
            "reaction_execute",
            event = "reaction_execute",
            federate = "outer-federate",
            enclave = "outer-enclave",
            reactor = "outer",
            reaction = "outer",
            logical_ns = 20_u64,
            microstep = 1_u64,
            level = 0_u64,
            state = "begin",
        );
        let _outer = outer.enter();
        let root = tracing::trace_span!(
            target: "boomerang::trace",
            parent: None,
            "tag_process",
            event = "tag_process",
            federate = "root-federate",
            enclave = "root-enclave",
            logical_ns = 30_u64,
            microstep = 2_u64,
            terminal = false,
            state = "processing",
        );
        let _root = root.enter();
        drop(_root);
        drop(root);
        tracing::trace!(
            target: "boomerang::trace",
            parent: None,
            event = "action_schedule",
            federate = "event-federate",
            enclave = "event-enclave",
            action_key = "ActionKey(0)",
            action = "tick",
            logical_ns = 31_u64,
            microstep = 3_u64,
            destination_logical_ns = 32_u64,
            destination_microstep = 0_u64,
            value_type = "()",
            value_size = 0_u64,
            outcome = "scheduled",
        );
    });
    session.finish().unwrap();

    let chunks = decode_finalized_rrd(&path, rerun::StoreKind::Recording);
    for (archetype, logical, microstep, federate, enclave) in [
        (
            "boomerang.TagProcessing",
            30,
            2,
            "root-federate",
            "root-enclave",
        ),
        (
            "boomerang.ActionScheduled",
            31,
            3,
            "event-federate",
            "event-enclave",
        ),
    ] {
        let (chunk, row) = rows(&chunks)
            .find(|(chunk, row)| {
                row_has_archetype(chunk, *row, archetype)
                    && uint_component_at(chunk, *row, ":boomerang.trace.logical_ns") == logical
            })
            .unwrap_or_else(|| panic!("explicit-root {archetype}"));
        assert!(optional_text_component_at(chunk, row, ":boomerang.trace.parent_id").is_none());
        assert_eq!(
            uint_component_at(chunk, row, ":boomerang.trace.microstep"),
            microstep
        );
        assert_eq!(
            optional_text_component_at(chunk, row, ":boomerang.trace.federate").as_deref(),
            Some(federate)
        );
        assert_eq!(
            optional_text_component_at(chunk, row, ":boomerang.trace.enclave").as_deref(),
            Some(enclave)
        );
    }
}

struct PanickingDebug;

impl std::fmt::Debug for PanickingDebug {
    fn fmt(&self, _formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("injected Debug panic")
    }
}

#[test]
fn callback_debug_panic_does_not_unwind_application_or_prevent_finalization() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("panic-isolation.rrd");
    let session = RerunSessionBuilder::new("panic-isolation")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    let application = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(
                target: "boomerang::trace",
                event = "shutdown",
                enclave = ?PanickingDebug,
                logical_ns = 40_u64,
                microstep = 0_u64,
                state = "complete",
                outcome = "success",
            );
        });
    }));
    let errors = session.error_count();
    let finalization = session.finish();
    let chunks = decode_finalized_rrd(&path, rerun::StoreKind::Recording);
    let diagnosed = rows(&chunks)
        .any(|(chunk, row)| row_has_archetype(chunk, row, "boomerang.SchemaDiagnostic"));

    assert!(
        application.is_ok(),
        "trace callback unwound into application"
    );
    assert!(
        errors > 0 || diagnosed,
        "isolated callback failure must remain observable"
    );
    let error = finalization.expect_err("finish must report the prior observational failure");
    assert!(error
        .to_string()
        .contains("recording was disabled after an observational failure"));
}

#[test]
fn malformed_traces_round_trip_as_diagnostics_without_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("malformed.rrd");
    let session = RerunSessionBuilder::new("malformed-trace")
        .source_id("closed-loop")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        macro_rules! malformed_span {
            ($($field:tt)*) => {{
                let span = tracing::trace_span!(target: "boomerang::trace", "malformed", $($field)*);
                let _entered = span.enter();
            }};
        }

        tracing::trace!(
            target: "boomerang::trace",
            event = "shutdown",
            outcome = "success"
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "action_schedule",
            enclave = "e0",
            outcome = "scheduled"
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "frontier_publish",
            enclave = "e0",
            logical_ns = 1_u64,
            microstep = 0_u64,
            state = "invalid",
            outcome = "published"
        );
        tracing::trace!(target: "boomerang::trace", event = "coordination_grant", enclave = "e0");
        tracing::trace!(target: "boomerang::trace", event = "tag_release", enclave = "e0");
        tracing::trace!(target: "boomerang::trace", event = "tag_complete", enclave = "e0");

        malformed_span!(
            event = "tag_process",
            enclave = "e0",
            logical_ns = 2_u64,
            microstep = 0_u64,
            state = "processing"
        );
        malformed_span!(
            event = "reaction_execute",
            enclave = "e0",
            reactor = "root",
            reaction = "react",
            logical_ns = 2_u64,
            microstep = 0_u64,
            level = "invalid",
            state = "begin"
        );
        malformed_span!(
            event = "coordination_wait",
            enclave = "e0",
            logical_ns = 2_u64,
            microstep = 0_u64
        );
        malformed_span!(
            event = "propagation_send",
            kind = "logical",
            enclave = "e0",
            action_key = "ActionKey(0)",
            action = "tick",
            logical_ns = 2_u64,
            microstep = 0_u64,
            value_type = "()",
            value_size = 0_u64,
            outcome = "accepted"
        );
        malformed_span!(
            event = "propagation_send",
            kind = "logical",
            destination_federate = "b",
            action_key = "ActionKey(0)",
            action = "tick",
            logical_ns = 2_u64,
            microstep = 0_u64,
            value_type = "()",
            value_size = 0_u64,
            outcome = "accepted"
        );
        for outcome in ["scheduled", "startup"] {
            tracing::trace!(
                target: "boomerang::trace",
                event = "action_schedule",
                enclave = "e0",
                action_key = "ActionKey(0)",
                action = "tick",
                destination_logical_ns = 3_u64,
                destination_microstep = 0_u64,
                value_type = "()",
                value_size = 0_u64,
                outcome,
            );
        }
        tracing::trace!(
            target: "boomerang::trace",
            event = "port_write",
            enclave = "e0",
            port_key = "PortKey(0)",
            port = "out",
            value_type = "u32",
            outcome = "written",
        );
        malformed_span!(
            event = "tag_process",
            enclave = "e0",
            logical_ns = 4_u64,
            microstep = 0_u64,
            terminal = false,
            state = "invalid"
        );
        malformed_span!(
            event = "coordination_wait",
            enclave = "e0",
            logical_ns = 4_u64,
            microstep = 0_u64,
            state = "invalid"
        );
        malformed_span!(
            event = "propagation_send",
            kind = "logical",
            enclave = "e0",
            destination = "e1",
            action_key = "ActionKey(0)",
            action = "tick",
            logical_ns = 4_u64,
            microstep = 0_u64,
            value_type = "()",
            value_size = 0_u64,
            outcome = "invalid"
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "async_ingress",
            kind = "shutdown",
            enclave = "e0",
            outcome = "accepted",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "async_ingress",
            kind = "shutdown",
            enclave = "e0",
            logical_ns = 5_u64,
            microstep = 0_u64,
            outcome = "invalid",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "port_write",
            enclave = "e0",
            logical_ns = 5_u64,
            microstep = 0_u64,
            port_key = "PortKey(0)",
            port = "out",
            value_type = "u32",
            outcome = "invalid",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "frontier_publish",
            enclave = "e0",
            logical_ns = 5_u64,
            microstep = 0_u64,
            state = "candidate",
            outcome = "invalid",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "coordination_grant",
            enclave = "e0",
            logical_ns = 5_u64,
            microstep = 0_u64,
            outcome = "invalid",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "tag_release",
            enclave = "e0",
            destination = "e1",
            logical_ns = 5_u64,
            microstep = 0_u64,
            outcome = "invalid",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "tag_complete",
            enclave = "e0",
            logical_ns = 5_u64,
            microstep = 0_u64,
            terminal = false,
            outcome = "invalid",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "shutdown",
            enclave = "e0",
            logical_ns = 5_u64,
            microstep = 0_u64,
            state = "complete",
            outcome = "invalid",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "shutdown",
            enclave = "e0",
            logical_ns = 5_u64,
            microstep = 0_u64,
            state = "invalid",
            outcome = "success",
        );
        malformed_span!(
            event = "propagation_send",
            kind = "logical",
            enclave = "e0",
            destination = "e1",
            action_key = "ActionKey(0)",
            action = "tick",
            logical_ns = 5_u64,
            microstep = 0_u64,
            value_type = "()",
            value_size = 0_u64,
            outcome = "rejected"
        );
    });
    session.finish().unwrap();

    let chunks = decode_finalized_rrd(&path, rerun::StoreKind::Recording);
    let diagnostic_rows = chunks
        .iter()
        .flat_map(|chunk| {
            (0..chunk.num_rows()).filter_map(move |row| {
                (chunk.entity_path().to_string() == "/diagnostics/schema"
                    && (row_has_archetype(chunk, row, "boomerang.SchemaDiagnostic")
                        || row_has_archetype(chunk, row, "rerun.archetypes.TextLog")))
                .then_some((chunk, row))
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        diagnostic_rows.len(),
        27,
        "one diagnostic row per malformed record"
    );
    assert!(diagnostic_rows.iter().all(|(diagnostic, row)| {
        row_has_archetype(diagnostic, *row, "boomerang.SchemaDiagnostic")
            && row_has_archetype(diagnostic, *row, "rerun.archetypes.TextLog")
    }));
    for chunk in &chunks {
        for row in 0..chunk.num_rows() {
            assert!(!chunk.component_descriptors().any(|descriptor| {
                descriptor.archetype.as_ref().is_some_and(|archetype| {
                    let archetype = archetype.as_str();
                    archetype != "boomerang.SchemaDiagnostic"
                        && (archetype.starts_with("boomerang.")
                            || matches!(
                                archetype,
                                "rerun.archetypes.StateChange"
                                    | "rerun.archetypes.GraphEdges"
                                    | "rerun.archetypes.Scalars"
                            ))
                        && chunk
                            .component_batch_raw(descriptor.component, row)
                            .is_some_and(|batch| batch.is_ok())
                })
            }));
        }
    }
}

struct FailingFinalization;

impl FlushDriver for FailingFinalization {
    fn flush(
        &self,
        _recording: &rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        Err(rerun::sink::SinkFlushError::failed(
            "injected finalization failure",
        ))
    }
}

struct CorruptingTeardown {
    path: std::path::PathBuf,
}

impl FlushDriver for CorruptingTeardown {
    fn flush(
        &self,
        _recording: &rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        Ok(())
    }

    fn teardown(
        &self,
        recording: rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        drop(recording);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&self.path)
            .unwrap()
            .set_len(0)
            .unwrap();
        Ok(())
    }
}

#[test]
fn finish_reports_failed_or_corrupt_file_finalization() {
    let failed = RerunSessionBuilder::new("failing-finalization")
        .blueprint(BlueprintConfig::None)
        .flush_driver(Arc::new(FailingFinalization))
        .build()
        .unwrap();
    let error = failed.finish().unwrap_err();
    assert!(error.to_string().contains("injected finalization failure"));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("corrupt.rrd");
    let corrupt = RerunSessionBuilder::new("corrupt-finalization")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .flush_driver(Arc::new(CorruptingTeardown { path: path.clone() }))
        .build()
        .unwrap();
    let error = corrupt.finish().unwrap_err();
    assert!(error
        .to_string()
        .contains("missing or truncated RRD footer"));
}

#[test]
fn blueprint_none_and_custom_control_blueprint_emission() {
    let directory = tempfile::tempdir().unwrap();
    let none_path = directory.path().join("none.rrd");
    let none = RerunSessionBuilder::new("boomerang-rerun-test")
        .blueprint(BlueprintConfig::None)
        .sink(SinkConfig::File(none_path.clone()))
        .build()
        .unwrap();
    none.finish().unwrap();
    assert!(decode_finalized_rrd(&none_path, rerun::StoreKind::Blueprint).is_empty());

    let custom = rerun::blueprint::Blueprint::new(
        rerun::blueprint::TextLogView::new("Caller-defined diagnostics")
            .with_origin("/diagnostics"),
    );
    let custom_path = directory.path().join("custom.rrd");
    let custom = RerunSessionBuilder::new("boomerang-rerun-test")
        .blueprint(BlueprintConfig::Custom(Box::new(custom)))
        .sink(SinkConfig::File(custom_path.clone()))
        .build()
        .unwrap();
    custom.finish().unwrap();
    let custom = decode_finalized_rrd(&custom_path, rerun::StoreKind::Blueprint);
    assert!(rows(&custom).any(|(chunk, row)| {
        chunk.entity_path().to_string().starts_with("/view/")
            && optional_text_component_at(chunk, row, "ViewBlueprint:display_name").as_deref()
                == Some("Caller-defined diagnostics")
    }));
}
