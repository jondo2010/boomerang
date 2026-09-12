//! Checked conversions and prebuilt connections between runtime and wire protocol metadata.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    FederateClientError, FederateClientMailbox, FederateClientRoute, FederateId,
    FederateProtocolSender, FederateToRti, WireDelay, WireTag,
};

#[derive(Debug, thiserror::Error)]
/// Failure converting a runtime value or lowered route to protocol state.
pub enum RuntimeBridgeError {
    /// A finite runtime tag used a negative time offset.
    #[error(
        "finite runtime tag {tag} has negative offset {offset_ns}ns; use Tag::NEVER for negative infinity"
    )]
    NegativeRuntimeTag {
        /// Runtime tag that cannot be sent as a finite wire tag.
        tag: boomerang_runtime::Tag,
        /// Negative nanosecond offset in the runtime tag.
        offset_ns: i128,
    },

    /// A runtime tag's microstep exceeds the wire representation.
    #[error("runtime tag {tag} microstep {microstep} does not fit wire u64")]
    RuntimeMicrostepOutOfRange {
        /// Runtime tag with the unrepresentable microstep.
        tag: boomerang_runtime::Tag,
        /// Microstep that cannot fit in `u64`.
        microstep: usize,
    },

    /// A finite wire tag used a negative time offset.
    #[error(
        "finite wire tag {tag} has negative offset {offset_ns}ns; use WireTag::NEVER for negative infinity"
    )]
    NegativeWireTag {
        /// Wire tag that cannot become a runtime tag.
        tag: WireTag,
        /// Negative nanosecond offset in the wire tag.
        offset_ns: i128,
    },

    /// A wire offset exceeds the runtime duration range.
    #[error("finite wire tag {tag} offset {offset_ns}ns does not fit runtime Duration")]
    WireTagOffsetOutOfRange {
        /// Wire tag with the unrepresentable offset.
        tag: WireTag,
        /// Nanosecond offset that cannot fit the runtime duration.
        offset_ns: i128,
    },

    /// A wire microstep exceeds the runtime index representation.
    #[error("finite wire tag {tag} microstep {microstep} does not fit runtime usize")]
    WireMicrostepOutOfRange {
        /// Wire tag with the unrepresentable microstep.
        tag: WireTag,
        /// Microstep that cannot fit in `usize`.
        microstep: u64,
    },

    /// A finite wire tag aliases the runtime's positive-infinity sentinel.
    #[error("finite wire tag {tag} collides with runtime Tag::FOREVER")]
    WireTagCollidesWithRuntimeForever {
        /// Finite wire tag that maps to `Tag::FOREVER`.
        tag: WireTag,
    },

    /// A cross-federate runtime delay is negative.
    #[error("cross-federate delay {delay} is negative; wire delays must be nonnegative")]
    NegativeRuntimeDelay {
        /// Runtime delay that cannot be serialized on the wire.
        delay: boomerang_runtime::Duration,
    },

    /// A runtime delay exceeds the wire nanosecond range.
    #[error("cross-federate delay {delay} does not fit wire u64 nanoseconds")]
    RuntimeDelayOutOfRange {
        /// Runtime delay that cannot fit in `u64` nanoseconds.
        delay: boomerang_runtime::Duration,
    },
}

/// Convert a runtime tag into its checked federated wire representation.
pub fn wire_tag_from_runtime(tag: boomerang_runtime::Tag) -> Result<WireTag, RuntimeBridgeError> {
    if tag == boomerang_runtime::Tag::NEVER {
        return Ok(WireTag::NEVER);
    }
    if tag == boomerang_runtime::Tag::FOREVER {
        return Ok(WireTag::FOREVER);
    }

    let offset_ns = tag.offset().whole_nanoseconds();
    if offset_ns < 0 {
        return Err(RuntimeBridgeError::NegativeRuntimeTag { tag, offset_ns });
    }

    let microstep =
        tag.microstep()
            .try_into()
            .map_err(|_| RuntimeBridgeError::RuntimeMicrostepOutOfRange {
                tag,
                microstep: tag.microstep(),
            })?;

    Ok(WireTag::finite(offset_ns, microstep))
}

/// Convert a federated wire tag into its checked runtime representation.
pub fn runtime_tag_from_wire(tag: WireTag) -> Result<boomerang_runtime::Tag, RuntimeBridgeError> {
    match tag {
        WireTag::Never => Ok(boomerang_runtime::Tag::NEVER),
        WireTag::Forever => Ok(boomerang_runtime::Tag::FOREVER),
        WireTag::Finite {
            offset_ns,
            microstep,
        } => {
            if offset_ns < 0 {
                return Err(RuntimeBridgeError::NegativeWireTag { tag, offset_ns });
            }

            let max_runtime_offset_ns = boomerang_runtime::Duration::MAX.whole_nanoseconds();
            if offset_ns > max_runtime_offset_ns {
                return Err(RuntimeBridgeError::WireTagOffsetOutOfRange { tag, offset_ns });
            }

            let microstep = microstep
                .try_into()
                .map_err(|_| RuntimeBridgeError::WireMicrostepOutOfRange { tag, microstep })?;
            let runtime_tag = boomerang_runtime::Tag::new(
                boomerang_runtime::Duration::nanoseconds_i128(offset_ns),
                microstep,
            );
            if runtime_tag == boomerang_runtime::Tag::FOREVER {
                return Err(RuntimeBridgeError::WireTagCollidesWithRuntimeForever { tag });
            }

            Ok(runtime_tag)
        }
    }
}

/// Convert a nonnegative runtime delay into its checked wire representation.
pub fn wire_delay_from_runtime(
    delay: boomerang_runtime::Duration,
) -> Result<WireDelay, RuntimeBridgeError> {
    let nanos = delay.whole_nanoseconds();
    if nanos < 0 {
        return Err(RuntimeBridgeError::NegativeRuntimeDelay { delay });
    }

    let nanos =
        u64::try_from(nanos).map_err(|_| RuntimeBridgeError::RuntimeDelayOutOfRange { delay })?;

    Ok(WireDelay::from_nanos(nanos))
}

/// Complete lowered connection state for one federate.
#[derive(Debug)]
pub(crate) struct FederatedRuntimeConnection {
    /// Ordered outbound protocol queue for this federate.
    mailbox: FederateClientMailbox,
    /// Stable endpoint routes targeting this federate.
    routes: BTreeMap<crate::EndpointId, FederateClientRoute>,
    /// Shared first-failure state for runtime endpoint workers.
    faults: boomerang_runtime::FederatedFaultState,
}

impl FederatedRuntimeConnection {
    fn new() -> Self {
        Self {
            mailbox: FederateClientMailbox::new(),
            routes: BTreeMap::new(),
            faults: boomerang_runtime::FederatedFaultState::default(),
        }
    }

    /// Consume this connection and return its prebuilt protocol mailbox.
    ///
    /// This is primarily useful to inspect lowering output without starting a runner.
    pub(crate) fn into_mailbox(self) -> FederateClientMailbox {
        self.mailbox
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        FederateClientMailbox,
        Vec<FederateClientRoute>,
        boomerang_runtime::FederatedFaultState,
    ) {
        (
            self.mailbox,
            self.routes.into_values().collect(),
            self.faults,
        )
    }
}

/// Complete per-federate connection bundles created during runtime lowering.
#[derive(Debug, Default)]
pub struct FederatedRuntimeConnections {
    /// Connection state keyed by stable federate identity.
    federates: BTreeMap<FederateId, FederatedRuntimeConnection>,
}

impl FederatedRuntimeConnections {
    /// Build complete federate connections and assign each route to its target.
    pub fn new(
        federates: impl IntoIterator<Item = FederateId>,
        routes: impl IntoIterator<Item = FederateClientRoute>,
    ) -> Result<Self, FederateClientError> {
        let mut federate_ids = BTreeSet::new();
        let mut connections = BTreeMap::new();
        for federate in federates {
            if !federate_ids.insert(federate.clone()) {
                return Err(FederateClientError::Protocol(format!(
                    "duplicate prebuilt runtime connection for '{federate}'"
                )));
            }
            connections.insert(federate, FederatedRuntimeConnection::new());
        }

        let mut route_endpoints = BTreeSet::new();
        for route in routes {
            if !route_endpoints.insert(route.endpoint.clone()) {
                return Err(FederateClientError::DuplicateRoute(route.endpoint));
            }
            if !federate_ids.contains(&route.source) {
                return Err(FederateClientError::Protocol(format!(
                    "route for endpoint '{}' references source federate '{}' without a prebuilt runtime connection",
                    route.endpoint, route.source
                )));
            }
            let target = connections.get_mut(&route.target).ok_or_else(|| {
                FederateClientError::Protocol(format!(
                    "route for endpoint '{}' references target federate '{}' without a prebuilt runtime connection",
                    route.endpoint, route.target
                ))
            })?;
            let endpoint = route.endpoint.clone();
            if target.routes.insert(endpoint.clone(), route).is_some() {
                return Err(FederateClientError::DuplicateRoute(endpoint));
            }
        }

        Ok(Self {
            federates: connections,
        })
    }

    /// Build the runtime sink and shared fault state for an outbound endpoint.
    pub fn outbound_endpoint(
        &self,
        endpoint: &crate::EndpointId,
    ) -> Result<
        (
            Box<dyn boomerang_runtime::OutboundBoundarySink>,
            boomerang_runtime::FederatedFaultState,
        ),
        FederateClientError,
    > {
        let route = self
            .federates
            .values()
            .find_map(|connection| connection.routes.get(endpoint))
            .ok_or_else(|| FederateClientError::UnknownRoute(endpoint.clone()))?;
        let source = self
            .federates
            .get(&route.source)
            .expect("route sources are validated when connections are built");
        Ok((
            Box::new(ProtocolOutboundBoundarySink {
                endpoint: route.endpoint.clone(),
                source: route.source.clone(),
                target: route.target.clone(),
                sender: source.mailbox.sender(),
            }),
            source.faults.clone(),
        ))
    }

    /// Attach a typed runtime receiver to one lowered inbound route.
    pub fn register_inbound<T>(
        &mut self,
        federate: &FederateId,
        endpoint: crate::EndpointId,
        context: boomerang_runtime::SendContext,
        action_ref: boomerang_runtime::AsyncActionRef<T>,
        decoder: Box<dyn boomerang_runtime::PayloadDecoder<T>>,
    ) -> Result<(), FederateClientError>
    where
        T: boomerang_runtime::ReactorData,
    {
        let connection = self.federates.get_mut(federate).ok_or_else(|| {
            FederateClientError::Protocol(format!(
                "missing prebuilt runtime connection for federate '{federate}'"
            ))
        })?;
        let route = connection
            .routes
            .get_mut(&endpoint)
            .ok_or_else(|| FederateClientError::UnknownRoute(endpoint.clone()))?;
        if route.inbound().is_some() {
            return Err(FederateClientError::DuplicateInboundBinding(endpoint));
        }
        let inbound =
            boomerang_runtime::LegacyInboundActionAdapter::new(context, action_ref, decoder)?;
        route.bind_inbound(inbound);
        Ok(())
    }

    pub(crate) fn take_federate(
        &mut self,
        federate: &FederateId,
    ) -> Option<FederatedRuntimeConnection> {
        self.federates.remove(federate)
    }

    /// Consume one federate's prebuilt mailbox for direct lowering inspection.
    pub fn take_mailbox(&mut self, federate: &FederateId) -> Option<FederateClientMailbox> {
        self.take_federate(federate)
            .map(FederatedRuntimeConnection::into_mailbox)
    }

    /// Return the lowered runtime handler attached to a stable inbound route.
    pub fn inbound_endpoint(
        &self,
        federate: &FederateId,
        endpoint: &crate::EndpointId,
    ) -> Option<&boomerang_runtime::LegacyInboundActionAdapter> {
        self.federates
            .get(federate)?
            .routes
            .get(endpoint)?
            .inbound()
    }

    /// Iterate over every lowered route in deterministic federate order.
    pub fn routes(&self) -> impl Iterator<Item = &FederateClientRoute> {
        self.federates
            .values()
            .flat_map(|connection| connection.routes.values())
    }

    /// Report whether lowering created a connection for `federate`.
    pub fn contains_federate(&self, federate: &FederateId) -> bool {
        self.federates.contains_key(federate)
    }

    /// Return the number of federates with prebuilt connection state.
    pub fn len(&self) -> usize {
        self.federates.len()
    }

    /// Report whether no federate connection state was created.
    pub fn is_empty(&self) -> bool {
        self.federates.is_empty()
    }
}

/// Runtime-facing outbound sink that emits one protocol `MSG` route.
struct ProtocolOutboundBoundarySink {
    /// Stable endpoint selected during lowering.
    endpoint: crate::EndpointId,
    /// Federate that owns the outbound runtime endpoint.
    source: FederateId,
    /// Federate selected as the protocol message target.
    target: FederateId,
    /// Ordered federate-to-RTI protocol queue.
    sender: FederateProtocolSender,
}

impl boomerang_runtime::OutboundBoundarySink for ProtocolOutboundBoundarySink {
    fn send(
        &self,
        message: boomerang_runtime::TaggedPayload,
    ) -> Result<(), boomerang_runtime::BoundarySubmissionError> {
        let tag = wire_tag_from_runtime(message.tag)
            .map_err(|error| boomerang_runtime::BoundarySubmissionError::new(error.to_string()))?;
        self.sender
            .send(FederateToRti::Msg {
                source: self.source.clone(),
                target: self.target.clone(),
                endpoint: self.endpoint.clone(),
                tag,
                payload: message.payload,
            })
            .map_err(|error| boomerang_runtime::BoundarySubmissionError::new(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_runtime_bridge_error<T>(result: Result<T, RuntimeBridgeError>, expected: &str) {
        assert!(matches!(result, Err(error) if error.to_string().contains(expected)));
    }

    #[test]
    fn tag_bridge_round_trips_runtime_sentinels_and_finite_tags() {
        for tag in [
            boomerang_runtime::Tag::NEVER,
            boomerang_runtime::Tag::ZERO,
            boomerang_runtime::Tag::new(boomerang_runtime::Duration::nanoseconds(42), 7),
            boomerang_runtime::Tag::FOREVER,
        ] {
            let wire_tag = wire_tag_from_runtime(tag).unwrap();
            assert_eq!(runtime_tag_from_wire(wire_tag).unwrap(), tag);
        }
    }

    #[test]
    fn tag_bridge_rejects_negative_finite_tags() {
        assert_eq!(
            wire_tag_from_runtime(boomerang_runtime::Tag::NEVER).unwrap(),
            WireTag::NEVER
        );
        assert_runtime_bridge_error(
            wire_tag_from_runtime(boomerang_runtime::Tag::new(
                boomerang_runtime::Duration::nanoseconds(-1),
                0,
            )),
            "negative offset",
        );
        assert_runtime_bridge_error(
            runtime_tag_from_wire(WireTag::finite(-1, 0)),
            "negative offset",
        );
    }

    #[test]
    fn tag_bridge_rejects_wire_values_outside_runtime_representation() {
        let too_large = boomerang_runtime::Duration::MAX.whole_nanoseconds() + 1;
        assert_runtime_bridge_error(
            runtime_tag_from_wire(WireTag::finite(too_large, 0)),
            "does not fit runtime Duration",
        );

        #[cfg(target_pointer_width = "64")]
        assert_runtime_bridge_error(
            runtime_tag_from_wire(WireTag::finite(
                boomerang_runtime::Duration::MAX.whole_nanoseconds(),
                u64::MAX,
            )),
            "collides with runtime Tag::FOREVER",
        );
    }

    #[test]
    fn delay_bridge_rejects_invalid_wire_delays() {
        assert_eq!(
            wire_delay_from_runtime(boomerang_runtime::Duration::ZERO).unwrap(),
            WireDelay::ZERO
        );
        assert_eq!(
            wire_delay_from_runtime(boomerang_runtime::Duration::nanoseconds(5))
                .unwrap()
                .as_nanos(),
            5
        );
        assert_runtime_bridge_error(
            wire_delay_from_runtime(boomerang_runtime::Duration::nanoseconds(-1)),
            "negative",
        );
        assert_runtime_bridge_error(
            wire_delay_from_runtime(boomerang_runtime::Duration::nanoseconds_i128(
                i128::from(u64::MAX) + 1,
            )),
            "does not fit wire u64",
        );
    }

    #[test]
    fn prebuilt_outbound_sink_emits_exact_protocol_message() {
        let source = FederateId::new("source");
        let target = FederateId::new("target");
        let endpoint = crate::EndpointId::new("source/out->target/in");
        let route = FederateClientRoute::new(endpoint.clone(), source.clone(), target.clone());
        let mut connections =
            FederatedRuntimeConnections::new([source.clone(), target.clone()], [route]).unwrap();
        let (sink, _) = connections.outbound_endpoint(&endpoint).unwrap();

        sink.send(boomerang_runtime::TaggedPayload {
            tag: boomerang_runtime::Tag::ZERO,
            payload: b"7".to_vec(),
        })
        .unwrap();

        let mut mailbox = connections.take_federate(&source).unwrap().into_mailbox();
        assert_eq!(
            mailbox.try_recv().unwrap(),
            Some(FederateToRti::Msg {
                source,
                target,
                endpoint,
                tag: WireTag::ZERO,
                payload: b"7".to_vec(),
            })
        );
        assert_eq!(mailbox.try_recv().unwrap(), None);
    }

    #[test]
    fn outbound_messages_enter_the_shared_mailbox_before_later_progress() {
        let source = FederateId::new("source");
        let target = FederateId::new("target");
        let endpoint = crate::EndpointId::new("source/out->target/in");
        let route = FederateClientRoute::new(endpoint.clone(), source.clone(), target.clone());
        let mut connections =
            FederatedRuntimeConnections::new([source.clone(), target.clone()], [route]).unwrap();
        let (sink, _) = connections.outbound_endpoint(&endpoint).unwrap();
        let progress = connections
            .federates
            .get(&source)
            .expect("source connection exists")
            .mailbox
            .sender();

        for payload in [b"first".to_vec(), b"second".to_vec()] {
            sink.send(boomerang_runtime::TaggedPayload {
                tag: boomerang_runtime::Tag::ZERO,
                payload,
            })
            .unwrap();
        }
        progress
            .send(FederateToRti::Ltc {
                federate_id: source.clone(),
                tag: WireTag::ZERO,
            })
            .unwrap();
        progress
            .send(FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::finite(0, 1),
            })
            .unwrap();

        let mut mailbox = connections.take_mailbox(&source).unwrap();
        for payload in [b"first".to_vec(), b"second".to_vec()] {
            assert_eq!(
                mailbox.try_recv().unwrap(),
                Some(FederateToRti::Msg {
                    source: source.clone(),
                    target: target.clone(),
                    endpoint: endpoint.clone(),
                    tag: WireTag::ZERO,
                    payload,
                })
            );
        }
        assert_eq!(
            mailbox.try_recv().unwrap(),
            Some(FederateToRti::Ltc {
                federate_id: source.clone(),
                tag: WireTag::ZERO,
            })
        );
        assert_eq!(
            mailbox.try_recv().unwrap(),
            Some(FederateToRti::Net {
                federate_id: source,
                tag: WireTag::finite(0, 1),
            })
        );
    }

    #[test]
    fn prebuilt_connections_reject_route_without_source_connection() {
        let endpoint = crate::EndpointId::new("source/out->target/in");
        let error = FederatedRuntimeConnections::new(
            [FederateId::new("target")],
            [FederateClientRoute::new(endpoint, "source", "target")],
        )
        .expect_err("route source must be validated while connections are built");

        assert!(error
            .to_string()
            .contains("without a prebuilt runtime connection"));
    }

    #[test]
    fn runtime_connections_attach_inbound_endpoints_to_target_routes() {
        let source = FederateId::new("source");
        let first = FederateId::new("first");
        let second = FederateId::new("second");
        let first_endpoint = crate::EndpointId::new("source/out->first/in");
        let second_endpoint = crate::EndpointId::new("source/out->second/in");
        let mut connections = FederatedRuntimeConnections::new(
            [source, first.clone(), second.clone()],
            [
                FederateClientRoute::new(first_endpoint.clone(), "source", first.clone()),
                FederateClientRoute::new(second_endpoint.clone(), "source", second.clone()),
            ],
        )
        .unwrap();

        let mut first_enclave = boomerang_runtime::Enclave::default();
        let first_action = first_enclave.insert_action(|key| {
            boomerang_runtime::Action::<u32>::new("first", key, None, true).boxed()
        });
        connections
            .register_inbound(
                &first,
                first_endpoint.clone(),
                first_enclave.create_send_context(boomerang_runtime::EnclaveKey::from(0)),
                first_enclave.create_async_action_ref(first_action),
                Box::new(|bytes: &[u8]| {
                    std::str::from_utf8(bytes)
                        .unwrap()
                        .parse::<u32>()
                        .map_err(|error| {
                            boomerang_runtime::PayloadCodecError::new(error.to_string())
                        })
                }),
            )
            .unwrap();

        let mut second_enclave = boomerang_runtime::Enclave::default();
        let second_action = second_enclave.insert_action(|key| {
            boomerang_runtime::Action::<u32>::new("second", key, None, true).boxed()
        });
        connections
            .register_inbound(
                &second,
                second_endpoint.clone(),
                second_enclave.create_send_context(boomerang_runtime::EnclaveKey::from(1)),
                second_enclave.create_async_action_ref(second_action),
                Box::new(|bytes: &[u8]| {
                    std::str::from_utf8(bytes)
                        .unwrap()
                        .parse::<u32>()
                        .map_err(|error| {
                            boomerang_runtime::PayloadCodecError::new(error.to_string())
                        })
                }),
            )
            .unwrap();

        connections
            .inbound_endpoint(&first, &first_endpoint)
            .expect("first route must own its lowered inbound handler")
            .schedule(boomerang_runtime::Tag::ZERO, b"7")
            .unwrap();
        assert!(first_enclave.event_rx.recv().is_ok());
        assert!(connections
            .inbound_endpoint(&second, &second_endpoint)
            .is_some());
        assert!(connections
            .inbound_endpoint(&first, &second_endpoint)
            .is_none());
    }
}
