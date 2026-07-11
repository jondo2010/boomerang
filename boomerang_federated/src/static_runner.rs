//! Static federated runtime runners.

#[cfg(feature = "serde-json-codec")]
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

#[cfg(feature = "serde-json-codec")]
use futures_util::StreamExt;
use futures_util::{Sink, TryStream};

use crate::{
    in_memory_transport_pair, FederateClientError, FederateClientRoute, FederateId,
    FederateProtocolClient, FederatedTopology, ProtocolFrame, RtiFederatedTimeBarrier,
    RtiSessionEndpoint, SessionError, StaticRtiSession, TransportError,
};
#[cfg(feature = "serde-json-codec")]
use crate::{json_protocol_frame_transport, run_tcp_static_rti_session};

/// Runtime parts required to execute one static federation.
pub struct StaticFederationRuntimeParts {
    pub topology: FederatedTopology,
    pub routes: Vec<FederateClientRoute>,
    pub federate_enclaves: BTreeMap<FederateId, boomerang_runtime::EnclaveKey>,
    pub enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    pub outbound_sink: boomerang_runtime::BufferedFederatedOutboundSink,
    pub faults: boomerang_runtime::FederatedFaultState,
    pub inbound_endpoints: boomerang_runtime::FederatedInboundEndpointRegistry,
}

/// TCP listener configuration for the single-process static federation runner.
#[cfg(feature = "serde-json-codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpStaticFederationConfig {
    pub bind_addr: SocketAddr,
}

#[cfg(feature = "serde-json-codec")]
impl Default for TcpStaticFederationConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        }
    }
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

    #[error("failed to build the static federation Tokio runtime: {source}")]
    RuntimeBuild {
        #[source]
        source: std::io::Error,
    },

    #[error("failed to bind the static federation TCP listener at {addr}: {source}")]
    #[cfg(feature = "serde-json-codec")]
    TcpBind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read the static federation TCP listener address: {source}")]
    #[cfg(feature = "serde-json-codec")]
    TcpLocalAddress {
        #[source]
        source: std::io::Error,
    },

    #[error("failed to connect federate `{federate_id}` to {addr}: {source}")]
    #[cfg(feature = "serde-json-codec")]
    TcpConnect {
        federate_id: FederateId,
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },

    #[error("federate `{federate_id}` client task failed: {source}")]
    ClientTask {
        federate_id: FederateId,
        #[source]
        source: tokio::task::JoinError,
    },

    #[error("federate `{federate_id}` client connection failed: {source}")]
    ClientConnect {
        federate_id: FederateId,
        #[source]
        source: FederateClientError,
    },

    #[error("RTI session task failed: {source}")]
    SessionTask {
        #[source]
        source: tokio::task::JoinError,
    },

    #[error("failed to spawn scheduler thread for federate `{federate_id}`: {source}")]
    SchedulerThreadSpawn {
        federate_id: FederateId,
        #[source]
        source: std::io::Error,
    },

    #[error("federate scheduler thread panicked: {what}")]
    SchedulerThreadPanic { what: String },

    #[error("federate `{federate_id}` scheduler failed: {source}")]
    SchedulerRuntime {
        federate_id: FederateId,
        #[source]
        source: boomerang_runtime::RuntimeError,
    },
}

type FederationEnvs =
    tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Env>;
type SessionHandle = tokio::task::JoinHandle<Result<(), SessionError>>;

struct PreparedStaticFederation {
    topology: FederatedTopology,
    routes: Vec<FederateClientRoute>,
    federate_enclaves: BTreeMap<FederateId, boomerang_runtime::EnclaveKey>,
    federate_by_enclave: tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, FederateId>,
    enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    outbound_channels: BTreeMap<FederateId, boomerang_runtime::FederatedOutboundChannel>,
    outbound_receivers: BTreeMap<FederateId, boomerang_runtime::FederatedOutboundReceiver>,
    inbound_endpoints: boomerang_runtime::FederatedInboundEndpointRegistry,
    faults: boomerang_runtime::FederatedFaultState,
}

/// Execute a static federation in memory using the real RTI session and federate clients.
pub fn execute_federation_in_memory(
    parts: StaticFederationRuntimeParts,
    config: boomerang_runtime::Config,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let prepared = prepare_static_federation(parts)?;
    let tokio_runtime = build_tokio_runtime(prepared.federate_enclaves.len())?;
    let mut session_endpoints = BTreeMap::new();
    let mut client_transports = BTreeMap::new();
    for federate_id in &prepared.topology.federates {
        let (client_transport, rti_transport) =
            in_memory_transport_pair::<ProtocolFrame, ProtocolFrame>();
        let (rti_sink, rti_stream) = rti_transport;
        session_endpoints.insert(
            federate_id.clone(),
            RtiSessionEndpoint::new(rti_sink, rti_stream),
        );
        client_transports.insert(federate_id.clone(), client_transport);
    }

    let session = StaticRtiSession::new(prepared.topology.clone(), session_endpoints);
    let session_handle = tokio_runtime.spawn(session.run());
    let clients = connect_clients(&tokio_runtime, &prepared.topology, client_transports)?;

    execute_connected_static_federation(prepared, config, &tokio_runtime, session_handle, clients)
}

/// Execute a static federation over TCP using the shared RTI session and federate clients.
#[cfg(feature = "serde-json-codec")]
pub fn execute_federation_over_tcp(
    parts: StaticFederationRuntimeParts,
    config: boomerang_runtime::Config,
    tcp: TcpStaticFederationConfig,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let prepared = prepare_static_federation(parts)?;
    let tokio_runtime = build_tokio_runtime(prepared.federate_enclaves.len())?;
    let listener = tokio_runtime
        .block_on(tokio::net::TcpListener::bind(tcp.bind_addr))
        .map_err(|source| StaticFederationRunnerError::TcpBind {
            addr: tcp.bind_addr,
            source,
        })?;
    let listener_addr = listener
        .local_addr()
        .map_err(|source| StaticFederationRunnerError::TcpLocalAddress { source })?;
    let connect_addr = listener_connect_addr(listener_addr);
    let session_handle = tokio_runtime.spawn(run_tcp_static_rti_session(
        listener,
        prepared.topology.clone(),
    ));

    let mut client_transports = BTreeMap::new();
    for federate_id in &prepared.topology.federates {
        let stream = match tokio_runtime.block_on(tokio::net::TcpStream::connect(connect_addr)) {
            Ok(stream) => stream,
            Err(source) => {
                session_handle.abort();
                return Err(StaticFederationRunnerError::TcpConnect {
                    federate_id: federate_id.clone(),
                    addr: connect_addr,
                    source,
                });
            }
        };
        let (sink, stream) = json_protocol_frame_transport(stream).split();
        client_transports.insert(federate_id.clone(), (sink, stream));
    }

    let clients = match connect_clients(&tokio_runtime, &prepared.topology, client_transports) {
        Ok(clients) => clients,
        Err(error) => {
            session_handle.abort();
            return Err(error);
        }
    };

    execute_connected_static_federation(prepared, config, &tokio_runtime, session_handle, clients)
}

fn prepare_static_federation(
    parts: StaticFederationRuntimeParts,
) -> Result<PreparedStaticFederation, StaticFederationRunnerError> {
    validate_static_runner_parts(&parts)?;

    let StaticFederationRuntimeParts {
        topology,
        routes,
        federate_enclaves,
        enclaves,
        outbound_sink,
        faults,
        inbound_endpoints,
    } = parts;
    let federate_by_enclave = federate_by_enclave_map(&federate_enclaves)?;

    for (enclave_key, enclave) in enclaves.iter() {
        if federate_by_enclave.get(enclave_key).is_none() && !enclave.env.reactions.is_empty() {
            return Err(unsupported_topology(format!(
                "static federation runner requires every non-empty runtime enclave to map to exactly one federate; enclave {enclave_key:?} is not mapped"
            )));
        }
    }

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

    Ok(PreparedStaticFederation {
        topology,
        routes,
        federate_enclaves,
        federate_by_enclave,
        enclaves,
        outbound_channels,
        outbound_receivers,
        inbound_endpoints,
        faults,
    })
}

fn build_tokio_runtime(
    federate_count: usize,
) -> Result<tokio::runtime::Runtime, StaticFederationRunnerError> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads((federate_count + 1).max(2))
        .enable_all()
        .build()
        .map_err(|source| StaticFederationRunnerError::RuntimeBuild { source })
}

fn connect_clients<S, R>(
    tokio_runtime: &tokio::runtime::Runtime,
    topology: &FederatedTopology,
    mut transports: BTreeMap<FederateId, (S, R)>,
) -> Result<BTreeMap<FederateId, FederateProtocolClient>, StaticFederationRunnerError>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError> + Send + 'static,
    R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
    R::Error: Into<TransportError> + Send + 'static,
{
    let mut connect_handles = Vec::new();
    for federate_id in &topology.federates {
        let (sink, stream) = transports.remove(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing client transport for federate '{federate_id}'"
            ))
        })?;
        let federate_id_for_client = federate_id.clone();
        let neighbors = topology.neighbors_for(federate_id);
        connect_handles.push((
            federate_id.clone(),
            tokio_runtime.spawn(async move {
                FederateProtocolClient::connect(federate_id_for_client, neighbors, sink, stream)
                    .await
            }),
        ));
    }

    let mut clients = BTreeMap::new();
    for (federate_id, connect_handle) in connect_handles {
        let client = tokio_runtime.block_on(connect_handle).map_err(|source| {
            StaticFederationRunnerError::ClientTask {
                federate_id: federate_id.clone(),
                source,
            }
        })?;
        let client = client.map_err(|source| StaticFederationRunnerError::ClientConnect {
            federate_id: federate_id.clone(),
            source,
        })?;
        clients.insert(federate_id, client);
    }

    Ok(clients)
}

fn execute_connected_static_federation(
    prepared: PreparedStaticFederation,
    config: boomerang_runtime::Config,
    tokio_runtime: &tokio::runtime::Runtime,
    session_handle: SessionHandle,
    mut clients: BTreeMap<FederateId, FederateProtocolClient>,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let PreparedStaticFederation {
        topology,
        routes,
        federate_enclaves,
        federate_by_enclave,
        enclaves,
        outbound_channels: _outbound_channels,
        mut outbound_receivers,
        inbound_endpoints,
        faults,
    } = prepared;

    let mut barriers = BTreeMap::new();
    for federate_id in federate_enclaves.keys() {
        let client = clients.remove(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing connected client for federate '{federate_id}'"
            ))
        })?;
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
            faults.clone(),
        )?;
        barriers.insert(
            federate_id.clone(),
            SharedFederatedTimeBarrier::new(barrier),
        );
    }

    let mut envs = tinymap::TinySecondaryMap::new();
    let mut barrier_error = None;
    let mut handles: Vec<
        std::thread::JoinHandle<(
            FederateId,
            boomerang_runtime::EnclaveKey,
            boomerang_runtime::Env,
            Result<(), boomerang_runtime::RuntimeError>,
            Result<(), FederateClientError>,
        )>,
    > = Vec::new();
    for (enclave_key, enclave) in enclaves {
        let Some(federate_id) = federate_by_enclave.get(enclave_key).cloned() else {
            if enclave.env.reactions.is_empty() {
                continue;
            }

            unreachable!("non-empty unmapped enclaves were rejected during preparation");
        };

        let barrier = barriers
            .get(&federate_id)
            .expect("barriers were built from federate_enclaves")
            .clone();

        if federate_has_no_initial_work(&enclave, &topology, &federate_id) {
            let boomerang_runtime::Enclave { env, .. } = enclave;
            envs.insert(enclave_key, env);
            if let Err(error) = barrier.stop() {
                barrier_error.get_or_insert_with(|| error.to_string());
            }
            continue;
        }

        let config = config.clone();
        let thread_federate_id = federate_id.clone();
        let handle = match std::thread::Builder::new()
            .name(format!("federate-{federate_id}"))
            .spawn(move || {
                let stop_barrier = barrier.clone();
                let mut scheduler = boomerang_runtime::Scheduler::new_with_federated_time_barrier(
                    enclave_key,
                    enclave,
                    config,
                    barrier,
                );
                let scheduler_result = scheduler.try_event_loop();
                let env = scheduler.into_env();
                let stop_result = stop_barrier.stop();
                (
                    thread_federate_id,
                    enclave_key,
                    env,
                    scheduler_result,
                    stop_result,
                )
            }) {
            Ok(handle) => handle,
            Err(source) => {
                for barrier in barriers.values() {
                    let _ = barrier.stop();
                }
                session_handle.abort();
                for handle in handles {
                    let _ = handle.join();
                }
                return Err(StaticFederationRunnerError::SchedulerThreadSpawn {
                    federate_id,
                    source,
                });
            }
        };
        handles.push(handle);
    }

    let mut thread_panic = None;
    let mut scheduler_error = None;
    for handle in handles {
        match handle.join() {
            Ok((federate_id, enclave_key, env, scheduler_result, stop_result)) => {
                envs.insert(enclave_key, env);
                if let Err(source) = scheduler_result {
                    scheduler_error.get_or_insert(StaticFederationRunnerError::SchedulerRuntime {
                        federate_id,
                        source,
                    });
                }
                if let Err(error) = stop_result {
                    barrier_error.get_or_insert_with(|| error.to_string());
                }
            }
            Err(error) => {
                thread_panic = Some(format!("{error:?}"));
            }
        }
    }

    for barrier in barriers.values() {
        if let Err(error) = barrier.stop() {
            barrier_error.get_or_insert_with(|| error.to_string());
        }
    }

    let session_result = tokio_runtime
        .block_on(session_handle)
        .map_err(|source| StaticFederationRunnerError::SessionTask { source })?;

    if let Some(error) = thread_panic {
        return Err(StaticFederationRunnerError::SchedulerThreadPanic { what: error });
    }
    if let Some(error) = scheduler_error {
        return Err(error);
    }
    if let Some(error) = barrier_error {
        return Err(bridge_error(error));
    }
    session_result?;

    Ok(envs)
}

#[cfg(feature = "serde-json-codec")]
fn listener_connect_addr(listener_addr: SocketAddr) -> SocketAddr {
    match listener_addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), listener_addr.port())
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), listener_addr.port())
        }
        _ => listener_addr,
    }
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
    ) -> Result<boomerang_runtime::FederatedBarrierOutcome, boomerang_runtime::FederatedBarrierError>
    {
        let mut barrier = self.inner.lock().map_err(|_| {
            boomerang_runtime::FederatedBarrierError::new("federate barrier lock poisoned")
        })?;
        boomerang_runtime::FederatedTimeBarrier::acquire_tag(&mut *barrier, tag, event_rx)
    }

    fn logical_tag_complete(
        &mut self,
        tag: boomerang_runtime::Tag,
    ) -> Result<(), boomerang_runtime::FederatedBarrierError> {
        let mut barrier = self.inner.lock().map_err(|_| {
            boomerang_runtime::FederatedBarrierError::new("federate barrier lock poisoned")
        })?;
        boomerang_runtime::FederatedTimeBarrier::logical_tag_complete(&mut *barrier, tag)
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
            "static federation runner requires a non-empty federation topology with at least one cross-federate endpoint",
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

fn federate_has_no_initial_work(
    enclave: &boomerang_runtime::Enclave,
    topology: &FederatedTopology,
    federate_id: &FederateId,
) -> bool {
    enclave.env.reactions.is_empty()
        || (enclave.graph.startup_actions.is_empty()
            && topology.incoming_edges(federate_id).next().is_none())
}

fn unsupported_topology(what: impl Into<String>) -> StaticFederationRunnerError {
    StaticFederationRunnerError::UnsupportedTopology { what: what.into() }
}

fn bridge_error(what: impl Into<String>) -> StaticFederationRunnerError {
    StaticFederationRunnerError::Bridge { what: what.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "serde-json-codec")]
    #[test]
    fn tcp_config_defaults_to_ephemeral_ipv4_loopback() {
        assert_eq!(
            TcpStaticFederationConfig::default().bind_addr,
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
        );
    }

    #[cfg(feature = "serde-json-codec")]
    #[test]
    fn wildcard_listener_addresses_connect_through_same_family_loopback() {
        assert_eq!(
            listener_connect_addr(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 4321))),
            SocketAddr::from((Ipv4Addr::LOCALHOST, 4321))
        );
        assert_eq!(
            listener_connect_addr(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 4321))),
            SocketAddr::from((Ipv6Addr::LOCALHOST, 4321))
        );
        assert_eq!(
            listener_connect_addr(SocketAddr::from(([192, 0, 2, 1], 4321))),
            SocketAddr::from(([192, 0, 2, 1], 4321))
        );
    }

    #[cfg(feature = "serde-json-codec")]
    #[test]
    fn tcp_runner_validates_parts_before_binding() {
        let parts = StaticFederationRuntimeParts {
            topology: FederatedTopology::default(),
            routes: Vec::new(),
            federate_enclaves: BTreeMap::new(),
            enclaves: tinymap::TinyMap::new(),
            outbound_sink: boomerang_runtime::BufferedFederatedOutboundSink::default(),
            faults: boomerang_runtime::FederatedFaultState::default(),
            inbound_endpoints: boomerang_runtime::FederatedInboundEndpointRegistry::default(),
        };
        let tcp = TcpStaticFederationConfig {
            bind_addr: SocketAddr::from(([203, 0, 113, 1], 1)),
        };

        let error = execute_federation_over_tcp(parts, boomerang_runtime::Config::default(), tcp)
            .expect_err("invalid runtime parts must fail before TCP bind");

        assert!(matches!(
            error,
            StaticFederationRunnerError::UnsupportedTopology { what }
                if what.contains("non-empty federation topology")
        ));
    }
}
