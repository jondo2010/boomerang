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

/// All outbound protocol mailboxes and endpoint bindings created during runtime lowering.
#[derive(Debug, Default)]
pub struct FederatedRuntimeConnections {
    routes: BTreeMap<boomerang_runtime::FederatedEndpointId, FederateClientRoute>,
    mailboxes: BTreeMap<FederateId, FederateClientMailbox>,
}

impl FederatedRuntimeConnections {
    pub fn new(
        federates: impl IntoIterator<Item = FederateId>,
        routes: impl IntoIterator<Item = FederateClientRoute>,
    ) -> Result<Self, FederateClientError> {
        let mut federate_ids = BTreeSet::new();
        let mut mailboxes = BTreeMap::new();
        for federate in federates {
            if !federate_ids.insert(federate.clone()) {
                return Err(FederateClientError::Protocol(format!(
                    "duplicate prebuilt federate mailbox for '{federate}'"
                )));
            }
            mailboxes.insert(federate, FederateClientMailbox::new());
        }

        let mut route_map = BTreeMap::new();
        for route in routes {
            if !federate_ids.contains(&route.source) {
                return Err(FederateClientError::Protocol(format!(
                    "route for endpoint '{}' references source federate '{}' without a prebuilt mailbox",
                    route.endpoint, route.source
                )));
            }
            if !federate_ids.contains(&route.target) {
                return Err(FederateClientError::Protocol(format!(
                    "route for endpoint '{}' references target federate '{}' without a prebuilt mailbox",
                    route.endpoint, route.target
                )));
            }
            if route_map
                .insert(route.endpoint.clone(), route.clone())
                .is_some()
            {
                return Err(FederateClientError::DuplicateRoute(route.endpoint));
            }
        }

        Ok(Self {
            routes: route_map,
            mailboxes,
        })
    }

    pub fn outbound_sink(
        &self,
        endpoint: &boomerang_runtime::FederatedEndpointId,
    ) -> Result<Box<dyn boomerang_runtime::FederatedOutboundSink>, FederateClientError> {
        let route = self
            .routes
            .get(endpoint)
            .ok_or_else(|| FederateClientError::UnknownRoute(endpoint.clone()))?
            .clone();
        let sender = self
            .mailboxes
            .get(&route.source)
            .expect("route sources are validated when connections are built")
            .sender();
        Ok(Box::new(ProtocolFederatedOutboundSink { route, sender }))
    }

    pub fn take_mailbox(&mut self, federate: &FederateId) -> Option<FederateClientMailbox> {
        self.mailboxes.remove(federate)
    }

    pub fn routes(&self) -> impl Iterator<Item = &FederateClientRoute> {
        self.routes.values()
    }

    pub fn contains_mailbox(&self, federate: &FederateId) -> bool {
        self.mailboxes.contains_key(federate)
    }

    pub fn mailbox_count(&self) -> usize {
        self.mailboxes.len()
    }
}

struct ProtocolFederatedOutboundSink {
    route: FederateClientRoute,
    sender: FederateProtocolSender,
}

impl boomerang_runtime::FederatedOutboundSink for ProtocolFederatedOutboundSink {
    fn send(
        &self,
        command: boomerang_runtime::FederatedOutboundCommand,
    ) -> Result<(), boomerang_runtime::FederatedEndpointError> {
        let boomerang_runtime::FederatedOutboundCommand::Msg(message) = command;
        if message.endpoint != self.route.endpoint {
            return Err(boomerang_runtime::FederatedEndpointError::UnknownEndpoint(
                message.endpoint,
            ));
        }

        let tag = WireTag::try_from(message.tag)
            .map_err(|error| boomerang_runtime::FederatedEndpointError::send(error.to_string()))?;
        self.sender
            .send(FederateToRti::Msg {
                source: self.route.source.clone(),
                target: self.route.target.clone(),
                endpoint: crate::EndpointId::new(message.endpoint.as_str()),
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
        let endpoint = boomerang_runtime::FederatedEndpointId::new("source/out->target/in");
        let route = FederateClientRoute::new(endpoint.clone(), source.clone(), target.clone());
        let mut connections =
            FederatedRuntimeConnections::new([source.clone(), target.clone()], [route]).unwrap();
        let sink = connections.outbound_sink(&endpoint).unwrap();

        sink.send(boomerang_runtime::FederatedOutboundCommand::Msg(
            boomerang_runtime::FederatedOutboundMessage {
                endpoint: endpoint.clone(),
                tag: boomerang_runtime::Tag::ZERO,
                payload: b"7".to_vec(),
            },
        ))
        .unwrap();

        let mut mailbox = connections.take_mailbox(&source).unwrap();
        assert_eq!(
            mailbox.try_recv().unwrap(),
            Some(FederateToRti::Msg {
                source,
                target,
                endpoint: crate::EndpointId::new(endpoint.as_str()),
                tag: WireTag::ZERO,
                payload: b"7".to_vec(),
            })
        );
        assert_eq!(mailbox.try_recv().unwrap(), None);
    }

    #[test]
    fn prebuilt_connections_reject_route_without_source_mailbox() {
        let endpoint = boomerang_runtime::FederatedEndpointId::new("source/out->target/in");
        let error = FederatedRuntimeConnections::new(
            [FederateId::new("target")],
            [FederateClientRoute::new(endpoint, "source", "target")],
        )
        .expect_err("route source must be validated while connections are built");

        assert!(error.to_string().contains("without a prebuilt mailbox"));
    }
}
