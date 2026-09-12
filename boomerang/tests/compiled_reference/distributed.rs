//! Compiled network route adapters reuse the owned execution fixture and scheduler.
use super::*;
use boomerang::runtime::{
    execute_owned_federate_with_backend, CoordinationRevision, FederateAcquisition,
    FederateCompletion, FederateCoordinationBackend, FederateCoordinationError,
    FederatePublication, FederatedEndpointError, FederatedOutboundCommand,
    FederatedOutboundMessage, FederatedOutboundSink,
};
use std::sync::{Arc, Mutex};

/// Immediate grants isolate executor wiring from the separately tested RTI algorithm.
#[derive(Default)]
struct GrantBackend {
    /// Publication waiting for a grant.
    pending: Option<FederatePublication>,
    /// Injected transport failure at the first acquisition poll.
    fail: bool,
}

impl FederateCoordinationBackend for GrantBackend {
    fn publish(
        &mut self,
        publication: FederatePublication,
    ) -> Result<(), FederateCoordinationError> {
        self.pending = Some(publication);
        Ok(())
    }
    fn progress(
        &mut self,
        _: std::time::Duration,
    ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
        if self.fail {
            return Err(FederateCoordinationError::BackendAcquire {
                message: "transport lost".into(),
            });
        }
        Ok(self.pending.take().and_then(|p| {
            p.next_event()
                .map(|tag| FederateAcquisition::new(p.revision(), tag))
        }))
    }
    fn complete(&mut self, _: FederateCompletion) -> Result<(), FederateCoordinationError> {
        Ok(())
    }
    fn confirm_idle(&mut self, _: CoordinationRevision) -> Result<bool, FederateCoordinationError> {
        Ok(true)
    }
    fn stop(&mut self) -> Result<(), FederateCoordinationError> {
        Ok(())
    }
}

/// Captures encoded transport submissions, before any destination scheduler exists.
#[derive(Default)]
struct CaptureSink(
    /// Submitted messages in transport order.
    Mutex<Vec<FederatedOutboundMessage>>,
);
impl FederatedOutboundSink for CaptureSink {
    fn send(&self, command: FederatedOutboundCommand) -> Result<(), FederatedEndpointError> {
        let FederatedOutboundCommand::Msg(message) = command;
        self.0.lock().unwrap().push(message);
        Ok(())
    }
}

/// Decodes the fixture's explicit four-byte little-endian contract.
fn decode_u32(bytes: &[u8]) -> Result<u32, FederatedEndpointError> {
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| {
        FederatedEndpointError::codec("expected four bytes")
    })?))
}

/// Proves independent owned slices exchange the encoded value with one delay application.
#[test]
fn isolated_slices_execute_encoded_route_halves_at_canonical_enclave_keys() {
    bounded(|| {
        let wire = Arc::new(CaptureSink::default());
        let source = execute_owned_federate_with_backend(
            FederateIndex::new(3),
            fixture_federate("source", "host", "std", IndexSpan::new(5, 1)),
            &[ROUTED_SOURCE_IMAGE],
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(5), source_bindings())
                .bind_outbound_route(
                    route_boundary(),
                    PayloadType::<u32>::new(),
                    |value: &u32| Ok(value.to_le_bytes().to_vec()),
                    wire.clone(),
                ),
            Config::default().with_fast_forward(true),
            |inbound| {
                assert!(inbound.is_empty());
                Ok(GrantBackend::default())
            },
        )
        .unwrap();
        assert!(source.enclave(EnclaveIndex::new(0)).is_none());
        assert_eq!(
            source.enclave(EnclaveIndex::new(5)).unwrap().final_tag(),
            Tag::ZERO
        );
        let messages = wire.0.lock().unwrap().clone();
        assert_eq!(
            messages,
            [FederatedOutboundMessage {
                tag: Tag::new(Duration::milliseconds(1), 0),
                payload: vec![42, 0, 0, 0],
            }]
        );
        let sink = execute_owned_federate_with_backend(
            FederateIndex::new(9),
            fixture_federate("sink", "host", "std", IndexSpan::new(11, 1)),
            &[ROUTED_SINK_IMAGE],
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(11), sink_bindings())
                .bind_inbound_route(route_boundary(), PayloadType::<u32>::new(), decode_u32),
            Config::default().with_fast_forward(true),
            |inbound| {
                let endpoint = inbound.get(&route_boundary()).unwrap();
                assert!(endpoint.schedule(messages[0].tag, &[1]).is_err());
                endpoint
                    .schedule(messages[0].tag, &messages[0].payload)
                    .unwrap();
                Ok(GrantBackend::default())
            },
        )
        .unwrap();
        let result = sink.enclave(EnclaveIndex::new(11)).unwrap();
        assert_eq!(
            result
                .state::<RoutedSinkState>(StateSlotIndex::new(0))
                .unwrap()
                .values,
            [42]
        );
        assert_eq!(result.final_tag(), Tag::new(Duration::milliseconds(1), 0));
    });
}

/// Verifies backend failure releases an idle scheduler through shared supervision.
#[test]
fn slice_backend_failure_wakes_and_joins_an_idle_scheduler() {
    bounded(|| {
        let error = execute_owned_federate_with_backend(
            FederateIndex::new(9),
            fixture_federate("sink", "host", "std", IndexSpan::new(11, 1)),
            &[ROUTED_SINK_IMAGE],
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(11), sink_bindings())
                .bind_inbound_route(route_boundary(), PayloadType::<u32>::new(), decode_u32),
            Config::default().with_fast_forward(true),
            |_| {
                Ok(GrantBackend {
                    fail: true,
                    ..Default::default()
                })
            },
        )
        .unwrap_err();
        assert!(
            matches!(error, ExecuteOwnedFederateError::FederateCoordination {
            source: FederateCoordinationError::BackendAcquire { message }
        } if message == "transport lost")
        );
    });
}

/// Transport sink that exposes submission failures without blocking the scheduler.
struct RejectedSink;

impl FederatedOutboundSink for RejectedSink {
    fn send(&self, _: FederatedOutboundCommand) -> Result<(), FederatedEndpointError> {
        Err(FederatedEndpointError::send("transport rejected payload"))
    }
}

/// Verifies both codec and sink failures retain their typed source through cleanup.
#[test]
fn slice_preserves_codec_and_submission_failures_through_scheduler_cleanup() {
    bounded(|| {
        for reject_codec in [true, false] {
            let error = execute_owned_federate_with_backend(
                FederateIndex::new(3),
                fixture_federate("source", "host", "std", IndexSpan::new(5, 1)),
                &[ROUTED_SOURCE_IMAGE],
                FederateBindings::new()
                    .bind_enclave(EnclaveIndex::new(5), source_bindings())
                    .bind_outbound_route(
                        route_boundary(),
                        PayloadType::<u32>::new(),
                        move |value: &u32| {
                            if reject_codec {
                                Err(FederatedEndpointError::codec("encoding rejected value"))
                            } else {
                                Ok(value.to_le_bytes().to_vec())
                            }
                        },
                        Arc::new(RejectedSink),
                    ),
                Config::default().with_fast_forward(true),
                |_| Ok(GrantBackend::default()),
            )
            .unwrap_err();
            let expected = if reject_codec {
                FederatedEndpointError::codec("encoding rejected value")
            } else {
                FederatedEndpointError::send("transport rejected payload")
            };
            assert!(
                matches!(error, ExecuteOwnedFederateError::EnclaveExecution {
                enclave, source: OwnedStorageError::ExternalRoute { boundary, source }
            } if enclave == EnclaveIndex::new(5)
                && boundary == route_boundary().as_str() && source == expected)
            );
        }
    });
}

/// Initializations exclusive to the sliced preflight test.
static SLICE_INITIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

/// Counts user initialization so invalid slice inputs cannot silently reach it.
fn counted_slice_sink() -> RoutedSinkState {
    SLICE_INITIALIZATIONS.fetch_add(1, Ordering::SeqCst);
    initialize_routed_sink()
}

/// Verifies malformed slice bindings cannot execute user initialization or connection code.
#[test]
fn slice_preflight_rejects_external_binding_errors_before_initialization_or_connect() {
    SLICE_INITIALIZATIONS.store(0, Ordering::SeqCst);
    for case in 0..7 {
        let owned = EnclaveBindings::new()
            .bind_state(BindingSlotIndex::new(0), counted_slice_sink)
            .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
            .bind_reaction(BindingSlotIndex::new(1), receive_routed_value);
        let mut bindings = FederateBindings::new().bind_enclave(EnclaveIndex::new(11), owned);
        bindings = match case {
            0 => bindings,
            1 => bindings.bind_outbound_route(
                route_boundary(),
                PayloadType::<u32>::new(),
                |_: &u32| Ok(Vec::new()),
                Arc::new(CaptureSink::default()),
            ),
            2 => bindings.bind_inbound_route(
                route_boundary(),
                PayloadType::<u64>::new(),
                |_: &[u8]| Ok(0_u64),
            ),
            _ => {
                bindings.bind_inbound_route(route_boundary(), PayloadType::<u32>::new(), decode_u32)
            }
        };
        if case == 3 || case == 4 {
            bindings = bindings.bind_inbound_route(
                if case == 3 {
                    route_boundary()
                } else {
                    BoundaryId::new("absent")
                },
                PayloadType::<u32>::new(),
                decode_u32,
            );
        }
        let span = match case {
            5 => IndexSpan::new(11, 2),
            6 => IndexSpan::new(usize::MAX, 1),
            _ => IndexSpan::new(11, 1),
        };
        let error = execute_owned_federate_with_backend(
            FederateIndex::new(9),
            fixture_federate("sink", "host", "std", span),
            &[ROUTED_SINK_IMAGE],
            bindings,
            Config::default(),
            |_| -> Result<GrantBackend, FederateCoordinationError> {
                panic!("invalid slice reached connect")
            },
        )
        .unwrap_err();
        let expected = match case {
            0 => matches!(error, ExecuteOwnedFederateError::MissingRouteBinding { .. }),
            1 | 5 | 6 => matches!(error, ExecuteOwnedFederateError::ImageValidation { .. }),
            2 => matches!(
                error,
                ExecuteOwnedFederateError::RoutePayloadTypeMismatch { .. }
            ),
            3 => matches!(
                error,
                ExecuteOwnedFederateError::DuplicateRouteBinding { .. }
            ),
            4 => matches!(
                error,
                ExecuteOwnedFederateError::UnexpectedRouteBinding { .. }
            ),
            _ => unreachable!(),
        };
        assert!(expected, "case {case}: {error:?}");
        assert_eq!(SLICE_INITIALIZATIONS.load(Ordering::SeqCst), 0);
    }
}
