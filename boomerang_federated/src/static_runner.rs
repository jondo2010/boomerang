//! Static federated runtime runners.

#[cfg(feature = "serde-json-codec")]
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::{
    collections::BTreeMap,
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
use crate::transport::run_tcp_static_rti_session;
use crate::{
    in_memory_transport_pair, FederateClientError, FederateClientRoute, FederateId,
    FederateProtocolClient, ProtocolFrame, RtiGraph, RtiLogicalTimeCoordinator, RtiSessionEndpoint,
    SessionError, StaticRtiSession, TransportError,
};

/// Fully lowered federation-specific state required by a static runner.
#[doc(hidden)]
pub struct StaticFederationRuntime {
    /// Final immutable RTI graph.
    graph: RtiGraph,
    /// Prebuilt protocol mailboxes, routes, inbound handlers, and fault state.
    connections: crate::FederatedRuntimeConnections,
}

impl StaticFederationRuntime {
    /// Create static runner state from artifacts produced during lowering.
    ///
    pub fn new(graph: RtiGraph, connections: crate::FederatedRuntimeConnections) -> Self {
        Self { graph, connections }
    }

    /// Return the final immutable RTI graph.
    pub fn graph(&self) -> &RtiGraph {
        &self.graph
    }

    /// Return the prebuilt runtime connections.
    pub fn connections(&self) -> &crate::FederatedRuntimeConnections {
        &self.connections
    }

    /// Return mutable access to the prebuilt runtime connections during lowering.
    pub fn connections_mut(&mut self) -> &mut crate::FederatedRuntimeConnections {
        &mut self.connections
    }

    /// Consume transient lowering state into the runtime Federation hierarchy.
    pub fn finalize(
        self,
        runtimes: BTreeMap<
            FederateId,
            tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
        >,
    ) -> Result<crate::RuntimeFederation, crate::RuntimeFederationError> {
        crate::RuntimeFederation::from_lowered(self.graph, runtimes, self.connections)
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

/// Final runtime environments grouped by Federate and owner-local Enclave key.
pub type FederationEnvs = BTreeMap<
    FederateId,
    tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Env>,
>;
type SessionHandle = tokio::task::JoinHandle<Result<(), SessionError>>;
type RuntimeFederateEnclaves = BTreeMap<
    FederateId,
    tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
>;
type ConnectedRuntimeFederates = (
    RuntimeFederateEnclaves,
    BTreeMap<FederateId, ConnectedFederate>,
);
type SchedulerThreadResult = (
    FederateId,
    boomerang_runtime::EnclaveKey,
    boomerang_runtime::Env,
    Result<(), boomerang_runtime::RuntimeError>,
    Result<(), FederateClientError>,
);
type SchedulerThreadHandle = std::thread::JoinHandle<SchedulerThreadResult>;

struct PreparedStaticFederation {
    /// Final immutable graph moved into the runner-owned RTI session.
    graph: RtiGraph,
    /// Complete runtime Federates kept independent from the RTI graph.
    federates: BTreeMap<FederateId, crate::RuntimeFederate>,
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
    let PreparedStaticFederation { graph, federates } = prepare_static_federation(runtime);
    validate_static_runner_config(&config)?;
    let tokio_runtime = build_tokio_runtime(federates.len())?;
    let mut session_endpoints = BTreeMap::new();
    let mut client_transports = BTreeMap::new();
    for federate_id in federates.keys() {
        let (client_transport, rti_transport) =
            in_memory_transport_pair::<ProtocolFrame, ProtocolFrame>();
        let (rti_sink, rti_stream) = rti_transport;
        session_endpoints.insert(
            federate_id.clone(),
            RtiSessionEndpoint::new(rti_sink, rti_stream),
        );
        client_transports.insert(federate_id.clone(), client_transport);
    }

    let session = StaticRtiSession::new(graph, session_endpoints);
    let session_handle = tokio_runtime.spawn(session.run());
    let (runtimes, clients) = connect_clients(&tokio_runtime, federates, client_transports)?;

    execute_connected_static_federation(runtimes, config, &tokio_runtime, session_handle, clients)
}

/// Run a lowered static federation over TCP using the shared RTI session and federate clients.
#[cfg(feature = "serde-json-codec")]
pub fn run_over_tcp(
    runtime: crate::RuntimeFederation,
    config: boomerang_runtime::Config,
    tcp: TcpStaticFederationConfig,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    let PreparedStaticFederation { graph, federates } = prepare_static_federation(runtime);
    validate_static_runner_config(&config)?;
    let tokio_runtime = build_tokio_runtime(federates.len())?;
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
    let session_handle = tokio_runtime.spawn(run_tcp_static_rti_session(listener, graph));

    let mut client_transports = BTreeMap::new();
    for federate_id in federates.keys() {
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

    let (runtimes, clients) = match connect_clients(&tokio_runtime, federates, client_transports) {
        Ok(connected) => connected,
        Err(error) => {
            session_handle.abort();
            return Err(error);
        }
    };

    execute_connected_static_federation(runtimes, config, &tokio_runtime, session_handle, clients)
}

fn prepare_static_federation(runtime: crate::RuntimeFederation) -> PreparedStaticFederation {
    let (graph, federates) = runtime.into_parts();
    PreparedStaticFederation { graph, federates }
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
    federates: BTreeMap<FederateId, crate::RuntimeFederate>,
    mut transports: BTreeMap<FederateId, (S, R)>,
) -> Result<ConnectedRuntimeFederates, StaticFederationRunnerError>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError> + Send + 'static,
    R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
    R::Error: Into<TransportError> + Send + 'static,
{
    let mut connect_handles = Vec::new();
    let mut runtimes = BTreeMap::new();
    for (map_id, federate) in federates {
        let (federate_id, enclaves, connection) = federate.into_parts();
        if map_id != federate_id {
            return Err(bridge_error(format!(
                "runtime Federate map key '{map_id}' does not match owned identity '{federate_id}'"
            )));
        }
        let (sink, stream) = transports.remove(&federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing client transport for federate '{federate_id}'"
            ))
        })?;
        let federate_id_for_client = federate_id.clone();
        let (mailbox, routes, faults) = connection.into_parts();
        runtimes.insert(federate_id.clone(), enclaves);
        connect_handles.push((
            federate_id.clone(),
            routes,
            faults,
            tokio_runtime.spawn(async move {
                FederateProtocolClient::connect_with_mailbox(
                    federate_id_for_client,
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

    Ok((runtimes, clients))
}

fn execute_connected_static_federation(
    runtimes: RuntimeFederateEnclaves,
    config: boomerang_runtime::Config,
    tokio_runtime: &tokio::runtime::Runtime,
    session_handle: SessionHandle,
    mut clients: BTreeMap<FederateId, ConnectedFederate>,
) -> Result<FederationEnvs, StaticFederationRunnerError> {
    // One scheduler acts as the Federate's RTI gateway. Other Enclaves retain their local
    // scheduler coordination and feed the gateway through in-process crosslinks. A blocking RTI
    // coordinator cannot be shared directly by multiple scheduler threads because one waiting
    // acquire would hold the coordinator while another Enclave needs to advance it.
    let gateway_enclaves = runtimes
        .iter()
        .map(|(federate, enclaves)| {
            let gateway = enclaves
                .iter()
                .find_map(|(key, enclave)| (!enclave.upstream_enclaves.is_empty()).then_some(key))
                .or_else(|| enclaves.keys().next())
                .expect("a finalized Federate owns at least one Enclave");
            (federate.clone(), gateway)
        })
        .collect::<BTreeMap<_, _>>();

    let mut barriers = BTreeMap::new();
    let mut has_inbound_routes = BTreeMap::new();
    for federate_id in runtimes.keys() {
        let connected = clients.remove(federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing connected client for federate '{federate_id}'"
            ))
        })?;
        has_inbound_routes.insert(federate_id.clone(), !connected.routes.is_empty());
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

    let mut envs = BTreeMap::<
        FederateId,
        tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Env>,
    >::new();
    let mut barrier_error = None;
    let mut handles: Vec<SchedulerThreadHandle> = Vec::new();
    for (federate_id, enclaves) in runtimes {
        let barrier = barriers
            .get(&federate_id)
            .expect("barriers were built from runtime Federates")
            .clone();
        for (enclave_key, enclave) in enclaves {
            let is_gateway = gateway_enclaves[&federate_id] == enclave_key;

            if federate_has_no_initial_work(&enclave, has_inbound_routes[&federate_id]) {
                let boomerang_runtime::Enclave { env, .. } = enclave;
                envs.entry(federate_id.clone())
                    .or_default()
                    .insert(enclave_key, env);
                if is_gateway {
                    if let Err(error) = barrier.finish_participant() {
                        barrier_error.get_or_insert_with(|| error.to_string());
                    }
                }
                continue;
            }

            let config = config.clone();
            let thread_federate_id = federate_id.clone();
            let scheduler_barrier = barrier.clone();
            let handle = match std::thread::Builder::new()
                .name(format!("federate-{federate_id}"))
                .spawn(move || {
                    let stop_barrier = is_gateway.then(|| scheduler_barrier.clone());
                    let mut scheduler = if is_gateway {
                        boomerang_runtime::Scheduler::new_with_logical_time_coordinator(
                            enclave_key,
                            enclave,
                            config,
                            scheduler_barrier,
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
    }

    let mut thread_panic = None;
    let mut scheduler_error = None;
    for handle in handles {
        match handle.join() {
            Ok((federate_id, enclave_key, env, scheduler_result, stop_result)) => {
                envs.entry(federate_id.clone())
                    .or_default()
                    .insert(enclave_key, env);
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
    has_inbound_routes: bool,
) -> bool {
    enclave.env.reactions.is_empty()
        || (enclave.graph.startup_actions.is_empty()
            && enclave.upstream_enclaves.is_empty()
            && !has_inbound_routes)
}

fn bridge_error(what: impl Into<String>) -> StaticFederationRunnerError {
    StaticFederationRunnerError::Bridge { what: what.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn federate_maps_allocate_independent_dense_keys() {
        let mut first =
            tinymap::TinyMap::<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>::new();
        let mut second =
            tinymap::TinyMap::<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>::new();

        let first_key = first.insert(boomerang_runtime::Enclave::default());
        let second_key = second.insert(boomerang_runtime::Enclave::default());

        assert_eq!(first_key, second_key);
    }

    fn valid_empty_static_runtime() -> (
        StaticFederationRuntime,
        BTreeMap<
            FederateId,
            tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
        >,
    ) {
        let source = FederateId::new("source");
        let sink = FederateId::new("sink");
        let endpoint = crate::EndpointId::new("source.out->sink.in");
        let mut source_enclaves = tinymap::TinyMap::new();
        source_enclaves.insert(boomerang_runtime::Enclave::default());
        let mut sink_enclaves = tinymap::TinyMap::new();
        sink_enclaves.insert(boomerang_runtime::Enclave::default());

        let graph = crate::rti::test_graph(
            [
                crate::rti::RtiFederateParts {
                    id: source.clone(),
                    transitive_incoming: Vec::new(),
                    affected_downstream: vec![sink.clone()],
                },
                crate::rti::RtiFederateParts {
                    id: sink.clone(),
                    transitive_incoming: vec![(source.clone(), crate::WireDelay::ZERO)],
                    affected_downstream: Vec::new(),
                },
            ],
            [crate::rti::RtiEndpointParts {
                id: endpoint.clone(),
                source: source.clone(),
                target: sink.clone(),
                delay: crate::WireDelay::ZERO,
            }],
        );
        let connections = crate::FederatedRuntimeConnections::new(
            [source.clone(), sink.clone()],
            [crate::FederateClientRoute::new(
                endpoint,
                source.clone(),
                sink.clone(),
            )],
        )
        .unwrap();
        let runtime = StaticFederationRuntime::new(graph, connections);
        (
            runtime,
            BTreeMap::from([(source, source_enclaves), (sink, sink_enclaves)]),
        )
    }

    #[test]
    fn preparation_preserves_dense_enclave_keys() {
        let (runtime, runtimes) = valid_empty_static_runtime();
        let expected_keys = runtimes
            .iter()
            .map(|(id, enclaves)| (id.clone(), enclaves.keys().collect::<Vec<_>>()))
            .collect::<BTreeMap<_, _>>();
        let runtime = runtime.finalize(runtimes).unwrap();

        let prepared = prepare_static_federation(runtime);

        let prepared_keys = prepared
            .federates
            .iter()
            .map(|(id, federate)| (id.clone(), federate.enclaves().keys().collect::<Vec<_>>()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(prepared_keys, expected_keys);
    }

    #[test]
    fn runtime_federate_is_complete_without_rti_graph_access() {
        let (runtime, runtimes) = valid_empty_static_runtime();
        let runtime = runtime.finalize(runtimes).unwrap();

        let (graph, mut federates) = runtime.into_parts();
        let source = FederateId::new("source");
        let runtime_federate = federates
            .remove(&source)
            .expect("lowering must produce the source runtime Federate");
        let (id, enclaves, bridge) = runtime_federate.into_parts();

        assert_eq!(id, source);
        assert!(!enclaves.is_empty());
        assert!(bridge.routes().all(|route| route.target == id));
        assert_eq!(graph.federate_ids().count(), 2);
    }

    #[test]
    fn runner_rejects_runtime_federate_stored_under_mismatched_identity() {
        let (runtime, runtimes) = valid_empty_static_runtime();
        let mut runtime = runtime.finalize(runtimes).unwrap();
        let source = FederateId::new("source");
        let wrong = FederateId::new("wrong");
        let source_runtime = runtime
            .federates_mut()
            .remove(&source)
            .expect("fixture must contain the source runtime Federate");
        runtime
            .federates_mut()
            .insert(wrong.clone(), source_runtime);

        let error = run_in_memory(
            runtime,
            boomerang_runtime::Config::default().with_fast_forward(true),
        )
        .expect_err("runner must reject a map key that differs from the owned Federate identity");

        assert!(matches!(
            error,
            StaticFederationRunnerError::Bridge { what }
                if what.contains("runtime Federate map key 'wrong'")
                    && what.contains("owned identity 'source'")
        ));
    }

    #[test]
    fn unsupported_configuration_rejects_wall_clock_static_federation() {
        let (runtime, runtimes) = valid_empty_static_runtime();
        let runtime = runtime.finalize(runtimes).unwrap();
        let error = run_in_memory(runtime, boomerang_runtime::Config::default())
            .expect_err("wall-clock static federation must be rejected");

        assert!(matches!(
            error,
            StaticFederationRunnerError::UnsupportedConfiguration { what }
                if what.contains("with_fast_forward(true)")
                    && what.contains("common physical start")
        ));

        let (runtime, runtimes) = valid_empty_static_runtime();
        let runtime = runtime.finalize(runtimes).unwrap();
        run_in_memory(
            runtime,
            boomerang_runtime::Config::default().with_fast_forward(true),
        )
        .expect("fast-forward static federation should pass configuration validation");
    }

    #[test]
    fn prebuilt_runtime_connections_are_required_before_runner_startup() {
        let (mut runtime, runtimes) = valid_empty_static_runtime();
        let source = FederateId::new("source");
        runtime.connections_mut().take_federate(&source).unwrap();

        let error = runtime
            .finalize(runtimes)
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
    fn tcp_runner_validates_configuration_before_binding() {
        let tcp = TcpStaticFederationConfig {
            bind_addr: SocketAddr::from(([203, 0, 113, 1], 1)),
        };

        let (runtime, runtimes) = valid_empty_static_runtime();
        let runtime = runtime.finalize(runtimes).unwrap();
        let error = run_over_tcp(runtime, boomerang_runtime::Config::default(), tcp)
            .expect_err("unsupported configuration must fail before TCP bind");

        assert!(matches!(
            error,
            StaticFederationRunnerError::UnsupportedConfiguration { .. }
        ));
    }
}
