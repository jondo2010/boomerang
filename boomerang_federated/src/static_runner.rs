//! Static federated runtime runners.

#[cfg(feature = "serde-json-codec")]
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
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
    FederateId, FederateProtocolClient, FederatedTopology, ProtocolFrame,
    RtiLogicalTimeCoordinator, RtiSessionEndpoint, SessionError, StaticRtiSession, TransportError,
};

/// Fully lowered federation-specific state required by a static runner.
#[doc(hidden)]
pub struct StaticFederationRuntime {
    /// Validated RTI topology and its precomputed coordination indexes.
    topology: CompiledTopology,
    /// Validated bidirectional placement of protocol federates and runtime enclaves.
    placement: FederateEnclaveMap,
    /// Prebuilt protocol mailboxes, routes, inbound handlers, and fault state.
    connections: crate::FederatedRuntimeConnections,
}

/// Error returned when one Enclave is assigned to more than one Federate.
#[derive(Debug, thiserror::Error)]
#[error(
    "ambiguous enclave-to-federate mapping: enclave {enclave_key:?} maps to both '{first}' and '{second}'"
)]
pub struct FederatePlacementError {
    /// Runtime enclave assigned to more than one federate.
    enclave_key: boomerang_runtime::EnclaveKey,
    /// First federate assigned to the enclave in deterministic identity order.
    first: FederateId,
    /// Second federate found with the same enclave assignment.
    second: FederateId,
}

struct FederateEnclaveMap {
    /// Runtime Enclaves assigned to each protocol Federate identity.
    by_federate: BTreeMap<FederateId, Vec<boomerang_runtime::EnclaveKey>>,
    /// Protocol federate identity assigned to each runtime enclave.
    by_enclave: tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, FederateId>,
}

impl FederateEnclaveMap {
    fn new(
        by_federate: BTreeMap<FederateId, Vec<boomerang_runtime::EnclaveKey>>,
    ) -> Result<Self, FederatePlacementError> {
        let mut by_enclave =
            tinymap::TinySecondaryMap::<boomerang_runtime::EnclaveKey, FederateId>::new();
        for (federate_id, enclave_keys) in &by_federate {
            for &enclave_key in enclave_keys {
                if let Some(first) = by_enclave.get(enclave_key) {
                    return Err(FederatePlacementError {
                        enclave_key,
                        first: first.clone(),
                        second: federate_id.clone(),
                    });
                }
                by_enclave.insert(enclave_key, federate_id.clone());
            }
        }
        Ok(Self {
            by_federate,
            by_enclave,
        })
    }

    fn len(&self) -> usize {
        self.by_federate.len()
    }

    fn federate_for_enclave(
        &self,
        enclave_key: boomerang_runtime::EnclaveKey,
    ) -> Option<&FederateId> {
        self.by_enclave.get(enclave_key)
    }
}

impl StaticFederationRuntime {
    /// Create static runner state from artifacts produced during lowering.
    ///
    /// Returns an error when more than one federate is assigned to the same runtime enclave.
    pub fn new(
        topology: CompiledTopology,
        federate_enclaves: BTreeMap<FederateId, Vec<boomerang_runtime::EnclaveKey>>,
    ) -> Result<Self, FederatePlacementError> {
        let connections = crate::FederatedRuntimeConnections::from_topology(topology.topology())
            .expect("compiled topology must produce valid runtime connections");
        Ok(Self {
            topology,
            placement: FederateEnclaveMap::new(federate_enclaves)?,
            connections,
        })
    }

    /// Return the validated topology and its precomputed coordination indexes.
    pub fn topology(&self) -> &CompiledTopology {
        &self.topology
    }

    /// Return the runtime enclave assigned to each protocol federate identity.
    pub fn federate_enclaves(&self) -> &BTreeMap<FederateId, Vec<boomerang_runtime::EnclaveKey>> {
        &self.placement.by_federate
    }

    /// Return the prebuilt runtime connections.
    pub fn connections(&self) -> &crate::FederatedRuntimeConnections {
        &self.connections
    }

    /// Return mutable access to the prebuilt runtime connections during lowering.
    pub fn connections_mut(&mut self) -> &mut crate::FederatedRuntimeConnections {
        &mut self.connections
    }

    /// Consume transient lowering state into independently owned runtime Federates.
    pub fn finalize(
        self,
        runtime: boomerang_runtime::RuntimeEnclaves,
    ) -> Result<crate::RuntimeFederation, crate::RuntimeFederationError> {
        crate::RuntimeFederation::from_lowered(
            self.topology,
            self.placement.by_federate,
            self.connections,
            runtime,
        )
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
    RuntimeEndpoint(#[from] crate::FederatedEndpointError),

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
    /// Validated bidirectional placement of protocol federates and runtime enclaves.
    placement: FederateEnclaveMap,
    /// Fully lowered runtime enclaves awaiting scheduler construction.
    enclaves: boomerang_runtime::RuntimeEnclaves,
}

struct ConnectedFederate {
    /// Connected protocol client used by the federate's logical-time coordinator.
    client: FederateProtocolClient,
    /// Validated inbound message routes owned by this federate.
    routes: Vec<FederateClientRoute>,
    /// Shared first-error state for protocol and runtime endpoint failures.
    faults: crate::FederatedFaultState,
}

/// Run a lowered static federation in memory using the real RTI session and federate clients.
pub fn run_in_memory(
    runtime: crate::RuntimeFederation,
    config: boomerang_runtime::Config,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let (prepared, connections) = prepare_static_federation(runtime)?;
    validate_static_runner_config(&config)?;
    let tokio_runtime = build_tokio_runtime(prepared.placement.len())?;
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
        &prepared.topology,
        connections,
        client_transports,
    )?;

    execute_connected_static_federation(prepared, config, &tokio_runtime, session_handle, clients)
}

/// Run a lowered static federation over TCP using the shared RTI session and federate clients.
#[cfg(feature = "serde-json-codec")]
pub fn run_over_tcp(
    runtime: crate::RuntimeFederation,
    config: boomerang_runtime::Config,
    tcp: TcpStaticFederationConfig,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let (prepared, connections) = prepare_static_federation(runtime)?;
    validate_static_runner_config(&config)?;
    let tokio_runtime = build_tokio_runtime(prepared.placement.len())?;
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
        &prepared.topology,
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
    runtime: crate::RuntimeFederation,
) -> Result<
    (
        PreparedStaticFederation,
        BTreeMap<FederateId, crate::FederateRuntimeBridge>,
    ),
    StaticFederationRunnerError,
> {
    validate_static_runner_runtime(&runtime)?;

    let (topology, federates) = runtime.into_parts();
    let mut by_federate = BTreeMap::new();
    let mut enclaves = boomerang_runtime::RuntimeEnclaves::new();
    let mut connections = BTreeMap::new();
    for (map_id, federate) in federates {
        let (id, runtime, bridge) = federate.into_parts();
        if map_id != id {
            return Err(bridge_error(format!(
                "runtime Federate map key '{map_id}' does not match owned id '{id}'"
            )));
        }
        let keys = runtime.keys().collect::<Vec<_>>();
        for (key, enclave) in runtime {
            enclaves.insert_at(key, enclave);
        }
        by_federate.insert(id.clone(), keys);
        connections.insert(id, bridge);
    }
    let placement =
        FederateEnclaveMap::new(by_federate).map_err(|error| bridge_error(error.to_string()))?;

    for (enclave_key, enclave) in enclaves.iter() {
        if placement.federate_for_enclave(enclave_key).is_none()
            && !enclave.env.reactions.is_empty()
        {
            return Err(unsupported_topology(format!(
                "static federation runner requires every non-empty runtime enclave to map to exactly one federate; enclave {enclave_key:?} is not mapped"
            )));
        }
    }

    Ok((
        PreparedStaticFederation {
            topology,
            placement,
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
    topology: &CompiledTopology,
    mut connections: BTreeMap<FederateId, crate::FederateRuntimeBridge>,
    mut transports: BTreeMap<FederateId, (S, R)>,
) -> Result<BTreeMap<FederateId, ConnectedFederate>, StaticFederationRunnerError>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError> + Send + 'static,
    R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
    R::Error: Into<TransportError> + Send + 'static,
{
    let mut connect_handles = Vec::new();
    for federate_id in &topology.topology().federates {
        let (sink, stream) = transports.remove(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing client transport for federate '{federate_id}'"
            ))
        })?;
        let federate_id_for_client = federate_id.clone();
        let neighbors = topology
            .neighbors_for(federate_id)
            .ok_or_else(|| {
                bridge_error(format!(
                    "missing compiled neighbor structure for federate '{federate_id}'"
                ))
            })?
            .clone();
        let connection = connections.remove(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing prebuilt runtime connection for federate '{federate_id}'"
            ))
        })?;
        let (mailbox, routes, faults) = connection.into_parts();
        connect_handles.push((
            federate_id.clone(),
            routes,
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
    for (federate_id, routes, faults, connect_handle) in connect_handles {
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
        placement,
        enclaves,
    } = prepared;

    // One scheduler acts as the Federate's RTI gateway. Other Enclaves retain their local
    // scheduler coordination and feed the gateway through in-process crosslinks. A blocking RTI
    // coordinator cannot be shared directly by multiple scheduler threads because one waiting
    // acquire would hold the coordinator while another Enclave needs to advance it.
    let gateway_enclaves = placement
        .by_federate
        .iter()
        .map(|(federate, keys)| {
            let gateway = keys
                .iter()
                .copied()
                .find(|key| {
                    enclaves
                        .get(*key)
                        .is_some_and(|enclave| !enclave.upstream_enclaves.is_empty())
                })
                .or_else(|| keys.first().copied())
                .expect("a finalized Federate owns at least one Enclave");
            (federate.clone(), gateway)
        })
        .collect::<BTreeMap<_, _>>();

    let mut barriers = BTreeMap::new();
    for federate_id in placement.by_federate.keys() {
        let connected = clients.remove(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing connected client for federate '{federate_id}'"
            ))
        })?;
        let barrier = RtiLogicalTimeCoordinator::new(
            federate_id.clone(),
            connected.client,
            connected.routes,
            connected.faults,
        )?;
        barriers.insert(
            federate_id.clone(),
            SharedRtiLogicalTimeCoordinator::new(barrier, 1),
        );
    }

    let mut envs = tinymap::TinySecondaryMap::new();
    let mut barrier_error = None;
    let mut handles: Vec<SchedulerThreadHandle> = Vec::new();
    for (enclave_key, enclave) in enclaves {
        let Some(federate_id) = placement.federate_for_enclave(enclave_key).cloned() else {
            if enclave.env.reactions.is_empty() {
                continue;
            }

            unreachable!("non-empty unmapped enclaves were rejected during preparation");
        };

        let barrier = barriers
            .get(&federate_id)
            .expect("barriers were built from federate placement")
            .clone();
        let is_gateway = gateway_enclaves[&federate_id] == enclave_key;

        if federate_has_no_initial_work(&enclave, topology.topology(), &federate_id) {
            let boomerang_runtime::Enclave { env, .. } = enclave;
            envs.insert(enclave_key, env);
            if is_gateway {
                if let Err(error) = barrier.finish_participant() {
                    barrier_error.get_or_insert_with(|| error.to_string());
                }
            }
            continue;
        }

        let config = config.clone();
        let thread_federate_id = federate_id.clone();
        let handle = match std::thread::Builder::new()
            .name(format!("federate-{federate_id}"))
            .spawn(move || {
                let stop_barrier = is_gateway.then(|| barrier.clone());
                let mut scheduler = if is_gateway {
                    boomerang_runtime::Scheduler::new_with_logical_time_coordinator(
                        enclave_key,
                        enclave,
                        config,
                        barrier,
                    )
                } else {
                    boomerang_runtime::Scheduler::new(enclave_key, enclave, config)
                };
                let scheduler_result = scheduler.try_event_loop();
                let env = scheduler.into_env();
                let stop_result = stop_barrier
                    .map(|barrier| barrier.finish_participant())
                    .unwrap_or(Ok(()));
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
                    let _ = barrier.force_stop();
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
        if let Err(error) = barrier.force_stop() {
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
struct SharedRtiLogicalTimeCoordinator {
    /// Shared RTI coordinator serialized across scheduler calls.
    inner: Arc<Mutex<RtiLogicalTimeCoordinator>>,
    remaining_participants: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
}

impl SharedRtiLogicalTimeCoordinator {
    fn new(barrier: RtiLogicalTimeCoordinator, participants: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(barrier)),
            remaining_participants: Arc::new(AtomicUsize::new(participants)),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    fn finish_participant(&self) -> Result<(), FederateClientError> {
        let previous = self.remaining_participants.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |remaining| remaining.checked_sub(1),
        );
        match previous {
            Ok(1) => self.force_stop(),
            Ok(_) | Err(0) => Ok(()),
            Err(_) => unreachable!("participant count can only fail at zero"),
        }
    }

    fn force_stop(&self) -> Result<(), FederateClientError> {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.inner
            .lock()
            .map_err(|_| FederateClientError::Protocol("RTI coordinator lock poisoned".into()))?
            .stop()
    }
}

impl boomerang_runtime::LogicalTimeCoordinator for SharedRtiLogicalTimeCoordinator {
    fn acquire(
        &mut self,
        tag: boomerang_runtime::Tag,
        event_rx: &boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent>,
    ) -> Result<boomerang_runtime::CoordinationOutcome, boomerang_runtime::CoordinationError> {
        let mut barrier = self.inner.lock().map_err(|_| {
            boomerang_runtime::CoordinationError::new("RTI coordinator lock poisoned")
        })?;
        boomerang_runtime::LogicalTimeCoordinator::acquire(&mut *barrier, tag, event_rx)
    }

    fn complete(
        &mut self,
        tag: boomerang_runtime::Tag,
    ) -> Result<(), boomerang_runtime::CoordinationError> {
        let mut barrier = self.inner.lock().map_err(|_| {
            boomerang_runtime::CoordinationError::new("RTI coordinator lock poisoned")
        })?;
        boomerang_runtime::LogicalTimeCoordinator::complete(&mut *barrier, tag)
    }
}

fn validate_static_runner_runtime(
    runtime: &crate::RuntimeFederation,
) -> Result<(), StaticFederationRunnerError> {
    let topology = runtime.topology().topology();
    if topology.federates.is_empty() || topology.edges.is_empty() {
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

    for federate_id in runtime.federates().keys() {
        if !federates.contains(federate_id) {
            return Err(bridge_error(format!(
                "federate '{federate_id}' has a runtime enclave but is missing from topology"
            )));
        }
    }

    for federate_id in &topology.federates {
        if !runtime.federates().contains_key(federate_id) {
            return Err(unsupported_topology(format!(
                "federate '{federate_id}' has no runtime enclave"
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
    for route in runtime
        .federates()
        .values()
        .flat_map(|federate| federate.bridge().routes())
    {
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

fn federate_has_no_initial_work(
    enclave: &boomerang_runtime::Enclave,
    topology: &FederatedTopology,
    federate_id: &FederateId,
) -> bool {
    enclave.env.reactions.is_empty()
        || (enclave.graph.startup_actions.is_empty()
            && enclave.upstream_enclaves.is_empty()
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

    #[test]
    fn static_runtime_rejects_two_federates_in_one_enclave() {
        let mut enclaves = tinymap::TinyMap::new();
        let enclave_key = enclaves.insert(boomerang_runtime::Enclave::default());
        let error = FederateEnclaveMap::new(BTreeMap::from([
            (FederateId::new("first"), vec![enclave_key]),
            (FederateId::new("second"), vec![enclave_key]),
        ]))
        .err()
        .expect("duplicate enclave placement must be rejected");

        assert_eq!(error.enclave_key, enclave_key);
        assert_eq!(error.first, FederateId::new("first"));
        assert_eq!(error.second, FederateId::new("second"));
    }

    fn valid_empty_static_runtime() -> (StaticFederationRuntime, boomerang_runtime::RuntimeEnclaves)
    {
        let source = FederateId::new("source");
        let sink = FederateId::new("sink");
        let endpoint = crate::EndpointId::new("source.out->sink.in");
        let mut enclaves = boomerang_runtime::RuntimeEnclaves::new();
        let source_enclave = enclaves.insert(boomerang_runtime::Enclave::default());
        let sink_enclave = enclaves.insert(boomerang_runtime::Enclave::default());

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
                (source.clone(), vec![source_enclave]),
                (sink.clone(), vec![sink_enclave]),
            ]),
        )
        .unwrap();
        (runtime, enclaves)
    }

    #[test]
    fn unsupported_configuration_rejects_wall_clock_static_federation() {
        let (runtime, enclaves) = valid_empty_static_runtime();
        let runtime = runtime.finalize(enclaves).unwrap();
        let error = run_in_memory(runtime, boomerang_runtime::Config::default())
            .expect_err("wall-clock static federation must be rejected");

        assert!(matches!(
            error,
            StaticFederationRunnerError::UnsupportedConfiguration { what }
                if what.contains("with_fast_forward(true)")
                    && what.contains("common physical start")
        ));

        let (runtime, enclaves) = valid_empty_static_runtime();
        let runtime = runtime.finalize(enclaves).unwrap();
        run_in_memory(
            runtime,
            boomerang_runtime::Config::default().with_fast_forward(true),
        )
        .expect("fast-forward static federation should pass configuration validation");
    }

    #[test]
    fn prebuilt_runtime_connections_are_required_before_runner_startup() {
        let (mut runtime, enclaves) = valid_empty_static_runtime();
        let source = FederateId::new("source");
        runtime.connections_mut().take_federate(&source).unwrap();

        let error = runtime
            .finalize(enclaves)
            .err()
            .expect("finalization must not recreate a missing lowered mailbox");

        assert!(matches!(
            error,
            crate::RuntimeFederationError::MissingBridge(id) if id == source
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
        )
        .unwrap();
        let tcp = TcpStaticFederationConfig {
            bind_addr: SocketAddr::from(([203, 0, 113, 1], 1)),
        };

        let runtime = runtime
            .finalize(boomerang_runtime::RuntimeEnclaves::new())
            .unwrap();
        let error = run_over_tcp(runtime, boomerang_runtime::Config::default(), tcp)
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
        let runtime = runtime.finalize(enclaves).unwrap();
        let error = run_over_tcp(runtime, boomerang_runtime::Config::default(), tcp)
            .expect_err("unsupported configuration must fail before TCP bind");

        assert!(matches!(
            error,
            StaticFederationRunnerError::UnsupportedConfiguration { .. }
        ));
    }
}
