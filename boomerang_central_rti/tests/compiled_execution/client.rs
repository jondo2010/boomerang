//! Ordered scripted replies exercise the production client and real compiled scheduler.
use super::*;
use boomerang_central_rti::compiled::{CentralRtiError, RtiReplySource, RtiRequestSink};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
    time::Duration as StdDuration,
};
#[derive(Clone, Default)]
struct TraceOutput(Arc<Mutex<Vec<u8>>>);

struct TraceWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for TraceWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TraceOutput {
    type Writer = TraceWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TraceWriter(self.0.clone())
    }
}

/// Capture the formatter's bytes, including events from runtime-owned worker threads.
pub(super) fn capture_coordination<T>(run: impl FnOnce() -> T) -> (T, Vec<serde_json::Value>) {
    let output = TraceOutput::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .without_time()
        .with_writer(output.clone())
        .with_env_filter("boomerang::coordination=debug")
        .finish();
    let result = tracing::subscriber::with_default(subscriber, run);
    let bytes = output.0.lock().unwrap();
    let events = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (result, events)
}

#[derive(Default)]
struct Script {
    replies: VecDeque<RtiReply>,
    requests: Vec<RtiRequest>,
    decoded: Vec<u32>,
    observed_grants: usize,
    single_horizon: bool,
    horizon_sent: bool,
    observed_horizons: usize,
}

struct RequestRecorder(Arc<Mutex<Script>>);
impl RtiRequestSink for RequestRecorder {
    fn send(&self, request: RtiRequest) -> Result<(), CentralRtiError> {
        let mut script = self.0.lock().unwrap();
        match request {
            RtiRequest::Publish { revision, next_event: Some(tag) } => {
                if !script.single_horizon || tag < WireTag::finite(1_000_000, 0) || !script.horizon_sent {
                    let tag = if script.single_horizon && tag >= WireTag::finite(1_000_000, 0) { WireTag::finite(2_000_000, 0) } else { tag };
                    script.replies.push_back(RtiReply::Grant { revision, tag });
                    script.horizon_sent |= tag == WireTag::finite(2_000_000, 0);
                }
            }
            RtiRequest::ConfirmIdle { revision } if script.requests.iter().any(|request| {
                matches!(request, RtiRequest::Complete { tag } if *tag == WireTag::finite(2_000_000, 0))
            }) => script.replies.push_back(RtiReply::Idle { revision }),
            RtiRequest::Stop => script.replies.push_back(RtiReply::Stopped),
            _ => {}
        }
        script.requests.push(request);
        Ok(())
    }
}

struct OrderedReplies(Arc<Mutex<Script>>);
impl RtiReplySource for OrderedReplies {
    fn receive(&mut self, _timeout: StdDuration) -> Result<Option<RtiReply>, CentralRtiError> {
        let mut script = self.0.lock().unwrap();
        let reply = script.replies.pop_front();
        if matches!(reply, Some(RtiReply::Grant { .. })) {
            assert_eq!(
                script.decoded,
                [41, 42],
                "all preceding payloads must be admitted before exposing a grant"
            );
            if script.observed_grants == 0 {
                assert!(
                    !script
                        .requests
                        .iter()
                        .any(|r| matches!(r, RtiRequest::Complete { .. })),
                    "admitting future input must not emit LTC before processing"
                );
            }
            script.observed_grants += 1;
            if matches!(reply, Some(RtiReply::Grant { tag, .. }) if tag == WireTag::finite(2_000_000, 0))
            {
                script.observed_horizons += 1;
            }
        }
        Ok(reply)
    }
}

fn payload(value: u32, nanos: i128) -> RtiReply {
    RtiReply::Payload {
        route: RtiRouteIndex::new(0),
        tag: WireTag::finite(nanos, 0),
        payload: value.to_le_bytes().to_vec(),
    }
}

fn execute_script(
    script: Arc<Mutex<Script>>,
) -> Result<boomerang_runtime::FederateExecution, boomerang_runtime::ExecuteOwnedFederateError> {
    let view = CompiledDeploymentView::new(DEPLOYMENT).unwrap();
    let bindings = RtiClientBindings::new(&view, MEMBERS[1], IDENTITY).unwrap();
    let decoder = script.clone();
    execute_owned_federate_with_backend(
        MEMBERS[1],
        &FEDERATES[1],
        &ENCLAVES[1..],
        FederateBindings::new()
            .bind_enclave(EnclaveIndex::new(1), sink_bindings())
            .bind_enclave(EnclaveIndex::new(2), sink_bindings())
            .bind_inbound_route(
                BoundaryId::new("pipe"),
                PayloadType::<u32>::new(),
                move |bytes: &[u8]| {
                    let bytes = bytes
                        .try_into()
                        .map_err(|_| std::io::Error::other("scripted malformed payload"))?;
                    let value = u32::from_le_bytes(bytes);
                    decoder.lock().unwrap().decoded.push(value);
                    Ok::<_, std::io::Error>(value)
                },
            ),
        Config::default().with_fast_forward(true),
        |inbound| {
            CentralRtiClient::connect(
                Arc::new(RequestRecorder(script.clone())),
                OrderedReplies(script),
                bindings,
                inbound,
                StdDuration::from_secs(2),
            )
        },
    )
}

#[test]
fn multiple_payloads_are_admitted_in_order_before_grant_and_execute_at_their_tags() {
    bounded(|| {
        let script = Arc::new(Mutex::new(Script {
            replies: [
                RtiReply::Started,
                payload(41, 1_000_000),
                payload(42, 2_000_000),
            ]
            .into(),
            ..Script::default()
        }));
        let (result, events) = capture_coordination(|| execute_script(script.clone()).unwrap());
        let sink = result.enclave(EnclaveIndex::new(1)).unwrap();
        assert_eq!(
            sink.state::<RoutedSinkState>(StateSlotIndex::new(0))
                .unwrap()
                .values,
            [41, 42]
        );
        assert_eq!(sink.final_tag(), Tag::new(Duration::milliseconds(2), 0));
        let script = script.lock().unwrap();
        assert_eq!(script.decoded, [41, 42]);
        assert!(script.observed_grants > 0);
        assert!(matches!(script.requests.last(), Some(RtiRequest::Stop)));
        let payload_events: Vec<_> = events
            .iter()
            .filter(|event| {
                matches!(
                    event["fields"]["event"].as_str(),
                    Some("coordination.payload.received" | "coordination.boundary.admitted")
                )
            })
            .collect();
        assert_eq!(payload_events.len(), 4, "{events:#?}");
        for pair in payload_events.as_chunks::<2>().0 {
            assert_eq!(pair[0]["fields"]["event"], "coordination.payload.received");
            assert_eq!(pair[1]["fields"]["event"], "coordination.boundary.admitted");
            assert_eq!(pair[0]["fields"]["tag"], pair[1]["fields"]["tag"]);
            for event in pair {
                assert_eq!(event["fields"]["federate"], "FederateIndex(1)");
                assert_eq!(event["fields"]["route"], "RtiRouteIndex(0)");
                assert_eq!(event["fields"]["coordination"], format!("{IDENTITY:?}"));
                assert!(event["fields"].get("payload").is_none());
            }
        }
        let first_grant = events
            .iter()
            .position(|event| event["fields"]["event"] == "coordination.grant.received")
            .unwrap();
        let last_admission = events
            .iter()
            .rposition(|event| event["fields"]["event"] == "coordination.boundary.admitted")
            .unwrap();
        assert!(last_admission < first_grant);
    });
}

#[test]
fn one_horizon_executes_multiple_events_without_another_rti_grant() {
    bounded(|| {
        let script = Arc::new(Mutex::new(Script {
            replies: [
                RtiReply::Started,
                payload(41, 1_000_000),
                payload(42, 2_000_000),
            ]
            .into(),
            single_horizon: true,
            ..Script::default()
        }));
        let result = execute_script(script.clone()).unwrap();
        let sink = result.enclave(EnclaveIndex::new(1)).unwrap();
        assert_eq!(
            sink.state::<RoutedSinkState>(StateSlotIndex::new(0))
                .unwrap()
                .values,
            [41, 42]
        );
        assert_eq!(sink.final_tag(), Tag::new(Duration::milliseconds(2), 0));
        let script = script.lock().unwrap();
        assert_eq!(script.observed_horizons, 1);
        // Reusing authority must preserve eager NET and cumulative completion reports.
        for nanos in [1_000_000, 2_000_000] {
            assert!(script.requests.iter().any(|request| matches!(request,
                RtiRequest::Publish { next_event: Some(tag), .. } if *tag == WireTag::finite(nanos, 0))));
            assert!(script.requests.iter().any(|request| matches!(request,
                RtiRequest::Complete { tag } if *tag >= WireTag::finite(nanos, 0))));
        }
        assert!(matches!(script.requests.last(), Some(RtiRequest::Stop)));
    });
}

#[test]
fn decode_failure_terminates_execution_before_queued_grant() {
    let (_, events) = capture_coordination(|| {
        bounded(|| {
            let script = Arc::new(Mutex::new(Script {
                replies: [
                    RtiReply::Started,
                    RtiReply::Payload {
                        route: RtiRouteIndex::new(0),
                        tag: WireTag::finite(1_000_000, 0),
                        payload: vec![0xff],
                    },
                    RtiReply::Grant {
                        revision: 1,
                        tag: WireTag::finite(1_000_000, 0),
                    },
                ]
                .into(),
                ..Script::default()
            }));
            let error = execute_script(script.clone()).unwrap_err().to_string();
            assert!(error.contains("scripted malformed payload"), "{error}");
            let script = script.lock().unwrap();
            assert!(script.decoded.is_empty());
            assert_eq!(script.observed_grants, 0);
            assert!(matches!(
                script.replies.front(),
                Some(RtiReply::Grant { .. })
            ));
            assert!(!script
                .requests
                .iter()
                .any(|request| matches!(request, RtiRequest::Complete { .. })));
            assert!(
                matches!(script.requests.last(), Some(RtiRequest::Abort { message }) if message.contains("scripted malformed payload"))
            );
        })
    });
    let rejected = events
        .iter()
        .find(|event| event["fields"]["event"] == "coordination.boundary.rejected")
        .unwrap();
    assert_eq!(rejected["fields"]["reason"], "decode");
    assert!(!events.iter().any(|event| matches!(
        event["fields"]["event"].as_str(),
        Some("coordination.grant.received" | "coordination.boundary.admitted")
    )));
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("scripted malformed payload"));
}

#[test]
fn dnet_reduces_net_traffic_with_identical_compiled_execution() {
    bounded(|| {
        let run = |suppress| {
            let mut replies = VecDeque::from([RtiReply::Started]);
            if suppress {
                replies.push_back(RtiReply::SuppressPublication {
                    tag: WireTag::FOREVER,
                });
            }
            replies.extend([payload(41, 1_000_000), payload(42, 2_000_000)]);
            let script = Arc::new(Mutex::new(Script {
                replies,
                single_horizon: true,
                ..Script::default()
            }));
            let execution = execute_script(script.clone()).unwrap();
            let values = execution
                .enclave(EnclaveIndex::new(1))
                .unwrap()
                .state::<RoutedSinkState>(StateSlotIndex::new(0))
                .unwrap()
                .values
                .clone();
            let script = script.lock().unwrap();
            let second_nets = script.requests.iter().filter(|r| matches!(r,
                RtiRequest::Publish { next_event: Some(tag), .. } if *tag == WireTag::finite(2_000_000, 0))).count();
            assert!(script.requests.iter().any(|r| matches!(r,
                RtiRequest::Complete { tag } if *tag == WireTag::finite(2_000_000, 0))));
            (values, second_nets)
        };
        let eager = run(false);
        let suppressed = run(true);
        assert_eq!(eager.0, [41, 42]);
        assert_eq!(suppressed.0, eager.0);
        assert!(eager.1 >= 1);
        assert_eq!(suppressed.1, 0);
    });
}

#[test]
fn outbound_payload_tightens_dnet_before_the_next_publication() {
    use boomerang_runtime::{
        CoordinationRevision, FederateCoordinationBackend, FederatePublication, TaggedPayload,
    };
    let script = Arc::new(Mutex::new(Script {
        replies: [
            RtiReply::Started,
            RtiReply::SuppressPublication {
                tag: WireTag::FOREVER,
            },
        ]
        .into(),
        ..Script::default()
    }));
    let sink = Arc::new(RequestRecorder(script.clone()));
    let view = CompiledDeploymentView::new(DEPLOYMENT).unwrap();
    let bindings = RtiClientBindings::new(&view, MEMBERS[0], IDENTITY).unwrap();
    let outbound = bindings
        .outbound_sink(sink.clone(), BoundaryId::new("pipe"))
        .unwrap();
    let mut client = CentralRtiClient::connect(
        sink,
        OrderedReplies(script.clone()),
        bindings,
        BTreeMap::new(),
        StdDuration::from_millis(10),
    )
    .unwrap();
    client.progress(StdDuration::ZERO).unwrap();
    let publication = |revision, millis| {
        FederatePublication::new(
            CoordinationRevision::new(revision),
            Some(Tag::new(Duration::milliseconds(millis), 0)),
        )
        .with_grant_horizon(Some(Tag::FOREVER))
    };
    client.publish(publication(1, 1)).unwrap();
    outbound
        .send(TaggedPayload {
            tag: Tag::new(Duration::milliseconds(1), 0),
            payload: vec![42],
        })
        .unwrap();
    client.publish(publication(2, 2)).unwrap();
    assert!(matches!(script.lock().unwrap().requests.as_slice(), [
        RtiRequest::Hello { .. }, RtiRequest::Payload { .. },
        RtiRequest::Publish { revision: 2, next_event: Some(tag) }
    ] if *tag == WireTag::finite(2_000_000, 0)));
}

#[test]
fn tightened_dnet_trace_explains_the_restored_net() {
    use boomerang_runtime::{
        CoordinationRevision, FederateCoordinationBackend, FederatePublication,
    };
    let (_, events) = capture_coordination(|| {
        let view = CompiledDeploymentView::new(DEPLOYMENT).unwrap();
        let script = Arc::new(Mutex::new(Script {
            replies: [
                RtiReply::Started,
                RtiReply::SuppressPublication {
                    tag: WireTag::FOREVER,
                },
                RtiReply::SuppressPublication { tag: WireTag::ZERO },
            ]
            .into(),
            ..Script::default()
        }));
        let mut client = CentralRtiClient::connect(
            Arc::new(RequestRecorder(script.clone())),
            OrderedReplies(script),
            RtiClientBindings::new(&view, MEMBERS[0], IDENTITY).unwrap(),
            BTreeMap::new(),
            StdDuration::from_secs(1),
        )
        .unwrap();
        client.progress(StdDuration::ZERO).unwrap();
        client
            .publish(
                FederatePublication::new(
                    CoordinationRevision::new(3),
                    Some(Tag::new(Duration::milliseconds(1), 0)),
                )
                .with_grant_horizon(Some(Tag::FOREVER)),
            )
            .unwrap();
        client.progress(StdDuration::ZERO).unwrap();
    });
    let kinds: Vec<_> = events
        .iter()
        .map(|event| event["fields"]["event"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "coordination.dnet.received",
            "coordination.publication.suppressed",
            "coordination.dnet.received",
            "coordination.publication.restored"
        ]
    );
    assert_eq!(events[1]["fields"]["revision"], 3);
    assert_eq!(events[3]["fields"]["revision"], 3);
}
