//! Ordered scripted replies exercise the production client and real compiled scheduler.
use super::*;
use boomerang_central_rti::compiled::{CentralRtiError, RtiReplySource, RtiRequestSink};
use std::{collections::VecDeque, sync::Mutex, time::Duration as StdDuration};

#[derive(Default)]
struct Script {
    replies: VecDeque<RtiReply>,
    requests: Vec<RtiRequest>,
    decoded: Vec<u32>,
    observed_grants: usize,
}

struct RequestRecorder(Arc<Mutex<Script>>);
impl RtiRequestSink for RequestRecorder {
    fn send(&self, request: RtiRequest) -> Result<(), CentralRtiError> {
        let mut script = self.0.lock().unwrap();
        match request {
            RtiRequest::Publish { revision, next_event: Some(tag) } => {
                script.replies.push_back(RtiReply::Grant { revision, tag });
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
            script.observed_grants += 1;
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
        assert_eq!(script.decoded, [41, 42]);
        assert!(script.observed_grants > 0);
        assert!(matches!(script.requests.last(), Some(RtiRequest::Stop)));
    });
}

#[test]
fn decode_failure_terminates_execution_before_queued_grant() {
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
    });
}
