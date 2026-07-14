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

#[cfg(feature = "serde-json-codec")]
use crate::json_protocol_frame_transport;
#[cfg(feature = "serde-json-codec")]
use crate::transport::run_tcp_static_rti_session_compiled;
use crate::{
    in_memory_transport_pair, CompiledTopology, FederateClientError, FederateClientRoute,
    FederateId, FederateProtocolClient, FederatedTopology, ProtocolFrame, RtiFederatedTimeBarrier,
    RtiSessionEndpoint, SessionError, StaticRtiSession, TransportError,
};

/// Fully lowered federation-specific state required by a static runner.
pub struct StaticFederationRuntime {
    /// Validated RTI topology and its precomputed coordination indexes.
    topology: CompiledTopology,
    /// Runtime enclave assigned to each protocol federate identity.
    federate_enclaves: BTreeMap<FederateId, boomerang_runtime::EnclaveKey>,
    /// Prebuilt protocol mailboxes, routes, inbound handlers, and fault state.
    connections: crate::FederatedRuntimeConnections,
}

impl StaticFederationRuntime {
    /// Create static runner state from artifacts produced during lowering.
    pub fn new(
        topology: CompiledTopology,
        federate_enclaves: BTreeMap<FederateId, boomerang_runtime::EnclaveKey>,
        connections: crate::FederatedRuntimeConnections,
    ) -> Self {
        Self {
            topology,
            federate_enclaves,
            connections,
        }
    }

    /// Return the validated topology and its precomputed coordination indexes.
    pub fn topology(&self) -> &CompiledTopology {
        &self.topology
    }

    /// Return the runtime enclave assigned to each protocol federate identity.
    pub fn federate_enclaves(&self) -> &BTreeMap<FederateId, boomerang_runtime::EnclaveKey> {
        &self.federate_enclaves
    }

    /// Return the prebuilt runtime connections.
    pub fn connections(&self) -> &crate::FederatedRuntimeConnections {
        &self.connections
    }

    /// Return mutable access to the prebuilt runtime connections during lowering.
    pub fn connections_mut(&mut self) -> &mut crate::FederatedRuntimeConnections {
        &mut self.connections
    }
}

/// TCP listener configuration for the single-process static federation runner.
#[cfg(feature = "serde-json-codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpStaticFederationConfig {
    /// Socket address on which the runner-owned RTI listener should bind.
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

    #[error("unsupported static federation configuration: {what}")]
    UnsupportedConfiguration { what: String },

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

/// Final runtime environments returned for each executed enclave.
pub type FederationEnvs =
    tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Env>;
type SessionHandle = tokio::task::JoinHandle<Result<(), SessionError>>;
type SchedulerThreadResult = (
    FederateId,
    boomerang_runtime::EnclaveKey,
    boomerang_runtime::Env,
    Result<(), boomerang_runtime::RuntimeError>,
    Result<(), FederateClientError>,
);
type SchedulerThreadHandle = std::thread::JoinHandle<SchedulerThreadResult>;

struct PreparedStaticFederation {
    /// Validated RTI topology shared with the runner-owned session.
    topology: CompiledTopology,
    /// Runtime enclave assigned to each protocol federate identity.
    federate_enclaves: BTreeMap<FederateId, boomerang_runtime::EnclaveKey>,
    /// Reverse lookup used to assign each runtime enclave to one federate.
    federate_by_enclave: tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, FederateId>,
    /// Fully lowered runtime enclaves awaiting scheduler construction.
    enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
}

struct ConnectedFederate {
    /// Connected protocol client used by the federate's time barrier.
    client: FederateProtocolClient,
    /// Validated inbound message routes owned by this federate.
    routes: Vec<FederateClientRoute>,
    /// Runtime endpoint registry used to admit routed payloads.
    inbound: boomerang_runtime::FederatedInboundEndpointRegistry,
    /// Shared first-error state for protocol and runtime endpoint failures.
    faults: boomerang_runtime::FederatedFaultState,
}

/// Run a lowered static federation in memory using the real RTI session and federate clients.
pub fn run_in_memory(
    runtime: StaticFederationRuntime,
    enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    config: boomerang_runtime::Config,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let (prepared, connections) = prepare_static_federation(runtime, enclaves)?;
    validate_static_runner_config(&config)?;
    let tokio_runtime = build_tokio_runtime(prepared.federate_enclaves.len())?;
    let mut session_endpoints = BTreeMap::new();
    let mut client_transports = BTreeMap::new();
    for federate_id in &prepared.topology.topology().federates {
        let (client_transport, rti_transport) =
            in_memory_transport_pair::<ProtocolFrame, ProtocolFrame>();
        let (rti_sink, rti_stream) = rti_transport;
        session_endpoints.insert(
            federate_id.clone(),
            RtiSessionEndpoint::new(rti_sink, rti_stream),
        );
        client_transports.insert(federate_id.clone(), client_transport);
    }

    let session = StaticRtiSession::from_compiled(prepared.topology.clone(), session_endpoints);
    let session_handle = tokio_runtime.spawn(session.run());
    let clients = connect_clients(
        &tokio_runtime,
        prepared.topology.topology(),
        connections,
        client_transports,
    )?;

    execute_connected_static_federation(prepared, config, &tokio_runtime, session_handle, clients)
}

/// Run a lowered static federation over TCP using the shared RTI session and federate clients.
#[cfg(feature = "serde-json-codec")]
pub fn run_over_tcp(
    runtime: StaticFederationRuntime,
    enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    config: boomerang_runtime::Config,
    tcp: TcpStaticFederationConfig,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let (prepared, connections) = prepare_static_federation(runtime, enclaves)?;
    validate_static_runner_config(&config)?;
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
    let session_handle = tokio_runtime.spawn(run_tcp_static_rti_session_compiled(
        listener,
        prepared.topology.clone(),
    ));

    let mut client_transports = BTreeMap::new();
    for federate_id in &prepared.topology.topology().federates {
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

    let clients = match connect_clients(
        &tokio_runtime,
        prepared.topology.topology(),
        connections,
        client_transports,
    ) {
        Ok(clients) => clients,
        Err(error) => {
            session_handle.abort();
            return Err(error);
        }
    };

    execute_connected_static_federation(prepared, config, &tokio_runtime, session_handle, clients)
}

fn prepare_static_federation(
    runtime: StaticFederationRuntime,
    enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
) -> Result<
    (PreparedStaticFederation, crate::FederatedRuntimeConnections),
    StaticFederationRunnerError,
> {
    validate_static_runner_runtime(&runtime)?;

    let StaticFederationRuntime {
        topology,
        federate_enclaves,
        connections,
    } = runtime;
    let federate_by_enclave = federate_by_enclave_map(&federate_enclaves)?;

    for (enclave_key, enclave) in enclaves.iter() {
        if federate_by_enclave.get(enclave_key).is_none() && !enclave.env.reactions.is_empty() {
            return Err(unsupported_topology(format!(
                "static federation runner requires every non-empty runtime enclave to map to exactly one federate; enclave {enclave_key:?} is not mapped"
            )));
        }
    }

    Ok((
        PreparedStaticFederation {
            topology,
            federate_enclaves,
            federate_by_enclave,
            enclaves,
        },
        connections,
    ))
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
    mut connections: crate::FederatedRuntimeConnections,
    mut transports: BTreeMap<FederateId, (S, R)>,
) -> Result<BTreeMap<FederateId, ConnectedFederate>, StaticFederationRunnerError>
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
        let connection = connections.take_federate(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing prebuilt runtime connection for federate '{federate_id}'"
            ))
        })?;
        let (mailbox, routes, inbound, faults) = connection.into_parts();
        connect_handles.push((
            federate_id.clone(),
            routes,
            inbound,
            faults,
            tokio_runtime.spawn(async move {
                FederateProtocolClient::connect_with_mailbox(
                    federate_id_for_client,
                    neighbors,
                    sink,
                    stream,
                    mailbox,
                )
                .await
            }),
        ));
    }

    let mut clients = BTreeMap::new();
    for (federate_id, routes, inbound, faults, connect_handle) in connect_handles {
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
        clients.insert(
            federate_id,
            ConnectedFederate {
                client,
                routes,
                inbound,
                faults,
            },
        );
    }

    Ok(clients)
}

fn execute_connected_static_federation(
    prepared: PreparedStaticFederation,
    config: boomerang_runtime::Config,
    tokio_runtime: &tokio::runtime::Runtime,
    session_handle: SessionHandle,
    mut clients: BTreeMap<FederateId, ConnectedFederate>,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let PreparedStaticFederation {
        topology,
        federate_enclaves,
        federate_by_enclave,
        enclaves,
    } = prepared;

    let mut barriers = BTreeMap::new();
    for federate_id in federate_enclaves.keys() {
        let connected = clients.remove(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing connected client for federate '{federate_id}'"
            ))
        })?;
        let barrier = RtiFederatedTimeBarrier::new(
            federate_id.clone(),
            connected.client,
            connected.routes,
            connected.inbound,
            connected.faults,
        )?;
        barriers.insert(
            federate_id.clone(),
            SharedFederatedTimeBarrier::new(barrier),
        );
    }

    let mut envs = tinymap::TinySecondaryMap::new();
    let mut barrier_error = None;
    let mut handles: Vec<SchedulerThreadHandle> = Vec::new();
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

        if federate_has_no_initial_work(&enclave, topology.topology(), &federate_id) {
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
    /// Shared barrier implementation serialized across scheduler calls.
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
            .stop()
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

fn validate_static_runner_runtime(
    runtime: &StaticFederationRuntime,
) -> Result<(), StaticFederationRunnerError> {
    let topology = runtime.topology.topology();
    if topology.federates.is_empty()
        || topology.edges.is_empty()
        || runtime.connections.routes().next().is_none()
    {
        return Err(unsupported_topology(
            "static federation runner requires a non-empty federation topology with at least one cross-federate endpoint",
        ));
    }

    let mut federates = BTreeSet::new();
    for federate_id in &topology.federates {
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

    for federate_id in runtime.federate_enclaves.keys() {
        if !federates.contains(federate_id) {
            return Err(bridge_error(format!(
                "federate '{federate_id}' has a runtime enclave but is missing from topology"
            )));
        }
    }

    for federate_id in &topology.federates {
        if !runtime.federate_enclaves.contains_key(federate_id) {
            return Err(unsupported_topology(format!(
                "federate '{federate_id}' has no runtime enclave"
            )));
        }
    }

    if runtime.connections.len() != federates.len() {
        return Err(bridge_error(format!(
            "prebuilt runtime connection count {} does not match topology federate count {}",
            runtime.connections.len(),
            federates.len()
        )));
    }
    for federate_id in &topology.federates {
        if !runtime.connections.contains_federate(federate_id) {
            return Err(bridge_error(format!(
                "federate '{federate_id}' is missing its prebuilt runtime connection"
            )));
        }
    }

    let mut edge_endpoints = BTreeSet::new();
    for edge in &topology.edges {
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
    for route in runtime.connections.routes() {
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
        if !route_endpoints.contains(&endpoint) {
            return Err(bridge_error(format!(
                "missing runtime route for topology endpoint '{}'",
                endpoint
            )));
        }
    }

    Ok(())
}

fn validate_static_runner_config(
    config: &boomerang_runtime::Config,
) -> Result<(), StaticFederationRunnerError> {
    if config.fast_forward {
        Ok(())
    } else {
        Err(StaticFederationRunnerError::UnsupportedConfiguration {
            what: "static federation currently requires Config::with_fast_forward(true) because a common physical start is not implemented".into(),
        })
    }
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

    fn valid_empty_static_runtime() -> (
        StaticFederationRuntime,
        tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    ) {
        let source = FederateId::new("source");
        let sink = FederateId::new("sink");
        let endpoint = crate::EndpointId::new("source.out->sink.in");
        let mut enclaves = tinymap::TinyMap::new();
        let source_enclave = enclaves.insert(boomerang_runtime::Enclave::default());
        let sink_enclave = enclaves.insert(boomerang_runtime::Enclave::default());

        let route = FederateClientRoute::new(endpoint.clone(), source.clone(), sink.clone());
        let runtime = StaticFederationRuntime::new(
            CompiledTopology::new(FederatedTopology::with_edges(
                [source.clone(), sink.clone()],
                [crate::TopologyEdge::new(
                    source.clone(),
                    sink.clone(),
                    endpoint.clone(),
                    crate::WireDelay::ZERO,
                )],
            ))
            .unwrap(),
            BTreeMap::from([
                (source.clone(), source_enclave),
                (sink.clone(), sink_enclave),
            ]),
            crate::FederatedRuntimeConnections::new([source, sink], [route]).unwrap(),
        );
        (runtime, enclaves)
    }

    #[test]
    fn unsupported_configuration_rejects_wall_clock_static_federation() {
        let (runtime, enclaves) = valid_empty_static_runtime();
        let error = run_in_memory(runtime, enclaves, boomerang_runtime::Config::default())
            .expect_err("wall-clock static federation must be rejected");

        assert!(matches!(
            error,
            StaticFederationRunnerError::UnsupportedConfiguration { what }
                if what.contains("with_fast_forward(true)")
                    && what.contains("common physical start")
        ));

        let (runtime, enclaves) = valid_empty_static_runtime();
        run_in_memory(
            runtime,
            enclaves,
            boomerang_runtime::Config::default().with_fast_forward(true),
        )
        .expect("fast-forward static federation should pass configuration validation");
    }

    #[test]
    fn prebuilt_runtime_connections_are_required_before_runner_startup() {
        let (mut runtime, enclaves) = valid_empty_static_runtime();
        let source = FederateId::new("source");
        runtime.connections_mut().take_federate(&source).unwrap();

        let error = run_in_memory(
            runtime,
            enclaves,
            boomerang_runtime::Config::default().with_fast_forward(true),
        )
        .expect_err("runner must not recreate a missing lowered mailbox");

        assert!(matches!(
            error,
            StaticFederationRunnerError::Bridge { what }
                if what.contains("prebuilt runtime connection")
        ));
    }

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
        let runtime = StaticFederationRuntime::new(
            CompiledTopology::new(FederatedTopology::default()).unwrap(),
            BTreeMap::new(),
            crate::FederatedRuntimeConnections::new([], []).unwrap(),
        );
        let tcp = TcpStaticFederationConfig {
            bind_addr: SocketAddr::from(([203, 0, 113, 1], 1)),
        };

        let error = run_over_tcp(
            runtime,
            tinymap::TinyMap::new(),
            boomerang_runtime::Config::default(),
            tcp,
        )
        .expect_err("invalid runtime parts must fail before TCP bind");

        assert!(matches!(
            error,
            StaticFederationRunnerError::UnsupportedTopology { what }
                if what.contains("non-empty federation topology")
        ));
    }

    #[cfg(feature = "serde-json-codec")]
    #[test]
    fn tcp_runner_validates_configuration_before_binding() {
        let tcp = TcpStaticFederationConfig {
            bind_addr: SocketAddr::from(([203, 0, 113, 1], 1)),
        };

        let (runtime, enclaves) = valid_empty_static_runtime();
        let error = run_over_tcp(runtime, enclaves, boomerang_runtime::Config::default(), tcp)
            .expect_err("unsupported configuration must fail before TCP bind");

        assert!(matches!(
            error,
            StaticFederationRunnerError::UnsupportedConfiguration { .. }
        ));
    }
}
