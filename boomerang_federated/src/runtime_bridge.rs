//! Checked conversions and prebuilt connections between runtime and wire protocol metadata.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    FederateClientError, FederateClientMailbox, FederateClientRoute, FederateId,
    FederateProtocolSender, FederateToRti, WireDelay, WireTag,
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeBridgeError {
    #[error(
        "finite runtime tag {tag} has negative offset {offset_ns}ns; use Tag::NEVER for negative infinity"
    )]
    NegativeRuntimeTag {
        tag: boomerang_runtime::Tag,
        offset_ns: i128,
    },

    #[error("runtime tag {tag} microstep {microstep} does not fit wire u64")]
    RuntimeMicrostepOutOfRange {
        tag: boomerang_runtime::Tag,
        microstep: usize,
    },

    #[error(
        "finite wire tag {tag} has negative offset {offset_ns}ns; use WireTag::NEVER for negative infinity"
    )]
    NegativeWireTag { tag: WireTag, offset_ns: i128 },

    #[error("finite wire tag {tag} offset {offset_ns}ns does not fit runtime Duration")]
    WireTagOffsetOutOfRange { tag: WireTag, offset_ns: i128 },

    #[error("finite wire tag {tag} microstep {microstep} does not fit runtime usize")]
    WireMicrostepOutOfRange { tag: WireTag, microstep: u64 },

    #[error("finite wire tag {tag} collides with runtime Tag::FOREVER")]
    WireTagCollidesWithRuntimeForever { tag: WireTag },

    #[error("cross-federate delay {delay} is negative; wire delays must be nonnegative")]
    NegativeRuntimeDelay { delay: boomerang_runtime::Duration },

    #[error("cross-federate delay {delay} does not fit wire u64 nanoseconds")]
    RuntimeDelayOutOfRange { delay: boomerang_runtime::Duration },
}

impl TryFrom<boomerang_runtime::Tag> for WireTag {
    type Error = RuntimeBridgeError;

    fn try_from(tag: boomerang_runtime::Tag) -> Result<Self, Self::Error> {
        if tag == boomerang_runtime::Tag::NEVER {
            return Ok(Self::NEVER);
        }
        if tag == boomerang_runtime::Tag::FOREVER {
            return Ok(Self::FOREVER);
        }

        let offset_ns = tag.offset().whole_nanoseconds();
        if offset_ns < 0 {
            return Err(RuntimeBridgeError::NegativeRuntimeTag { tag, offset_ns });
        }

        let microstep = tag.microstep().try_into().map_err(|_| {
            RuntimeBridgeError::RuntimeMicrostepOutOfRange {
                tag,
                microstep: tag.microstep(),
            }
        })?;

        Ok(Self::finite(offset_ns, microstep))
    }
}

impl TryFrom<WireTag> for boomerang_runtime::Tag {
    type Error = RuntimeBridgeError;

    fn try_from(tag: WireTag) -> Result<Self, Self::Error> {
        match tag {
            WireTag::Never => Ok(Self::NEVER),
            WireTag::Forever => Ok(Self::FOREVER),
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
                let runtime_tag = Self::new(
                    boomerang_runtime::Duration::nanoseconds_i128(offset_ns),
                    microstep,
                );
                if runtime_tag == Self::FOREVER {
                    return Err(RuntimeBridgeError::WireTagCollidesWithRuntimeForever { tag });
                }

                Ok(runtime_tag)
            }
        }
    }
}

impl TryFrom<boomerang_runtime::Duration> for WireDelay {
    type Error = RuntimeBridgeError;

    fn try_from(delay: boomerang_runtime::Duration) -> Result<Self, Self::Error> {
        let nanos = delay.whole_nanoseconds();
        if nanos < 0 {
            return Err(RuntimeBridgeError::NegativeRuntimeDelay { delay });
        }

        let nanos = u64::try_from(nanos)
            .map_err(|_| RuntimeBridgeError::RuntimeDelayOutOfRange { delay })?;

        Ok(Self::from_nanos(nanos))
    }
}

/// Complete lowered connection state for one federate.
#[derive(Debug)]
pub(crate) struct FederatedRuntimeConnection {
    mailbox: FederateClientMailbox,
    routes: BTreeMap<crate::EndpointId, FederateClientRoute>,
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
    federates: BTreeMap<FederateId, FederatedRuntimeConnection>,
}

impl FederatedRuntimeConnections {
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

    pub fn outbound_endpoint(
        &self,
        endpoint: &crate::EndpointId,
    ) -> Result<
        (
            Box<dyn boomerang_runtime::FederatedOutboundSink>,
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
            Box::new(ProtocolFederatedOutboundSink {
                endpoint: route.endpoint.clone(),
                source: route.source.clone(),
                target: route.target.clone(),
                sender: source.mailbox.sender(),
            }),
            source.faults.clone(),
        ))
    }

    pub fn register_inbound<T>(
        &mut self,
        federate: &FederateId,
        endpoint: crate::EndpointId,
        context: boomerang_runtime::SendContext,
        action_ref: boomerang_runtime::AsyncActionRef<T>,
        decoder: Box<dyn boomerang_runtime::FederatedPayloadDecoder<T>>,
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
            boomerang_runtime::FederatedInboundEndpoint::new(context, action_ref, decoder)?;
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
    ) -> Option<&boomerang_runtime::FederatedInboundEndpoint> {
        self.federates
            .get(federate)?
            .routes
            .get(endpoint)?
            .inbound()
    }

    pub fn routes(&self) -> impl Iterator<Item = &FederateClientRoute> {
        self.federates
            .values()
            .flat_map(|connection| connection.routes.values())
    }

    pub fn contains_federate(&self, federate: &FederateId) -> bool {
        self.federates.contains_key(federate)
    }

    pub fn len(&self) -> usize {
        self.federates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.federates.is_empty()
    }
}

struct ProtocolFederatedOutboundSink {
    endpoint: crate::EndpointId,
    source: FederateId,
    target: FederateId,
    sender: FederateProtocolSender,
}

impl boomerang_runtime::FederatedOutboundSink for ProtocolFederatedOutboundSink {
    fn send(
        &self,
        command: boomerang_runtime::FederatedOutboundCommand,
    ) -> Result<(), boomerang_runtime::FederatedEndpointError> {
        let boomerang_runtime::FederatedOutboundCommand::Msg(message) = command;
        let tag = WireTag::try_from(message.tag)
            .map_err(|error| boomerang_runtime::FederatedEndpointError::send(error.to_string()))?;
        self.sender
            .send(FederateToRti::Msg {
                source: self.source.clone(),
                target: self.target.clone(),
                endpoint: self.endpoint.clone(),
                tag,
                payload: message.payload,
            })
            .map_err(|error| boomerang_runtime::FederatedEndpointError::send(error.to_string()))
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
            let wire_tag = WireTag::try_from(tag).unwrap();
            assert_eq!(boomerang_runtime::Tag::try_from(wire_tag).unwrap(), tag);
        }
    }

    #[test]
    fn tag_bridge_rejects_negative_finite_tags() {
        assert_eq!(
            WireTag::try_from(boomerang_runtime::Tag::NEVER).unwrap(),
            WireTag::NEVER
        );
        assert_runtime_bridge_error(
            WireTag::try_from(boomerang_runtime::Tag::new(
                boomerang_runtime::Duration::nanoseconds(-1),
                0,
            )),
            "negative offset",
        );
        assert_runtime_bridge_error(
            boomerang_runtime::Tag::try_from(WireTag::finite(-1, 0)),
            "negative offset",
        );
    }

    #[test]
    fn tag_bridge_rejects_wire_values_outside_runtime_representation() {
        let too_large = boomerang_runtime::Duration::MAX.whole_nanoseconds() + 1;
        assert_runtime_bridge_error(
            boomerang_runtime::Tag::try_from(WireTag::finite(too_large, 0)),
            "does not fit runtime Duration",
        );

        #[cfg(target_pointer_width = "64")]
        assert_runtime_bridge_error(
            boomerang_runtime::Tag::try_from(WireTag::finite(
                boomerang_runtime::Duration::MAX.whole_nanoseconds(),
                u64::MAX,
            )),
            "collides with runtime Tag::FOREVER",
        );
    }

    #[test]
    fn delay_bridge_rejects_invalid_wire_delays() {
        assert_eq!(
            WireDelay::try_from(boomerang_runtime::Duration::ZERO).unwrap(),
            WireDelay::ZERO
        );
        assert_eq!(
            WireDelay::try_from(boomerang_runtime::Duration::nanoseconds(5))
                .unwrap()
                .as_nanos(),
            5
        );
        assert_runtime_bridge_error(
            WireDelay::try_from(boomerang_runtime::Duration::nanoseconds(-1)),
            "negative",
        );
        assert_runtime_bridge_error(
            WireDelay::try_from(boomerang_runtime::Duration::nanoseconds_i128(
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

        sink.send(boomerang_runtime::FederatedOutboundCommand::Msg(
            boomerang_runtime::FederatedOutboundMessage {
                tag: boomerang_runtime::Tag::ZERO,
                payload: b"7".to_vec(),
            },
        ))
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
            sink.send(boomerang_runtime::FederatedOutboundCommand::Msg(
                boomerang_runtime::FederatedOutboundMessage {
                    tag: boomerang_runtime::Tag::ZERO,
                    payload,
                },
            ))
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
                            boomerang_runtime::FederatedEndpointError::codec(error.to_string())
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
                            boomerang_runtime::FederatedEndpointError::codec(error.to_string())
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
