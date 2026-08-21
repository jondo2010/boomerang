//! Static federated runtime runners.

#[cfg(feature = "serde-json-codec")]
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::{collections::BTreeMap, sync::mpsc};

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
enum SchedulerThreadResult {
    Completed {
        federate_id: FederateId,
        enclave_key: boomerang_runtime::EnclaveKey,
        env: boomerang_runtime::Env,
    },
    RuntimeError {
        federate_id: FederateId,
        source: boomerang_runtime::RuntimeError,
    },
    Panicked {
        what: String,
    },
}
type SchedulerThreadHandle = std::thread::JoinHandle<()>;

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
    use crate::federate_coordination::{FederateCoordinationLayout, FederateCoordinationService};

    let mut services = BTreeMap::new();
    let mut scheduler_inputs = Vec::new();
    for (federate_id, enclaves) in runtimes {
        let connected = clients.remove(&federate_id).ok_or_else(|| {
            bridge_error(format!(
                "missing connected client for federate '{federate_id}'"
            ))
        })?;
        let layout = FederateCoordinationLayout::new(enclaves.keys());
        let wakes = enclaves
            .iter()
            .map(|(key, enclave)| (key, enclave.create_send_context(key)))
            .collect();
        let coordinator = RtiLogicalTimeCoordinator::new(
            federate_id.clone(),
            connected.client,
            connected.routes,
            connected.faults,
        )?;
        let service = FederateCoordinationService::spawn(coordinator, layout, wakes)
            .map_err(|source| bridge_error(source.to_string()))?;
        for (enclave_key, enclave) in enclaves {
            scheduler_inputs.push((
                federate_id.clone(),
                enclave_key,
                enclave,
                service.participant(enclave_key),
            ));
        }
        services.insert(federate_id, service);
    }

    let (completion_tx, completion_rx) = mpsc::channel();
    let mut handles: Vec<SchedulerThreadHandle> = Vec::new();
    for (federate_id, enclave_key, enclave, participant) in scheduler_inputs {
        let config = config.clone();
        let completion_tx = completion_tx.clone();
        let thread_federate_id = federate_id.clone();
        let dispatch = tracing::enabled!(
            target: boomerang_runtime::trace::TRACE_TARGET,
            tracing::Level::TRACE
        )
        .then(|| tracing::dispatcher::get_default(Clone::clone));
        let handle = match std::thread::Builder::new()
            .name(format!("federate-{federate_id}-{enclave_key}"))
            .spawn(move || {
                let run = || {
                    let span = tracing::trace_span!(
                        target: boomerang_runtime::trace::TRACE_TARGET,
                        "scheduler_thread",
                        event = boomerang_runtime::trace::event::SCHEDULER_THREAD,
                        federate = %thread_federate_id,
                        enclave = %enclave_key,
                        state = "running",
                    );
                    let _entered = span.enter();
                    catch_scheduler_thread_body(|| {
                        let mut scheduler =
                            boomerang_runtime::Scheduler::new_with_logical_time_coordinator(
                                enclave_key,
                                enclave,
                                config,
                                participant,
                            );
                        match scheduler.try_event_loop() {
                            Ok(()) => SchedulerThreadResult::Completed {
                                federate_id: thread_federate_id,
                                enclave_key,
                                env: scheduler.into_env(),
                            },
                            Err(source) => SchedulerThreadResult::RuntimeError {
                                federate_id: thread_federate_id,
                                source,
                            },
                        }
                    })
                };
                let result = match dispatch {
                    Some(dispatch) => tracing::dispatcher::with_default(&dispatch, run),
                    None => run(),
                };
                let _ = completion_tx.send(result);
            }) {
            Ok(handle) => handle,
            Err(source) => {
                force_stop_services(&services);
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
    drop(completion_tx);

    let mut envs = BTreeMap::<
        FederateId,
        tinymap::TinySecondaryMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Env>,
    >::new();
    let mut first_error = None;
    for _ in 0..handles.len() {
        let result = match receive_scheduler_result(&completion_rx, || {
            force_stop_services(&services);
            session_handle.abort();
        }) {
            Ok(result) => result,
            Err(error) => {
                first_error.get_or_insert(error);
                break;
            }
        };
        match result {
            SchedulerThreadResult::Completed {
                federate_id,
                enclave_key,
                env,
            } => {
                envs.entry(federate_id)
                    .or_default()
                    .insert(enclave_key, env);
            }
            SchedulerThreadResult::RuntimeError {
                federate_id,
                source,
            } => {
                if first_error.is_none() {
                    first_error = Some(StaticFederationRunnerError::SchedulerRuntime {
                        federate_id,
                        source,
                    });
                    force_stop_services(&services);
                    session_handle.abort();
                }
            }
            SchedulerThreadResult::Panicked { what } => {
                if first_error.is_none() {
                    first_error = Some(StaticFederationRunnerError::SchedulerThreadPanic { what });
                    force_stop_services(&services);
                    session_handle.abort();
                }
            }
        }
    }

    for handle in handles {
        if let Err(payload) = handle.join() {
            first_error.get_or_insert_with(|| StaticFederationRunnerError::SchedulerThreadPanic {
                what: panic_payload_message(payload),
            });
        }
    }

    let mut service_error = None;
    let mut coordinators = Vec::new();
    for (_, service) in services {
        match service.join() {
            Ok(coordinator) => coordinators.push(coordinator),
            Err(error) => {
                service_error.get_or_insert(error);
            }
        }
    }

    if let Some(error) = first_error {
        return Err(error);
    }
    if let Some(error) = service_error {
        return Err(bridge_error(error));
    }

    let session_result = tokio_runtime
        .block_on(session_handle)
        .map_err(|source| StaticFederationRunnerError::SessionTask { source })?;
    session_result?;
    drop(coordinators);
    Ok(envs)
}

fn catch_scheduler_thread_body(
    body: impl FnOnce() -> SchedulerThreadResult,
) -> SchedulerThreadResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(payload) => SchedulerThreadResult::Panicked {
            what: panic_payload_message(payload),
        },
    }
}

fn receive_scheduler_result(
    completion_rx: &mpsc::Receiver<SchedulerThreadResult>,
    cleanup: impl FnOnce(),
) -> Result<SchedulerThreadResult, StaticFederationRunnerError> {
    completion_rx.recv().map_err(|_| {
        cleanup();
        bridge_error("scheduler completion channel closed before all schedulers terminated")
    })
}

fn force_stop_services(
    services: &BTreeMap<FederateId, crate::federate_coordination::FederateCoordinationService>,
) {
    for service in services.values() {
        let _ = service.force_stop();
    }
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|value| (*value).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "scheduler thread panicked with a non-string payload".into())
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

fn bridge_error(what: impl Into<String>) -> StaticFederationRunnerError {
    StaticFederationRunnerError::Bridge { what: what.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc, Mutex,
        },
        time::Duration as StdDuration,
    };

    use boomerang_runtime::CommonContext;
    use futures_util::{FutureExt, SinkExt, StreamExt};

    fn scheduled_enclave(
        name: &str,
        tag: boomerang_runtime::Tag,
        reaction: impl Fn(boomerang_runtime::Tag) + Send + Sync + 'static,
    ) -> boomerang_runtime::Enclave {
        let mut enclave = boomerang_runtime::Enclave::default();
        let reactor =
            enclave.insert_reactor(boomerang_runtime::Reactor::new(name, ()).boxed(), None);
        let scope = enclave.root_scope(reactor);
        let action = enclave.insert_action(|key| {
            boomerang_runtime::Action::<()>::new("scheduled", key, None, true).boxed()
        });
        enclave.insert_action_scope(action, scope);
        let reaction = enclave.insert_reaction(
            boomerang_runtime::Reaction::new(
                "scheduled",
                boomerang_runtime::reaction_closure!(ctx, _reactor, _refs => {
                    reaction(ctx.get_tag());
                }),
                None,
            ),
            reactor,
            std::iter::empty::<boomerang_runtime::PortKey>(),
            std::iter::empty::<boomerang_runtime::PortKey>(),
            std::iter::once(action),
            scope,
            None,
        );
        enclave.insert_action_trigger(action, (boomerang_runtime::Level::from(0), reaction));
        enclave.insert_startup_action(action, tag);
        enclave
    }

    async fn recv_federate_frame(
        transport: &mut crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>,
    ) -> crate::FederateToRti {
        match transport.1.next().await.unwrap().unwrap() {
            ProtocolFrame::FederateToRti(message) => message,
            frame => panic!("expected federate-to-RTI frame, got {frame:?}"),
        }
    }

    async fn send_federate_frame(
        transport: &mut crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        message: crate::RtiToFederate,
    ) {
        transport
            .0
            .send(ProtocolFrame::RtiToFederate(message))
            .await
            .unwrap();
    }

    fn execute_with_fake_rti<F, Fut>(
        runtimes: RuntimeFederateEnclaves,
        routes: Vec<FederateClientRoute>,
        fake_rti: F,
    ) -> Result<FederationEnvs, StaticFederationRunnerError>
    where
        F: FnOnce(crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>) -> Fut,
        Fut: std::future::Future<Output = Result<(), SessionError>> + Send + 'static,
    {
        execute_with_fake_rti_config(
            runtimes,
            routes,
            boomerang_runtime::Config::default().with_fast_forward(true),
            fake_rti,
        )
    }

    fn execute_with_fake_rti_config<F, Fut>(
        runtimes: RuntimeFederateEnclaves,
        routes: Vec<FederateClientRoute>,
        config: boomerang_runtime::Config,
        fake_rti: F,
    ) -> Result<FederationEnvs, StaticFederationRunnerError>
    where
        F: FnOnce(crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>) -> Fut,
        Fut: std::future::Future<Output = Result<(), SessionError>> + Send + 'static,
    {
        let federate = FederateId::new("federate");
        let tokio_runtime = build_tokio_runtime(1).unwrap();
        let (client_transport, rti_transport) = in_memory_transport_pair();
        let session_handle = tokio_runtime.spawn(fake_rti(rti_transport));
        let (sink, stream) = client_transport;
        let client = tokio_runtime
            .block_on(FederateProtocolClient::connect(
                federate.clone(),
                sink,
                stream,
            ))
            .unwrap();
        let clients = BTreeMap::from([(
            federate,
            ConnectedFederate {
                client,
                routes,
                faults: crate::FederatedFaultState::default(),
            },
        )]);

        execute_connected_static_federation(
            runtimes,
            config,
            &tokio_runtime,
            session_handle,
            clients,
        )
    }

    fn run_with_wall_timeout<T: Send + 'static>(
        label: &'static str,
        timeout: StdDuration,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let _ = tx.send(result);
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(value)) => value,
            Ok(Err(payload)) => std::panic::resume_unwind(payload),
            Err(_) => panic!("{label} timed out"),
        }
    }

    #[test]
    fn all_enclaves_participate_in_federate_rti_frontier() {
        let federate = FederateId::new("federate");
        let source = FederateId::new("source");
        let endpoint = crate::EndpointId::new("source.out->federate.in");
        let withheld_tag =
            boomerang_runtime::Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
        let later_tag =
            boomerang_runtime::Tag::new(boomerang_runtime::Duration::milliseconds(20), 0);
        let observations = Arc::new(Mutex::new(Vec::new()));
        let advanced_early = Arc::new(AtomicBool::new(false));

        let first_observations = Arc::clone(&observations);
        let first = scheduled_enclave("old-gateway", withheld_tag, move |tag| {
            first_observations.lock().unwrap().push(("first", tag));
        });
        let first_wake = first.create_send_context(boomerang_runtime::EnclaveKey::from(0));

        let (later_release_tx, later_release_rx) = mpsc::channel();
        let later_release_rx = Arc::new(Mutex::new(later_release_rx));
        let second_observations = Arc::clone(&observations);
        let later_release = Arc::clone(&later_release_rx);
        let mut second = scheduled_enclave("routed", later_tag, move |tag| {
            second_observations.lock().unwrap().push(("later", tag));
            later_release.lock().unwrap().recv().unwrap();
        });
        let second_key = boomerang_runtime::EnclaveKey::from(1);
        let inbound_action = second.insert_action(|key| {
            boomerang_runtime::Action::<u32>::new("inbound", key, None, true).boxed()
        });
        let second_reactor = second.env.reactors.keys().next().unwrap();
        let second_scope = second.root_scope(second_reactor);
        second.insert_action_scope(inbound_action, second_scope);
        let message_observations = Arc::clone(&observations);
        let inbound_reaction = second.insert_reaction(
            boomerang_runtime::Reaction::new(
                "message",
                boomerang_runtime::reaction_closure!(ctx, _reactor, _refs => {
                    message_observations
                        .lock()
                        .unwrap()
                        .push(("message", ctx.get_tag()));
                }),
                None,
            ),
            second_reactor,
            std::iter::empty::<boomerang_runtime::PortKey>(),
            std::iter::empty::<boomerang_runtime::PortKey>(),
            std::iter::once(inbound_action),
            second_scope,
            None,
        );
        second.insert_action_trigger(
            inbound_action,
            (boomerang_runtime::Level::from(0), inbound_reaction),
        );
        let inbound = crate::FederatedInboundEndpoint::new(
            second.create_send_context(second_key),
            second.create_async_action_ref::<u32>(inbound_action),
            Box::new(|payload: &[u8]| {
                std::str::from_utf8(payload)
                    .map_err(|error| crate::CodecError::message(error.to_string()))?
                    .parse::<u32>()
                    .map_err(|error| crate::CodecError::message(error.to_string()))
            }),
        )
        .unwrap();
        let mut route =
            FederateClientRoute::new(endpoint.clone(), source.clone(), federate.clone());
        route.bind_inbound(inbound);

        let mut enclaves = tinymap::TinyMap::new();
        assert_eq!(
            enclaves.insert(first),
            boomerang_runtime::EnclaveKey::from(0)
        );
        assert_eq!(enclaves.insert(second), second_key);
        let runtimes = BTreeMap::from([(federate.clone(), enclaves)]);
        let fake_observations = Arc::clone(&observations);
        let fake_advanced_early = Arc::clone(&advanced_early);
        let result =
            execute_with_fake_rti(runtimes, vec![route], move |mut transport| async move {
                assert!(matches!(
                    recv_federate_frame(&mut transport).await,
                    crate::FederateToRti::Hello { federate_id } if federate_id == federate
                ));
                send_federate_frame(
                    &mut transport,
                    crate::RtiToFederate::Start {
                        start_unix_epoch_ns: 0,
                    },
                )
                .await;
                assert!(matches!(
                    recv_federate_frame(&mut transport).await,
                    crate::FederateToRti::Net { tag, .. }
                        if tag == crate::WireTag::try_from(withheld_tag).unwrap()
                ));

                tokio::task::spawn_blocking(|| std::thread::sleep(StdDuration::from_millis(50)))
                    .await
                    .unwrap();
                let bypassed = fake_observations
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(label, _)| *label == "later");
                fake_advanced_early.store(bypassed, Ordering::Release);
                send_federate_frame(
                    &mut transport,
                    crate::RtiToFederate::Msg {
                        source,
                        endpoint,
                        tag: crate::WireTag::try_from(withheld_tag).unwrap(),
                        payload: b"7".to_vec(),
                    },
                )
                .await;
                if bypassed {
                    let _ = first_wake.schedule_external(
                        boomerang_runtime::AsyncEvent::TagReleaseProvisional {
                            enclave: second_key,
                            tag: withheld_tag,
                        },
                    );
                }
                tokio::task::spawn_blocking(|| std::thread::sleep(StdDuration::from_millis(20)))
                    .await
                    .unwrap();
                send_federate_frame(
                    &mut transport,
                    crate::RtiToFederate::Tag {
                        tag: crate::WireTag::try_from(withheld_tag).unwrap(),
                    },
                )
                .await;
                let _ = later_release_tx.send(());

                loop {
                    match recv_federate_frame(&mut transport).await {
                        crate::FederateToRti::Net { tag, .. } => {
                            send_federate_frame(&mut transport, crate::RtiToFederate::Tag { tag })
                                .await;
                        }
                        crate::FederateToRti::Ltc { .. } => {}
                        crate::FederateToRti::Stop { .. } => break,
                        message => panic!("unexpected federate frame: {message:?}"),
                    }
                }
                Ok(())
            });

        assert!(result.is_ok(), "runner failed: {result:?}");
        assert!(
            !advanced_early.load(Ordering::Acquire),
            "non-gateway Enclave advanced beyond the withheld federate frontier"
        );
        let observations = observations.lock().unwrap();
        let message = observations
            .iter()
            .position(|(label, _)| *label == "message")
            .unwrap();
        let later = observations
            .iter()
            .position(|(label, _)| *label == "later")
            .unwrap();
        assert!(
            message < later,
            "message must be observed before later-tag work: {observations:?}"
        );
    }

    #[test]
    fn same_federate_local_dependency_does_not_deadlock() {
        run_with_wall_timeout(
            "same-Federate local dependency",
            StdDuration::from_secs(2),
            || {
                let federate = FederateId::new("federate");
                let later =
                    boomerang_runtime::Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
                let observations = Arc::new(Mutex::new(Vec::new()));
                let first_observations = Arc::clone(&observations);
                let first = scheduled_enclave("first", boomerang_runtime::Tag::ZERO, move |tag| {
                    first_observations.lock().unwrap().push(("first", tag));
                });
                let second_observations = Arc::clone(&observations);
                let second = scheduled_enclave("second", later, move |tag| {
                    second_observations.lock().unwrap().push(("second", tag));
                });
                let mut enclaves = tinymap::TinyMap::new();
                let first_key = enclaves.insert(first);
                let second_key = enclaves.insert(second);
                boomerang_runtime::crosslink_enclaves(&mut enclaves, first_key, second_key, None);
                let first_wake = enclaves[first_key].create_send_context(first_key);
                let second_wake = enclaves[second_key].create_send_context(second_key);
                let runtimes = BTreeMap::from([(federate.clone(), enclaves)]);
                let frames = Arc::new(Mutex::new(Vec::new()));
                let fake_frames = Arc::clone(&frames);
                let fake_observations = Arc::clone(&observations);

                let result = execute_with_fake_rti_config(
                    runtimes,
                    Vec::new(),
                    boomerang_runtime::Config::default()
                        .with_fast_forward(true)
                        .with_keep_alive(true),
                    move |mut transport| async move {
                        assert!(matches!(
                            recv_federate_frame(&mut transport).await,
                            crate::FederateToRti::Hello { federate_id } if federate_id == federate
                        ));
                        send_federate_frame(
                            &mut transport,
                            crate::RtiToFederate::Start {
                                start_unix_epoch_ns: 0,
                            },
                        )
                        .await;
                        let wire_later = crate::WireTag::try_from(later).unwrap();
                        let mut pending_later = None;
                        let mut completed_zero = false;
                        let mut completed_later = false;
                        loop {
                            let frame = recv_federate_frame(&mut transport).await;
                            fake_frames.lock().unwrap().push(frame.clone());
                            match frame {
                                crate::FederateToRti::Net {
                                    tag: crate::WireTag::ZERO,
                                    ..
                                } => {
                                    send_federate_frame(
                                        &mut transport,
                                        crate::RtiToFederate::Tag {
                                            tag: crate::WireTag::ZERO,
                                        },
                                    )
                                    .await;
                                }
                                crate::FederateToRti::Net { tag, .. }
                                    if tag == wire_later && !completed_zero =>
                                {
                                    pending_later = Some(tag);
                                }
                                crate::FederateToRti::Net { tag, .. }
                                    if tag != crate::WireTag::FOREVER =>
                                {
                                    send_federate_frame(
                                        &mut transport,
                                        crate::RtiToFederate::Tag { tag },
                                    )
                                    .await;
                                }
                                crate::FederateToRti::Net {
                                    tag: crate::WireTag::FOREVER,
                                    ..
                                } => {
                                    assert_eq!(
                                        fake_observations.lock().unwrap().len(),
                                        2,
                                        "federate stopped before both local participants ran"
                                    );
                                }
                                crate::FederateToRti::Ltc {
                                    tag: crate::WireTag::ZERO,
                                    ..
                                } => {
                                    completed_zero = true;
                                    if let Some(tag) = pending_later.take() {
                                        send_federate_frame(
                                            &mut transport,
                                            crate::RtiToFederate::Tag { tag },
                                        )
                                        .await;
                                    }
                                }
                                crate::FederateToRti::Ltc { tag, .. } if tag == wire_later => {
                                    completed_later = true;
                                    assert!(first_wake.schedule_external(
                                        boomerang_runtime::AsyncEvent::Shutdown {
                                            delay: boomerang_runtime::Duration::ZERO,
                                        }
                                    ));
                                    assert!(second_wake.schedule_external(
                                        boomerang_runtime::AsyncEvent::Shutdown {
                                            delay: boomerang_runtime::Duration::ZERO,
                                        }
                                    ));
                                }
                                crate::FederateToRti::Ltc { .. } => {}
                                crate::FederateToRti::Stop { .. } => break,
                                frame => panic!("unexpected frame: {frame:?}"),
                            }
                        }
                        assert!(completed_zero, "zero round completion was not reported");
                        assert!(completed_later, "later round completion was not reported");
                        Ok(())
                    },
                );

                assert!(result.is_ok(), "runner failed: {result:?}");
                assert_eq!(
                    *observations.lock().unwrap(),
                    vec![("first", boomerang_runtime::Tag::ZERO), ("second", later)]
                );
                let frames = frames.lock().unwrap();
                assert_eq!(
                    *frames,
                    vec![
                        crate::FederateToRti::Net {
                            federate_id: FederateId::new("federate"),
                            tag: crate::WireTag::ZERO,
                        },
                        crate::FederateToRti::Ltc {
                            federate_id: FederateId::new("federate"),
                            tag: crate::WireTag::ZERO,
                        },
                        crate::FederateToRti::Net {
                            federate_id: FederateId::new("federate"),
                            tag: crate::WireTag::try_from(later).unwrap(),
                        },
                        crate::FederateToRti::Ltc {
                            federate_id: FederateId::new("federate"),
                            tag: crate::WireTag::try_from(later).unwrap(),
                        },
                        crate::FederateToRti::Net {
                            federate_id: FederateId::new("federate"),
                            tag: crate::WireTag::try_from(
                                later.delay(boomerang_runtime::Duration::ZERO)
                            )
                            .unwrap(),
                        },
                        crate::FederateToRti::Ltc {
                            federate_id: FederateId::new("federate"),
                            tag: crate::WireTag::try_from(
                                later.delay(boomerang_runtime::Duration::ZERO)
                            )
                            .unwrap(),
                        },
                        crate::FederateToRti::Net {
                            federate_id: FederateId::new("federate"),
                            tag: crate::WireTag::FOREVER,
                        },
                        crate::FederateToRti::Stop {
                            federate_id: FederateId::new("federate"),
                        },
                    ]
                );
                assert_eq!(
                    frames
                        .iter()
                        .filter(|frame| matches!(frame, crate::FederateToRti::Stop { .. }))
                        .count(),
                    1
                );
                assert_eq!(
                    frames
                        .iter()
                        .filter(|frame| matches!(
                            frame,
                            crate::FederateToRti::Net {
                                tag: crate::WireTag::FOREVER,
                                ..
                            }
                        ))
                        .count(),
                    1
                );
            },
        );
    }

    #[test]
    fn same_federate_positive_delay_dependency_does_not_deadlock() {
        run_with_wall_timeout(
            "same-Federate positive-delay dependency",
            StdDuration::from_secs(2),
            || {
                let federate = FederateId::new("federate");
                let later =
                    boomerang_runtime::Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
                let observations = Arc::new(Mutex::new(Vec::new()));
                let first_observations = Arc::clone(&observations);
                let first = scheduled_enclave("first", boomerang_runtime::Tag::ZERO, move |tag| {
                    first_observations.lock().unwrap().push(("first", tag));
                });
                let second_observations = Arc::clone(&observations);
                let second = scheduled_enclave("second", later, move |tag| {
                    second_observations.lock().unwrap().push(("second", tag));
                });
                let mut enclaves = tinymap::TinyMap::new();
                let first_key = enclaves.insert(first);
                let second_key = enclaves.insert(second);
                boomerang_runtime::crosslink_enclaves(
                    &mut enclaves,
                    first_key,
                    second_key,
                    Some(boomerang_runtime::Duration::milliseconds(5)),
                );
                let runtimes = BTreeMap::from([(federate.clone(), enclaves)]);
                let result =
                    execute_with_fake_rti(runtimes, Vec::new(), move |mut transport| async move {
                        assert!(matches!(
                            recv_federate_frame(&mut transport).await,
                            crate::FederateToRti::Hello { federate_id } if federate_id == federate
                        ));
                        send_federate_frame(
                            &mut transport,
                            crate::RtiToFederate::Start {
                                start_unix_epoch_ns: 0,
                            },
                        )
                        .await;
                        loop {
                            match recv_federate_frame(&mut transport).await {
                                crate::FederateToRti::Net { tag, .. }
                                    if tag != crate::WireTag::FOREVER =>
                                {
                                    send_federate_frame(
                                        &mut transport,
                                        crate::RtiToFederate::Tag { tag },
                                    )
                                    .await;
                                }
                                crate::FederateToRti::Net {
                                    tag: crate::WireTag::FOREVER,
                                    ..
                                }
                                | crate::FederateToRti::Ltc { .. } => {}
                                crate::FederateToRti::Stop { .. } => break,
                                frame => panic!("unexpected frame: {frame:?}"),
                            }
                        }
                        Ok(())
                    });
                assert!(result.is_ok(), "runner failed: {result:?}");
                assert_eq!(
                    *observations.lock().unwrap(),
                    vec![("first", boomerang_runtime::Tag::ZERO), ("second", later)]
                );
            },
        );
    }

    #[test]
    fn earlier_local_event_revises_published_federate_candidate() {
        run_with_wall_timeout(
            "earlier local candidate revision",
            StdDuration::from_secs(2),
            || {
                let federate = FederateId::new("federate");
                let later =
                    boomerang_runtime::Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
                let observations = Arc::new(Mutex::new(Vec::new()));
                let reaction_observations = Arc::clone(&observations);
                let enclave = scheduled_enclave("revised", later, move |tag| {
                    reaction_observations.lock().unwrap().push(tag);
                });
                let key = boomerang_runtime::EnclaveKey::from(0);
                let wake = enclave.create_send_context(key);
                let action = enclave.graph.startup_actions.first().unwrap().0;
                let mut enclaves = tinymap::TinyMap::new();
                assert_eq!(enclaves.insert(enclave), key);
                let runtimes = BTreeMap::from([(federate.clone(), enclaves)]);
                let frames = Arc::new(Mutex::new(Vec::new()));
                let fake_frames = Arc::clone(&frames);

                let result =
                    execute_with_fake_rti(runtimes, Vec::new(), move |mut transport| async move {
                        assert!(matches!(
                            recv_federate_frame(&mut transport).await,
                            crate::FederateToRti::Hello { federate_id } if federate_id == federate
                        ));
                        send_federate_frame(
                            &mut transport,
                            crate::RtiToFederate::Start {
                                start_unix_epoch_ns: 0,
                            },
                        )
                        .await;
                        let first = recv_federate_frame(&mut transport).await;
                        fake_frames.lock().unwrap().push(first.clone());
                        assert!(matches!(
                            first,
                            crate::FederateToRti::Net { tag, .. }
                                if tag == crate::WireTag::try_from(later).unwrap()
                        ));
                        assert!(
                            wake.schedule_external(boomerang_runtime::AsyncEvent::Logical {
                                tag: boomerang_runtime::Tag::ZERO,
                                key: action,
                                value: Box::new(()),
                            })
                        );
                        loop {
                            let frame = recv_federate_frame(&mut transport).await;
                            fake_frames.lock().unwrap().push(frame.clone());
                            match frame {
                                crate::FederateToRti::Net { tag, .. }
                                    if tag != crate::WireTag::FOREVER =>
                                {
                                    send_federate_frame(
                                        &mut transport,
                                        crate::RtiToFederate::Tag { tag },
                                    )
                                    .await;
                                }
                                crate::FederateToRti::Net {
                                    tag: crate::WireTag::FOREVER,
                                    ..
                                }
                                | crate::FederateToRti::Ltc { .. } => {}
                                crate::FederateToRti::Stop { .. } => break,
                                frame => panic!("unexpected frame: {frame:?}"),
                            }
                        }
                        Ok(())
                    });

                assert!(result.is_ok(), "runner failed: {result:?}");
                assert_eq!(
                    *observations.lock().unwrap(),
                    vec![boomerang_runtime::Tag::ZERO, later]
                );
                let frames = frames.lock().unwrap();
                assert!(matches!(
                    frames.as_slice(),
                    [
                        crate::FederateToRti::Net { tag: first, .. },
                        crate::FederateToRti::Net { tag: revised, .. },
                        ..
                    ] if *first == crate::WireTag::try_from(later).unwrap()
                        && *revised == crate::WireTag::ZERO
                ));
            },
        );
    }

    #[test]
    fn no_startup_upstream_participant_is_retained_for_later_local_work() {
        run_with_wall_timeout(
            "no-startup upstream participant",
            StdDuration::from_secs(2),
            || {
                let federate = FederateId::new("federate");
                let later =
                    boomerang_runtime::Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
                let observations = Arc::new(Mutex::new(Vec::new()));
                let downstream_observations = Arc::clone(&observations);
                let upstream = boomerang_runtime::Enclave::default();
                let downstream = scheduled_enclave("downstream", later, move |tag| {
                    downstream_observations.lock().unwrap().push(tag);
                });
                let mut enclaves = tinymap::TinyMap::new();
                let upstream_key = enclaves.insert(upstream);
                let downstream_key = enclaves.insert(downstream);
                boomerang_runtime::crosslink_enclaves(
                    &mut enclaves,
                    upstream_key,
                    downstream_key,
                    None,
                );
                let upstream_wake = enclaves[upstream_key].create_send_context(upstream_key);
                let downstream_wake = enclaves[downstream_key].create_send_context(downstream_key);
                let runtimes = BTreeMap::from([(federate.clone(), enclaves)]);
                let frames = Arc::new(Mutex::new(Vec::new()));
                let fake_frames = Arc::clone(&frames);

                let result = execute_with_fake_rti_config(
                    runtimes,
                    Vec::new(),
                    boomerang_runtime::Config::default()
                        .with_fast_forward(true)
                        .with_keep_alive(true),
                    move |mut transport| async move {
                        assert!(matches!(
                            recv_federate_frame(&mut transport).await,
                            crate::FederateToRti::Hello { federate_id } if federate_id == federate
                        ));
                        send_federate_frame(
                            &mut transport,
                            crate::RtiToFederate::Start {
                                start_unix_epoch_ns: 0,
                            },
                        )
                        .await;
                        let mut shutdown_sent = false;
                        loop {
                            let frame = recv_federate_frame(&mut transport).await;
                            fake_frames.lock().unwrap().push(frame.clone());
                            match frame {
                                crate::FederateToRti::Net { tag, .. }
                                    if tag != crate::WireTag::FOREVER =>
                                {
                                    send_federate_frame(
                                        &mut transport,
                                        crate::RtiToFederate::Tag { tag },
                                    )
                                    .await;
                                }
                                crate::FederateToRti::Ltc { tag, .. }
                                    if tag == crate::WireTag::try_from(later).unwrap()
                                        && !shutdown_sent =>
                                {
                                    shutdown_sent = true;
                                    assert!(upstream_wake.schedule_external(
                                        boomerang_runtime::AsyncEvent::Shutdown {
                                            delay: boomerang_runtime::Duration::ZERO,
                                        }
                                    ));
                                    assert!(downstream_wake.schedule_external(
                                        boomerang_runtime::AsyncEvent::Shutdown {
                                            delay: boomerang_runtime::Duration::ZERO,
                                        }
                                    ));
                                }
                                crate::FederateToRti::Ltc { .. }
                                | crate::FederateToRti::Net {
                                    tag: crate::WireTag::FOREVER,
                                    ..
                                } => {}
                                crate::FederateToRti::Stop { .. } => break,
                                frame => panic!("unexpected frame: {frame:?}"),
                            }
                        }
                        assert!(
                            shutdown_sent,
                            "later local work never reached a fixed point"
                        );
                        Ok(())
                    },
                );

                assert!(result.is_ok(), "runner failed: {result:?}");
                assert_eq!(*observations.lock().unwrap(), vec![later]);
                assert!(matches!(
                    frames.lock().unwrap().first(),
                    Some(crate::FederateToRti::Net { tag, .. })
                        if *tag == crate::WireTag::try_from(later).unwrap()
                ));
            },
        );
    }

    #[test]
    fn initially_idle_downstream_is_rewoken_after_later_local_work() {
        run_with_wall_timeout(
            "initially idle downstream re-wake",
            StdDuration::from_secs(2),
            || {
                let federate = FederateId::new("federate");
                let observations = Arc::new(Mutex::new(Vec::new()));
                let downstream_key = boomerang_runtime::EnclaveKey::from(1);
                let mut downstream = boomerang_runtime::Enclave::default();
                let downstream_reactor = downstream.insert_reactor(
                    boomerang_runtime::Reactor::new("downstream", ()).boxed(),
                    None,
                );
                let downstream_scope = downstream.root_scope(downstream_reactor);
                let downstream_action = downstream.insert_action(|key| {
                    boomerang_runtime::Action::<()>::new("local", key, None, true).boxed()
                });
                downstream.insert_action_scope(downstream_action, downstream_scope);
                let downstream_observations = Arc::clone(&observations);
                let downstream_reaction = downstream.insert_reaction(
                    boomerang_runtime::Reaction::new(
                        "local",
                        boomerang_runtime::reaction_closure!(ctx, _reactor, _refs => {
                            downstream_observations
                                .lock()
                                .unwrap()
                                .push(("downstream", ctx.get_tag()));
                        }),
                        None,
                    ),
                    downstream_reactor,
                    std::iter::empty::<boomerang_runtime::PortKey>(),
                    std::iter::empty::<boomerang_runtime::PortKey>(),
                    std::iter::once(downstream_action),
                    downstream_scope,
                    None,
                );
                downstream.insert_action_trigger(
                    downstream_action,
                    (boomerang_runtime::Level::from(0), downstream_reaction),
                );
                let downstream_wake = downstream.create_send_context(downstream_key);
                let local_delivery = downstream_wake.clone();
                let source_observations = Arc::clone(&observations);
                let source =
                    scheduled_enclave("source", boomerang_runtime::Tag::ZERO, move |tag| {
                        source_observations.lock().unwrap().push(("source", tag));
                        std::thread::sleep(StdDuration::from_millis(20));
                        assert!(local_delivery.schedule_external(
                            boomerang_runtime::AsyncEvent::Logical {
                                tag,
                                key: downstream_action,
                                value: Box::new(()),
                            }
                        ));
                    });
                let source_key = boomerang_runtime::EnclaveKey::from(0);
                let source_wake = source.create_send_context(source_key);
                let mut enclaves = tinymap::TinyMap::new();
                assert_eq!(enclaves.insert(source), source_key);
                assert_eq!(enclaves.insert(downstream), downstream_key);
                let runtimes = BTreeMap::from([(federate.clone(), enclaves)]);
                let fixed_point_observations = Arc::clone(&observations);
                let frames = Arc::new(Mutex::new(Vec::new()));
                let fake_frames = Arc::clone(&frames);

                let result = execute_with_fake_rti_config(
                    runtimes,
                    Vec::new(),
                    boomerang_runtime::Config::default()
                        .with_fast_forward(true)
                        .with_keep_alive(true),
                    move |mut transport| async move {
                        assert!(matches!(
                            recv_federate_frame(&mut transport).await,
                            crate::FederateToRti::Hello { federate_id } if federate_id == federate
                        ));
                        send_federate_frame(
                            &mut transport,
                            crate::RtiToFederate::Start {
                                start_unix_epoch_ns: 0,
                            },
                        )
                        .await;
                        let mut shutdown_sent = false;
                        loop {
                            let frame = recv_federate_frame(&mut transport).await;
                            fake_frames.lock().unwrap().push(frame.clone());
                            match frame {
                                crate::FederateToRti::Net { tag, .. }
                                    if tag != crate::WireTag::FOREVER =>
                                {
                                    send_federate_frame(
                                        &mut transport,
                                        crate::RtiToFederate::Tag { tag },
                                    )
                                    .await;
                                }
                                crate::FederateToRti::Ltc {
                                    tag: crate::WireTag::ZERO,
                                    ..
                                } if !shutdown_sent => {
                                    assert_eq!(
                                        fixed_point_observations.lock().unwrap().as_slice(),
                                        &[
                                            ("source", boomerang_runtime::Tag::ZERO),
                                            ("downstream", boomerang_runtime::Tag::ZERO),
                                        ],
                                        "LTC escaped before the idle downstream processed later local work"
                                    );
                                    shutdown_sent = true;
                                    assert!(source_wake.schedule_external(
                                        boomerang_runtime::AsyncEvent::Shutdown {
                                            delay: boomerang_runtime::Duration::ZERO,
                                        }
                                    ));
                                    assert!(downstream_wake.schedule_external(
                                        boomerang_runtime::AsyncEvent::Shutdown {
                                            delay: boomerang_runtime::Duration::ZERO,
                                        }
                                    ));
                                }
                                crate::FederateToRti::Ltc { .. }
                                | crate::FederateToRti::Net {
                                    tag: crate::WireTag::FOREVER,
                                    ..
                                } => {}
                                crate::FederateToRti::Stop { .. } => break,
                                frame => panic!("unexpected frame: {frame:?}"),
                            }
                        }
                        assert!(shutdown_sent, "post-work fixed point was never reported");
                        Ok(())
                    },
                );

                assert!(result.is_ok(), "runner failed: {result:?}");
                assert_eq!(
                    frames
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|frame| matches!(
                            frame,
                            crate::FederateToRti::Ltc {
                                tag: crate::WireTag::ZERO,
                                ..
                            }
                        ))
                        .count(),
                    1
                );
            },
        );
    }

    #[test]
    fn all_idle_participants_remain_live_without_a_finite_net() {
        run_with_wall_timeout("all-idle participants", StdDuration::from_secs(2), || {
            let federate = FederateId::new("federate");
            let mut enclaves = tinymap::TinyMap::new();
            let first_key = enclaves.insert(boomerang_runtime::Enclave::default());
            let second_key = enclaves.insert(boomerang_runtime::Enclave::default());
            let first_wake = enclaves[first_key].create_send_context(first_key);
            let second_wake = enclaves[second_key].create_send_context(second_key);
            let runtimes = BTreeMap::from([(federate.clone(), enclaves)]);
            let frames = Arc::new(Mutex::new(Vec::new()));
            let fake_frames = Arc::clone(&frames);

            let result = execute_with_fake_rti_config(
                runtimes,
                Vec::new(),
                boomerang_runtime::Config::default()
                    .with_fast_forward(true)
                    .with_keep_alive(true),
                move |mut transport| async move {
                    assert!(matches!(
                        recv_federate_frame(&mut transport).await,
                        crate::FederateToRti::Hello { federate_id } if federate_id == federate
                    ));
                    send_federate_frame(
                        &mut transport,
                        crate::RtiToFederate::Start {
                            start_unix_epoch_ns: 0,
                        },
                    )
                    .await;
                    tokio::task::spawn_blocking(|| {
                        std::thread::sleep(StdDuration::from_millis(50));
                    })
                    .await
                    .unwrap();
                    assert!(
                        transport.1.next().now_or_never().is_none(),
                        "all-idle live participants emitted an RTI frame"
                    );
                    assert!(first_wake.schedule_external(
                        boomerang_runtime::AsyncEvent::Shutdown {
                            delay: boomerang_runtime::Duration::ZERO,
                        }
                    ));
                    assert!(second_wake.schedule_external(
                        boomerang_runtime::AsyncEvent::Shutdown {
                            delay: boomerang_runtime::Duration::ZERO,
                        }
                    ));
                    loop {
                        let frame = recv_federate_frame(&mut transport).await;
                        fake_frames.lock().unwrap().push(frame.clone());
                        match frame {
                            crate::FederateToRti::Net { tag, .. }
                                if tag != crate::WireTag::FOREVER =>
                            {
                                send_federate_frame(
                                    &mut transport,
                                    crate::RtiToFederate::Tag { tag },
                                )
                                .await;
                            }
                            crate::FederateToRti::Ltc { .. }
                            | crate::FederateToRti::Net {
                                tag: crate::WireTag::FOREVER,
                                ..
                            } => {}
                            crate::FederateToRti::Stop { .. } => break,
                            frame => panic!("unexpected frame: {frame:?}"),
                        }
                    }
                    Ok(())
                },
            );

            assert!(result.is_ok(), "runner failed: {result:?}");
            assert!(matches!(
                frames.lock().unwrap().first(),
                Some(crate::FederateToRti::Net {
                    tag: crate::WireTag::ZERO,
                    ..
                })
            ));
        });
    }

    #[test]
    fn scheduler_panic_stops_waiting_peers() {
        let result = run_with_wall_timeout(
            "scheduler panic propagation",
            StdDuration::from_secs(2),
            || {
                let federate = FederateId::new("federate");
                let waiting_tag =
                    boomerang_runtime::Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
                let waiting = scheduled_enclave("waiting", waiting_tag, |_| {});
                let panicking =
                    scheduled_enclave("panicking", boomerang_runtime::Tag::ZERO, |_| {
                        panic!("intentional scheduler panic");
                    });
                let mut enclaves = tinymap::TinyMap::new();
                enclaves.insert(waiting);
                enclaves.insert(panicking);
                let runtimes = BTreeMap::from([(federate.clone(), enclaves)]);

                execute_with_fake_rti(runtimes, Vec::new(), move |mut transport| async move {
                    assert!(matches!(
                        recv_federate_frame(&mut transport).await,
                        crate::FederateToRti::Hello { federate_id } if federate_id == federate
                    ));
                    send_federate_frame(
                        &mut transport,
                        crate::RtiToFederate::Start {
                            start_unix_epoch_ns: 0,
                        },
                    )
                    .await;
                    assert!(matches!(
                        recv_federate_frame(&mut transport).await,
                        crate::FederateToRti::Net { .. }
                    ));
                    send_federate_frame(
                        &mut transport,
                        crate::RtiToFederate::Tag {
                            tag: crate::WireTag::ZERO,
                        },
                    )
                    .await;

                    loop {
                        if matches!(
                            recv_federate_frame(&mut transport).await,
                            crate::FederateToRti::Stop { .. }
                        ) {
                            break;
                        }
                    }
                    Ok(())
                })
            },
        );

        assert!(matches!(
            result,
            Err(StaticFederationRunnerError::SchedulerThreadPanic { what })
                if what.contains("intentional scheduler panic")
        ));
    }

    #[test]
    fn scheduler_thread_catches_panics_outside_the_event_loop() {
        let result = catch_scheduler_thread_body(|| {
            panic!("intentional scheduler construction panic");
        });

        assert!(matches!(
            result,
            SchedulerThreadResult::Panicked { what }
                if what.contains("intentional scheduler construction panic")
        ));
    }

    #[test]
    fn closed_completion_channel_runs_cleanup_before_returning_error() {
        let (completion_tx, completion_rx) = std::sync::mpsc::channel();
        drop(completion_tx);
        let cleaned = std::sync::atomic::AtomicBool::new(false);

        let result = receive_scheduler_result(&completion_rx, || {
            cleaned.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        assert!(matches!(
            result,
            Err(StaticFederationRunnerError::Bridge { .. })
        ));
        assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
    }

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
