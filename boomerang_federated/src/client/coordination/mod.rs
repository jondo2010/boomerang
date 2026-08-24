use std::{collections::BTreeMap, time::Duration as StdDuration};

use super::{FederateClientError, FederateClientRoute, FederateProtocolClient};
use crate::{FederateId, FederateToRti, RtiToFederate, WireTag};

const TERMINAL_DELIVERY_TIMEOUT: StdDuration = StdDuration::from_secs(1);

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtocolPoll {
    Pending,
    Progress,
    Granted(boomerang_runtime::Tag),
}

/// RTI-backed wire adapter for one federate.
///
/// Scheduler events and TAG-coverage policy belong to the Federate coordination service, not this
/// protocol adapter:
///
/// ```compile_fail
/// fn scheduler_policy_is_not_exposed_by_the_wire_adapter(
///     coordinator: &mut boomerang_federated::RtiLogicalTimeCoordinator,
///     event_rx: &boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent>,
/// ) {
///     coordinator
///         .wait_for_tag(boomerang_runtime::Tag::ZERO, event_rx)
///         .unwrap();
/// }
/// ```
#[derive(Debug)]
pub struct RtiLogicalTimeCoordinator {
    /// Stable protocol identity used for outgoing frames and inbound route validation.
    federate_id: FederateId,
    /// Persistent ordered protocol connection to the RTI.
    client: FederateProtocolClient,
    /// Federation route metadata keyed by its stable endpoint identifier.
    routes: BTreeMap<crate::EndpointId, FederateClientRoute>,
    /// Shared terminal fault reported by the runtime endpoint workers, if any.
    faults: crate::FederatedFaultState,
    /// Successfully queued `NET` request still awaiting a sufficient `TAG` response.
    pub(super) pending_request: Option<WireTag>,
    /// Whether this coordinator has entered its terminal stopped state.
    pub(super) stopped: bool,
    /// Whether an earlier protocol, transport, or admission error made further grants unsafe.
    pub(super) failed: bool,
    /// Maximum time spent waiting for an RTI frame before checking scheduler events again.
    poll_interval: StdDuration,
}

impl RtiLogicalTimeCoordinator {
    /// Create the Federate-wide RTI protocol adapter owned by the coordination service.
    /// Enclave schedulers participate through per-Enclave proxies.
    /// Route metadata binds runtime endpoints to source and target federates.
    #[tracing::instrument(
        level = "debug",
        skip(federate_id, client, routes),
        fields(federate = %federate_id)
    )]
    pub fn new(
        federate_id: FederateId,
        client: FederateProtocolClient,
        routes: impl IntoIterator<Item = FederateClientRoute>,
        faults: crate::FederatedFaultState,
    ) -> Result<Self, FederateClientError> {
        let mut route_map = BTreeMap::new();
        for route in routes {
            let endpoint = route.endpoint.clone();
            if route_map.insert(endpoint.clone(), route).is_some() {
                return Err(FederateClientError::DuplicateRoute(endpoint));
            }
        }

        Ok(Self {
            federate_id,
            client,
            routes: route_map,
            faults,
            pending_request: None,
            stopped: false,
            failed: false,
            poll_interval: StdDuration::from_millis(1),
        })
    }

    pub(crate) fn submit_net(
        &mut self,
        tag: boomerang_runtime::Tag,
    ) -> Result<(), FederateClientError> {
        if self.stopped {
            return Err(FederateClientError::RtiStopped);
        }
        if self.failed {
            return Err(FederateClientError::CoordinationFailed);
        }
        if let Err(error) = self.check_runtime_fault() {
            return self.fail(error);
        }
        let requested = WireTag::try_from(tag)?;
        if self.pending_request != Some(requested) {
            if let Err(error) = self.client.send(FederateToRti::Net {
                federate_id: self.federate_id.clone(),
                tag: requested,
            }) {
                return self.fail(error);
            }
            self.pending_request = Some(requested);
        }
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> Result<ProtocolPoll, FederateClientError> {
        let message = match self.client.recv_timeout(self.poll_interval) {
            Ok(message) => message,
            Err(error) => return self.fail(error),
        };
        let Some(message) = message else {
            return Ok(ProtocolPoll::Pending);
        };
        match message {
            RtiToFederate::Tag { tag } => {
                let runtime_tag =
                    boomerang_runtime::Tag::try_from(tag).map_err(FederateClientError::from)?;
                if self.pending_request.is_some_and(|pending| tag >= pending) {
                    self.pending_request = None;
                }
                Ok(ProtocolPoll::Granted(runtime_tag))
            }
            RtiToFederate::Msg {
                source,
                endpoint,
                tag,
                payload,
            } => {
                if let Err(error) = self.schedule_inbound_msg_now(source, endpoint, tag, &payload) {
                    return self.fail(error);
                }
                Ok(ProtocolPoll::Progress)
            }
            RtiToFederate::Stop => {
                self.pending_request = None;
                self.stopped = true;
                Err(FederateClientError::RtiStopped)
            }
            RtiToFederate::Error { message } => {
                self.fail(FederateClientError::RtiError { message })
            }
            RtiToFederate::Start { .. } => self.fail(FederateClientError::Protocol(
                "unexpected duplicate Start frame".into(),
            )),
        }
    }

    /// Report LTC after every reaction-emitted MSG has entered the ordered client mailbox.
    #[tracing::instrument(
        level = "debug",
        skip(self, tag),
        fields(federate = %self.federate_id, tag = %tag)
    )]
    pub fn report_logical_tag_complete(
        &mut self,
        tag: boomerang_runtime::Tag,
    ) -> Result<(), FederateClientError> {
        if self.failed {
            return Err(FederateClientError::CoordinationFailed);
        }
        if let Err(error) = self.check_runtime_fault() {
            return self.fail(error);
        }
        if let Err(error) = self.send_ltc(tag) {
            return self.fail(error);
        }
        Ok(())
    }

    /// Send a final Stop frame for this federate after its scheduler has terminated.
    #[tracing::instrument(
        level = "debug",
        skip(self),
        fields(federate = %self.federate_id)
    )]
    pub fn stop(&mut self) -> Result<(), FederateClientError> {
        if self.stopped {
            return Ok(());
        }

        let fault_result = self.check_runtime_fault();
        let net_result = self.client.send_confirmed(
            FederateToRti::Net {
                federate_id: self.federate_id.clone(),
                tag: WireTag::FOREVER,
            },
            TERMINAL_DELIVERY_TIMEOUT,
        );
        let stop_result = self.client.send_confirmed(
            FederateToRti::Stop {
                federate_id: self.federate_id.clone(),
            },
            TERMINAL_DELIVERY_TIMEOUT,
        );
        self.pending_request = None;
        self.stopped = true;
        fault_result?;
        net_result?;
        stop_result?;
        Ok(())
    }

    /// Schedule one inbound MSG payload through the handler attached during lowering.
    /// Returns the scheduler wake event produced by that async scheduling operation.
    #[tracing::instrument(
        level = "debug",
        skip(self, source, endpoint, tag, payload),
        fields(
            federate = %self.federate_id,
            source = %source,
            endpoint = %endpoint,
            tag = %tag,
            payload_len = payload.len()
        )
    )]
    fn schedule_inbound_msg_now(
        &mut self,
        source: FederateId,
        endpoint: crate::EndpointId,
        tag: WireTag,
        payload: &[u8],
    ) -> Result<(), FederateClientError> {
        let route = self.route_for(&endpoint)?;
        if route.target != self.federate_id {
            return Err(FederateClientError::RouteTargetMismatch {
                endpoint: endpoint.clone(),
                route_target: route.target.clone(),
                federate_id: self.federate_id.clone(),
            });
        }
        if route.source != source {
            return Err(FederateClientError::InboundSourceMismatch {
                endpoint: endpoint.clone(),
                observed_source: source,
                route_source: route.source.clone(),
            });
        }
        let runtime_tag = boomerang_runtime::Tag::try_from(tag)?;
        let inbound = route
            .inbound
            .as_ref()
            .ok_or_else(|| FederateClientError::UnboundInboundRoute(endpoint.clone()))?;
        inbound.schedule(runtime_tag, payload)?;
        Ok(())
    }

    fn check_runtime_fault(&self) -> Result<(), FederateClientError> {
        match self.faults.get() {
            Some(error) => Err(FederateClientError::RuntimeEndpoint(error)),
            None => Ok(()),
        }
    }

    fn fail<T>(&mut self, error: FederateClientError) -> Result<T, FederateClientError> {
        self.pending_request = None;
        self.failed = true;
        Err(error)
    }

    /// Send LTC for a scheduler tag through the federate protocol client.
    #[tracing::instrument(
        level = "debug",
        skip(self, tag),
        fields(federate = %self.federate_id, tag = %tag)
    )]
    fn send_ltc(&self, tag: boomerang_runtime::Tag) -> Result<(), FederateClientError> {
        self.client.send(FederateToRti::Ltc {
            federate_id: self.federate_id.clone(),
            tag: WireTag::try_from(tag)?,
        })
    }

    fn route_for(
        &self,
        endpoint: &crate::EndpointId,
    ) -> Result<&FederateClientRoute, FederateClientError> {
        self.routes
            .get(endpoint)
            .ok_or_else(|| FederateClientError::UnknownRoute(endpoint.clone()))
    }
}
