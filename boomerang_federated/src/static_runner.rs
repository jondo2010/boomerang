//! Static in-memory federated runtime runner.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use crate::{
    in_memory_transport_pair, FederateClientError, FederateClientRoute, FederateId,
    FederateProtocolClient, FederatedTopology, ProtocolFrame, RtiFederatedTimeBarrier,
    RtiSessionEndpoint, SessionError, StaticRtiSession,
};

/// Runtime parts required to execute one static in-memory federation.
pub struct StaticFederationRuntimeParts {
    pub topology: FederatedTopology,
    pub routes: Vec<FederateClientRoute>,
    pub federate_enclaves: BTreeMap<FederateId, boomerang_runtime::EnclaveKey>,
    pub enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    pub outbound_sink: boomerang_runtime::BufferedFederatedOutboundSink,
    pub inbound_endpoints: boomerang_runtime::FederatedInboundEndpointRegistry,
}

#[derive(Debug, thiserror::Error)]
pub enum StaticFederationRunnerError {
    #[error("unsupported static federation topology: {what}")]
    UnsupportedTopology { what: String },

    #[error("static federation runner error: {what}")]
    Bridge { what: String },

    #[error("federate client error: {0}")]
    FederateClient(#[from] FederateClientError),

    #[error("RTI session error: {0}")]
    Session(#[from] SessionError),

    #[error("runtime endpoint error: {0}")]
    RuntimeEndpoint(#[from] boomerang_runtime::FederatedEndpointError),

    #[error("federate scheduler thread spawn error: {0}")]
    ThreadSpawn(#[from] std::io::Error),
}

/// Execute a static federation in memory using the real RTI session and federate clients.
pub fn execute_federation_in_memory(
    parts: StaticFederationRuntimeParts,
    config: boomerang_runtime::Config,
) -> Result<
    tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Env>,
    StaticFederationRunnerError,
> {
    validate_static_runner_parts(&parts)?;

    let StaticFederationRuntimeParts {
        topology,
        routes,
        federate_enclaves,
        enclaves,
        outbound_sink,
        inbound_endpoints,
    } = parts;
    let federate_by_enclave = federate_by_enclave_map(&federate_enclaves)?;

    let mut outbound_channels = BTreeMap::new();
    let mut outbound_receivers = BTreeMap::new();
    for federate_id in federate_enclaves.keys() {
        let (channel, receiver) = boomerang_runtime::FederatedOutboundChannel::pair();
        outbound_channels.insert(federate_id.clone(), channel);
        outbound_receivers.insert(federate_id.clone(), receiver);
    }

    for route in &routes {
        let channel = outbound_channels.get(&route.source).ok_or_else(|| {
            bridge_error(format!(
                "route endpoint '{}' references source federate '{}' without a runtime enclave",
                route.endpoint.as_str(),
                route.source
            ))
        })?;
        outbound_sink.set_live_route(route.endpoint.clone(), channel.clone())?;
    }

    let mut session_endpoints = BTreeMap::new();
    let mut client_transports = BTreeMap::new();
    for federate_id in federate_enclaves.keys() {
        let (client_transport, rti_transport) =
            in_memory_transport_pair::<ProtocolFrame, ProtocolFrame>();
        let (rti_sink, rti_stream) = rti_transport;
        session_endpoints.insert(
            federate_id.clone(),
            RtiSessionEndpoint::new(rti_sink, rti_stream),
        );
        client_transports.insert(federate_id.clone(), client_transport);
    }

    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads((federate_enclaves.len() + 1).max(2))
        .enable_all()
        .build()?;
    let session = StaticRtiSession::new(topology.clone(), session_endpoints);
    let session_handle = tokio_runtime.spawn(session.run());

    let mut connect_handles = Vec::new();
    for federate_id in federate_enclaves.keys() {
        let (sink, stream) = client_transports.remove(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing client transport for federate '{federate_id}'"
            ))
        })?;
        let federate_id_for_client = federate_id.clone();
        let topology = topology.neighbors_for(federate_id);
        connect_handles.push((
            federate_id.clone(),
            tokio_runtime.spawn(async move {
                FederateProtocolClient::connect(federate_id_for_client, topology, sink, stream)
                    .await
            }),
        ));
    }

    let mut barriers = BTreeMap::new();
    for (federate_id, connect_handle) in connect_handles {
        let client = tokio_runtime.block_on(connect_handle).map_err(|error| {
            bridge_error(format!(
                "federate '{federate_id}' client task failed: {error}"
            ))
        })??;
        let outbound = outbound_receivers.remove(&federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing outbound receiver for federate '{federate_id}'"
            ))
        })?;
        let barrier = RtiFederatedTimeBarrier::new(
            federate_id.clone(),
            client,
            routes.clone(),
            outbound,
            inbound_endpoints.clone(),
        )?;
        barriers.insert(
            federate_id.clone(),
            SharedFederatedTimeBarrier::new(barrier),
        );
    }

    let mut handles = Vec::new();
    for (enclave_key, enclave) in enclaves {
        let Some(federate_id) = federate_by_enclave.get(enclave_key).cloned() else {
            if enclave.env.reactions.is_empty() {
                continue;
            }

            return Err(unsupported_topology(format!(
                "in-memory federation runner requires every non-empty runtime enclave to map to exactly one federate; enclave {enclave_key:?} is not mapped"
            )));
        };

        if enclave.env.reactions.is_empty() {
            return Err(unsupported_topology(format!(
                "federate '{federate_id}' maps to enclave {enclave_key:?}, but that enclave has no reactions; no-future federates are reserved for a later milestone"
            )));
        }

        let barrier = barriers
            .get(&federate_id)
            .expect("barriers were built from federate_enclaves")
            .clone();
        let config = config.clone();
        handles.push(
            std::thread::Builder::new()
                .name(format!("federate-{federate_id}"))
                .spawn(move || {
                    let mut scheduler =
                        boomerang_runtime::Scheduler::new_with_federated_time_barrier(
                            enclave_key,
                            enclave,
                            config,
                            barrier,
                        );
                    scheduler.event_loop();
                    (enclave_key, scheduler.into_env())
                })?,
        );
    }

    if handles.is_empty() {
        return Err(unsupported_topology(
            "in-memory federation runner found no federate scheduler enclaves",
        ));
    }

    let mut envs = tinymap::TinySecondaryMap::new();
    let mut thread_panic = None;
    for handle in handles {
        match handle.join() {
            Ok((enclave_key, env)) => {
                envs.insert(enclave_key, env);
            }
            Err(error) => {
                thread_panic = Some(format!("{error:?}"));
            }
        }
    }

    let mut barrier_error = None;
    for barrier in barriers.values() {
        if let Some(error) = barrier.take_error()? {
            barrier_error.get_or_insert_with(|| error.to_string());
        }
        if let Err(error) = barrier.stop() {
            barrier_error.get_or_insert_with(|| error.to_string());
        }
    }

    let session_result = tokio_runtime
        .block_on(session_handle)
        .map_err(|error| bridge_error(format!("RTI session task failed: {error}")))?;
    if let Err(error) = session_result {
        barrier_error.get_or_insert_with(|| error.to_string());
    }

    if let Some(error) = thread_panic {
        return Err(bridge_error(format!(
            "federate scheduler thread panicked: {error}"
        )));
    }
    if let Some(error) = barrier_error {
        return Err(bridge_error(error));
    }

    Ok(envs)
}

#[derive(Clone)]
struct SharedFederatedTimeBarrier {
    inner: Arc<Mutex<RtiFederatedTimeBarrier>>,
}

impl SharedFederatedTimeBarrier {
    fn new(barrier: RtiFederatedTimeBarrier) -> Self {
        Self {
            inner: Arc::new(Mutex::new(barrier)),
        }
    }

    fn take_error(&self) -> Result<Option<FederateClientError>, StaticFederationRunnerError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| bridge_error("federate barrier lock poisoned"))?
            .take_error())
    }

    fn stop(&self) -> Result<(), FederateClientError> {
        self.inner
            .lock()
            .map_err(|_| FederateClientError::Protocol("federate barrier lock poisoned".into()))?
            .stop_result()
    }
}

impl boomerang_runtime::FederatedTimeBarrier for SharedFederatedTimeBarrier {
    fn acquire_tag(
        &mut self,
        tag: boomerang_runtime::Tag,
        event_rx: &boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent>,
    ) -> Option<boomerang_runtime::AsyncEvent> {
        self.inner.lock().ok().and_then(|mut barrier| {
            boomerang_runtime::FederatedTimeBarrier::acquire_tag(&mut *barrier, tag, event_rx)
        })
    }

    fn logical_tag_complete(&mut self, tag: boomerang_runtime::Tag) {
        if let Ok(mut barrier) = self.inner.lock() {
            boomerang_runtime::FederatedTimeBarrier::logical_tag_complete(&mut *barrier, tag);
        }
    }
}

fn validate_static_runner_parts(
    parts: &StaticFederationRuntimeParts,
) -> Result<(), StaticFederationRunnerError> {
    if parts.topology.federates.is_empty()
        || parts.topology.edges.is_empty()
        || parts.routes.is_empty()
    {
        return Err(unsupported_topology(
            "in-memory federation runner requires a non-empty federation topology with at least one cross-federate endpoint",
        ));
    }

    let mut federates = BTreeSet::new();
    for federate_id in &parts.topology.federates {
        if federate_id.as_str().trim().is_empty() {
            return Err(bridge_error(
                "federation topology contains an empty federate id",
            ));
        }
        if !federates.insert(federate_id.clone()) {
            return Err(bridge_error(format!(
                "duplicate federate id '{federate_id}'"
            )));
        }
    }

    for federate_id in parts.federate_enclaves.keys() {
        if !federates.contains(federate_id) {
            return Err(bridge_error(format!(
                "federate '{federate_id}' has a runtime enclave but is missing from topology"
            )));
        }
    }

    for federate_id in &parts.topology.federates {
        if !parts.federate_enclaves.contains_key(federate_id) {
            return Err(unsupported_topology(format!(
                "federate '{federate_id}' has no runtime enclave"
            )));
        }
    }

    let mut edge_endpoints = BTreeSet::new();
    for edge in &parts.topology.edges {
        if edge.endpoint.as_str().trim().is_empty() {
            return Err(bridge_error(
                "federation topology contains an empty endpoint id",
            ));
        }
        if !federates.contains(&edge.source) {
            return Err(bridge_error(format!(
                "endpoint '{}' references unknown source federate '{}'",
                edge.endpoint, edge.source
            )));
        }
        if !federates.contains(&edge.target) {
            return Err(bridge_error(format!(
                "endpoint '{}' references unknown target federate '{}'",
                edge.endpoint, edge.target
            )));
        }
        if !edge_endpoints.insert(edge.endpoint.clone()) {
            return Err(bridge_error(format!(
                "duplicate topology edge endpoint '{}'",
                edge.endpoint
            )));
        }
    }

    let mut route_endpoints = BTreeSet::new();
    for route in &parts.routes {
        if route.endpoint.as_str().trim().is_empty() {
            return Err(bridge_error(
                "federation route contains an empty endpoint id",
            ));
        }
        if !federates.contains(&route.source) {
            return Err(bridge_error(format!(
                "route endpoint '{}' references unknown source federate '{}'",
                route.endpoint, route.source
            )));
        }
        if !federates.contains(&route.target) {
            return Err(bridge_error(format!(
                "route endpoint '{}' references unknown target federate '{}'",
                route.endpoint, route.target
            )));
        }
        if !route_endpoints.insert(route.endpoint.clone()) {
            return Err(bridge_error(format!(
                "duplicate route endpoint '{}'",
                route.endpoint
            )));
        }
    }

    for endpoint in edge_endpoints {
        let runtime_endpoint = boomerang_runtime::FederatedEndpointId::new(endpoint.as_str());
        if !route_endpoints.contains(&runtime_endpoint) {
            return Err(bridge_error(format!(
                "missing runtime route for topology endpoint '{}'",
                endpoint
            )));
        }
    }

    Ok(())
}

fn federate_by_enclave_map(
    federate_enclaves: &BTreeMap<FederateId, boomerang_runtime::EnclaveKey>,
) -> Result<
    tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, FederateId>,
    StaticFederationRunnerError,
> {
    let mut federate_by_enclave = tinymap::TinySecondaryMap::new();
    for (federate_id, &enclave_key) in federate_enclaves {
        if let Some(previous) = federate_by_enclave.get(enclave_key) {
            return Err(bridge_error(format!(
                "ambiguous enclave-to-federate mapping: enclave {enclave_key:?} maps to both '{previous}' and '{federate_id}'"
            )));
        }
        federate_by_enclave.insert(enclave_key, federate_id.clone());
    }

    Ok(federate_by_enclave)
}

fn unsupported_topology(what: impl Into<String>) -> StaticFederationRunnerError {
    StaticFederationRunnerError::UnsupportedTopology { what: what.into() }
}

fn bridge_error(what: impl Into<String>) -> StaticFederationRunnerError {
    StaticFederationRunnerError::Bridge { what: what.into() }
}
