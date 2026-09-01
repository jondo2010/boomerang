//! Standard-library reference implementation for synchronously executing validated compiled enclave images as a behavioral baseline for target executors.

use std::{
    any::{Any, TypeId},
    collections::BTreeMap,
    fmt,
    marker::PhantomData,
    panic::AssertUnwindSafe,
    time::Instant,
};

use tinymap::{TinyMap, TinySecondaryMap};

use crate::{
    image::{
        BoundaryId, CompiledDeploymentImage, CompiledDeploymentView, EnclaveImage,
        EnclaveImageView, EnclaveIndex, FederateIndex, ImageValidationError, PortIndex,
        RouteDirection, RouteIndex, StateSlotIndex, TimingDomain,
    },
    run_owned_scheduler,
    sched::{
        federate::{
            EnclaveDependencies, FederateQuiescence, FederateQuiescenceCoordinator,
            FederateQuiescenceHandle,
        },
        run_owned_scheduler_with_coordination,
    },
    storage::owned::StoredState,
    AsyncEvent, Config, EnclaveBindings, OwnedStorage, OwnedStorageError, PayloadType, ReactorData,
    RuntimeError, Tag,
};

/// Failure while validating, initializing, or synchronously executing a compiled image.
#[derive(Debug, thiserror::Error)]
pub enum ExecuteOwnedError<'image> {
    /// The borrowed compiled image was structurally invalid.
    #[error("invalid compiled image: {0}")]
    ImageValidation(ImageValidationError<'image>),
    /// The image contains scheduler-boundary routes, which this local executor cannot deliver.
    #[error("compiled reference execution does not support {count} scheduler-boundary route(s)")]
    RoutesUnsupported {
        /// Number of routes present in the validated enclave image.
        count: usize,
    },
    /// Owned storage initialization or directly bound reaction execution failed.
    #[error("compiled storage or reaction execution failed: {0}")]
    Storage(#[from] OwnedStorageError),
    /// The scheduler's local logical-time coordination failed.
    #[error("compiled scheduler coordination failed: {0}")]
    Coordination(#[source] RuntimeError),
}

impl<'image> From<ImageValidationError<'image>> for ExecuteOwnedError<'image> {
    fn from(source: ImageValidationError<'image>) -> Self {
        Self::ImageValidation(source)
    }
}

impl<'image> From<crate::sched::SchedulerError<OwnedStorageError>> for ExecuteOwnedError<'image> {
    fn from(error: crate::sched::SchedulerError<OwnedStorageError>) -> Self {
        match error {
            crate::sched::SchedulerError::Coordination(source) => Self::Coordination(source),
            crate::sched::SchedulerError::Execution(source) => Self::Storage(source),
        }
    }
}

/// A typed state-access failure from an owned compiled-image execution result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateAccessError {
    /// The requested slot exceeds the dense state table returned by execution.
    #[error("state slot {slot} is out of range")]
    OutOfRange {
        /// The requested compiled state storage slot.
        slot: StateSlotIndex,
    },
    /// The requested concrete Rust type differs from the state value's recorded type.
    #[error("state slot {slot} has type {found}, not {expected}")]
    TypeMismatch {
        /// The requested compiled state storage slot.
        slot: StateSlotIndex,
        /// The requested concrete Rust type.
        expected: &'static str,
        /// The concrete Rust type captured when the state was initialized.
        found: &'static str,
    },
}

/// Final owned state retained after synchronously executing one scheduler-owned Enclave.
/// It owns no scheduler machinery or image borrow and may outlive the executed image.
pub struct EnclaveExecution {
    /// Final owned reactor states keyed by compiled storage slot.
    states: TinyMap<StateSlotIndex, StoredState>,
    /// Last logical tag that processed non-terminal work.
    final_tag: Tag,
}

impl EnclaveExecution {
    /// Borrows the state stored at `slot` as its original concrete type.
    /// Invalid slots or concrete types return [`StateAccessError`].
    pub fn state<T: ReactorData>(&self, slot: StateSlotIndex) -> Result<&T, StateAccessError> {
        if slot.as_u32() as usize >= self.states.len() {
            return Err(StateAccessError::OutOfRange { slot });
        }
        let state = &self.states[slot];
        state
            .value
            .downcast_ref::<T>()
            .ok_or(StateAccessError::TypeMismatch {
                slot,
                expected: std::any::type_name::<T>(),
                found: state.type_name,
            })
    }

    /// Returns the last logical tag at which non-shutdown work was processed.
    /// Returns [`Tag::NEVER`] if execution reached only terminal shutdown processing.
    pub const fn final_tag(&self) -> Tag {
        self.final_tag
    }
}

/// Direct owned bindings aggregating every Enclave and typed local route executed under one
/// compiled Federate's shared coordination.
#[derive(Default)]
pub struct FederateBindings<'binding> {
    /// Caller-supplied Enclave bindings keyed directly by canonical deployment index.
    enclaves: TinySecondaryMap<EnclaveIndex, EnclaveBindings>,
    /// Repeated Enclave indices retained for pre-initialization duplicate validation.
    duplicate_enclaves: TinySecondaryMap<EnclaveIndex, ()>,
    /// Statically typed route adapters retained until image preflight resolves their endpoints.
    routes: Vec<Box<dyn RouteBinding + 'binding>>,
}

impl<'binding> FederateBindings<'binding> {
    /// Creates an empty Federate binding set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds one canonical Enclave to its complete direct owned bindings.
    pub fn bind_enclave(mut self, enclave: EnclaveIndex, bindings: EnclaveBindings) -> Self {
        if self.enclaves.insert(enclave, bindings).is_some() {
            self.duplicate_enclaves.insert(enclave, ());
        }
        self
    }

    /// Binds a route only when Rust has unified both endpoint payloads as the same `T`.
    pub fn bind_route<T: ReactorData + Clone>(
        mut self,
        boundary: BoundaryId<'binding>,
        source: PayloadType<T>,
        destination: PayloadType<T>,
    ) -> Self {
        let _ = (source, destination);
        self.routes.push(Box::new(TypedRouteBinding::<T> {
            boundary,
            marker: PhantomData,
        }));
        self
    }
}

/// Private typed-erased route binding installed after structural and payload preflight.
trait RouteBinding: Send {
    /// Returns the stable compiled boundary identity selected by the caller.
    fn boundary(&self) -> BoundaryId<'_>;
    /// Returns the concrete endpoint payload type selected at compile time.
    fn payload_type(&self) -> (TypeId, &'static str);
    /// Installs this route in its validated source storage.
    fn install<'image>(
        &self,
        source: &mut OwnedStorage<'image>,
        route: &ResolvedLocalRoute<'image>,
        destination_tx: crate::Sender<AsyncEvent>,
    );
}

/// Compile-time endpoint witness erased only after `T` is identical at both endpoints.
struct TypedRouteBinding<'binding, T: ReactorData + Clone> {
    /// Stable boundary identity borrowed from the caller.
    boundary: BoundaryId<'binding>,
    /// Retains the statically unified endpoint type without allocating a value.
    marker: PhantomData<fn() -> T>,
}

impl<T: ReactorData + Clone> RouteBinding for TypedRouteBinding<'_, T> {
    fn boundary(&self) -> BoundaryId<'_> {
        self.boundary
    }

    fn payload_type(&self) -> (TypeId, &'static str) {
        (TypeId::of::<T>(), std::any::type_name::<T>())
    }

    fn install<'image>(
        &self,
        source: &mut OwnedStorage<'image>,
        route: &ResolvedLocalRoute<'image>,
        destination_tx: crate::Sender<AsyncEvent>,
    ) {
        source.bind_outbound_route::<T>(
            route.source_port,
            route.boundary,
            route.destination,
            route.destination_port,
            route.timing_domain,
            route.delay_nanos,
            destination_tx,
        );
    }
}

/// Validated local endpoints and timing for one paired scheduler route.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LocalRouteKey {
    /// Canonical source Enclave containing the outbound route table.
    source: EnclaveIndex,
    /// Dense outbound route slot within the source Enclave.
    outbound: RouteIndex,
}

/// A validated same-Federate route resolved to canonical dense scheduler coordinates.
struct ResolvedLocalRoute<'image> {
    /// Typed source Enclave and outbound route slot used for internal selection.
    key: LocalRouteKey,
    /// Stable boundary identity shared by both route halves.
    boundary: BoundaryId<'image>,
    /// Canonical source Enclave index.
    source: EnclaveIndex,
    /// Dense source port selected by the outbound half.
    source_port: PortIndex,
    /// Canonical destination Enclave index.
    destination: EnclaveIndex,
    /// Dense destination port selected by the inbound half.
    destination_port: PortIndex,
    /// Compiled timing domain shared by both halves.
    timing_domain: TimingDomain,
    /// Compiled delay shared by both halves.
    delay_nanos: u64,
}

/// Final owned results aggregating every canonical Enclave executed through typed local routes
/// under one Federate's shared coordination origin.
pub struct FederateExecution {
    /// Per-Enclave results keyed by deployment-wide canonical Enclave index.
    enclaves: TinySecondaryMap<EnclaveIndex, EnclaveExecution>,
    /// Single monotonic origin injected into every scheduler and reaction context.
    origin: Instant,
}

impl fmt::Debug for FederateExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FederateExecution")
            .field("enclaves", &self.enclaves.len())
            .field("origin", &self.origin)
            .finish()
    }
}

impl FederateExecution {
    /// Returns one Enclave's final owned result by canonical deployment index.
    pub fn enclave(&self, enclave: EnclaveIndex) -> Option<&EnclaveExecution> {
        self.enclaves.get(enclave)
    }

    /// Returns the monotonic origin shared by every Enclave scheduler.
    pub const fn origin(&self) -> Instant {
        self.origin
    }
}

/// Failure while preflighting, initializing, or executing one owned compiled Federate.
#[derive(Debug, thiserror::Error)]
pub enum ExecuteOwnedFederateError {
    /// The deployment root or one nested image was structurally invalid.
    #[error("invalid compiled deployment: {message}")]
    ImageValidation {
        /// Complete typed validation diagnostic retained as owned text.
        message: String,
    },
    /// The requested canonical Federate index is absent.
    #[error("compiled Federate {federate} is not present")]
    FederateNotFound {
        /// Requested canonical Federate index.
        federate: FederateIndex,
    },
    /// One selected Enclave received no direct binding set.
    #[error("missing bindings for Enclave {enclave}")]
    MissingEnclaveBinding {
        /// Canonical Enclave index lacking bindings.
        enclave: EnclaveIndex,
    },
    /// A canonical Enclave binding was supplied more than once.
    #[error("duplicate bindings for Enclave {enclave}")]
    DuplicateEnclaveBinding {
        /// Canonical Enclave index supplied repeatedly.
        enclave: EnclaveIndex,
    },
    /// A binding named an Enclave outside the selected Federate.
    #[error("Enclave {enclave} does not belong to Federate {federate}")]
    UnexpectedEnclaveBinding {
        /// Canonical Enclave index outside the selected range.
        enclave: EnclaveIndex,
        /// Selected canonical Federate index.
        federate: FederateIndex,
    },
    /// A selected route crosses the boundary of the local Federate executor.
    #[error("route '{boundary}' crosses Enclaves {source_enclave} -> {destination} outside Federate {federate}")]
    CrossFederateRoute {
        /// Stable boundary identity.
        boundary: String,
        /// Canonical source Enclave index.
        source_enclave: EnclaveIndex,
        /// Canonical destination Enclave index.
        destination: EnclaveIndex,
        /// Selected canonical Federate index.
        federate: FederateIndex,
    },
    /// A compiled local route received no typed route binding.
    #[error("missing typed binding for route '{boundary}'")]
    MissingRouteBinding {
        /// Stable boundary identity lacking a binding.
        boundary: String,
    },
    /// A stable route identity was bound more than once.
    #[error("duplicate typed binding for route '{boundary}'")]
    DuplicateRouteBinding {
        /// Repeated stable boundary identity.
        boundary: String,
    },
    /// A typed route binding did not name a route in the selected Federate.
    #[error("route binding '{boundary}' is not present in Federate {federate}")]
    UnexpectedRouteBinding {
        /// Unknown stable boundary identity.
        boundary: String,
        /// Selected canonical Federate index.
        federate: FederateIndex,
    },
    /// A route binding disagreed with a concrete endpoint port binding.
    #[error("route '{boundary}' {direction:?} endpoint at Enclave {enclave} port {port} requires {expected}, found {found}")]
    RoutePayloadTypeMismatch {
        /// Stable boundary identity.
        boundary: String,
        /// Mismatched route half.
        direction: RouteDirection,
        /// Canonical endpoint Enclave index.
        enclave: EnclaveIndex,
        /// Dense endpoint port.
        port: PortIndex,
        /// Concrete type selected by the typed route binding.
        expected: &'static str,
        /// Concrete type selected by the port binding.
        found: &'static str,
    },
    /// A compiled route delay cannot fit the runtime logical duration.
    #[error("route '{boundary}' delay {delay_nanos}ns exceeds the runtime duration range")]
    RouteDelayOutOfRange {
        /// Stable boundary identity.
        boundary: String,
        /// Unrepresentable compiled delay.
        delay_nanos: u64,
    },
    /// One Enclave image or direct binding set failed initializer-free preflight.
    #[error("Enclave {enclave} preflight failed: {source}")]
    EnclavePreflight {
        /// Canonical failing Enclave index.
        enclave: EnclaveIndex,
        /// Existing owned-storage validation failure.
        #[source]
        source: OwnedStorageError,
    },
    /// One Enclave failed while constructing its already validated owned storage.
    #[error("Enclave {enclave} initialization failed: {source}")]
    EnclaveInitialization {
        /// Canonical failing Enclave index.
        enclave: EnclaveIndex,
        /// Existing owned-storage construction failure.
        #[source]
        source: OwnedStorageError,
    },
    /// The Federate-wide quiescence coordinator thread could not be created.
    #[error("failed to spawn Federate quiescence coordinator thread: {source}")]
    CoordinatorThreadSpawn {
        /// Operating-system thread creation failure.
        #[source]
        source: std::io::Error,
    },
    /// One Enclave scheduler thread could not be created.
    #[error("failed to spawn scheduler thread for Enclave {enclave}: {source}")]
    ThreadSpawn {
        /// Canonical Enclave index whose scheduler was not started.
        enclave: EnclaveIndex,
        /// Operating-system thread creation failure.
        #[source]
        source: std::io::Error,
    },
    /// One Enclave scheduler or typed route failed during execution.
    #[error("Enclave {enclave} execution failed: {source}")]
    EnclaveExecution {
        /// Canonical failing Enclave index.
        enclave: EnclaveIndex,
        /// Existing storage, reaction, or typed route failure.
        #[source]
        source: OwnedStorageError,
    },
    /// One Enclave scheduler failed in local logical-time coordination.
    #[error("Enclave {enclave} coordination failed: {source}")]
    EnclaveCoordination {
        /// Canonical failing Enclave index.
        enclave: EnclaveIndex,
        /// Existing scheduler coordination failure.
        #[source]
        source: RuntimeError,
    },
    /// One Enclave scheduler thread panicked before producing a result.
    #[error("Enclave {enclave} thread panicked: {message}")]
    ThreadPanicked {
        /// Canonical panicking Enclave index.
        enclave: EnclaveIndex,
        /// Best-effort panic payload diagnostic.
        message: String,
    },
    /// The executor's internal result channel closed before every thread reported.
    #[error("Federate result channel closed before all Enclaves joined")]
    ResultChannelClosed,
}

/// Converts a panic payload into a stable best-effort diagnostic.
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload
            .downcast::<&'static str>()
            .map(|message| (*message).to_owned())
            .unwrap_or_else(|_| "non-string panic payload".to_owned()),
    }
}

/// Requests immediate shutdown from every still-live Enclave scheduler without blocking.
fn request_federate_shutdown(senders: &[crate::Sender<AsyncEvent>]) {
    for sender in senders {
        let _ = sender.close();
    }
}

#[cfg(test)]
#[test]
fn federate_shutdown_unblocks_a_full_mailbox_and_blocked_sender() {
    let (sender, receiver) = kanal::bounded(1);
    sender
        .send(AsyncEvent::Shutdown {
            delay: crate::Duration::ZERO,
        })
        .unwrap();
    let blocked = sender.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        done_tx
            .send(blocked.send(AsyncEvent::Shutdown {
                delay: crate::Duration::ZERO,
            }))
            .unwrap();
    });

    request_federate_shutdown(&[sender]);
    let result = done_rx.recv_timeout(std::time::Duration::from_secs(1));
    drop(receiver);
    worker.join().unwrap();
    assert!(matches!(result, Ok(Err(_))));
}

/// Returns whether a canonical Enclave index belongs to one Federate's dense range.
fn federate_contains_enclave(federate: crate::image::FederateImage, enclave: EnclaveIndex) -> bool {
    federate.enclaves().contains(enclave)
}

/// Resolves every outbound route to its unique inbound half after root validation.
fn local_route_endpoints<'image>(
    deployment: &CompiledDeploymentImage<'image>,
    selected_federate: FederateIndex,
    selected: crate::image::FederateImage,
) -> Result<Vec<ResolvedLocalRoute<'image>>, ExecuteOwnedFederateError> {
    let mut endpoints = Vec::new();
    for (source, source_image) in deployment.enclaves.iter() {
        for (outbound_route, outbound) in source_image
            .routes
            .iter()
            .filter(|(_, route)| route.direction() == RouteDirection::Outbound)
        {
            let boundary = BoundaryId::new(
                outbound
                    .boundary()
                    .get(source_image.identity_data)
                    .expect("root validation checked outbound boundary identity"),
            );
            let (destination, inbound) = deployment
                .enclaves
                .iter()
                .flat_map(|(enclave, image)| {
                    image
                        .routes
                        .values()
                        .map(move |route| (enclave, image, route))
                })
                .find(|(_, image, route)| {
                    route.direction() == RouteDirection::Inbound
                        && route
                            .boundary()
                            .get(image.identity_data)
                            .map(BoundaryId::new)
                            == Some(boundary)
                })
                .map(|(enclave, _, route)| (enclave, *route))
                .expect("root validation paired every outbound route");
            let source_selected = federate_contains_enclave(selected, source);
            let destination_selected = federate_contains_enclave(selected, destination);
            if source_selected != destination_selected {
                return Err(ExecuteOwnedFederateError::CrossFederateRoute {
                    boundary: boundary.as_str().to_owned(),
                    source_enclave: source,
                    destination,
                    federate: selected_federate,
                });
            }
            if source_selected {
                endpoints.push(ResolvedLocalRoute {
                    key: LocalRouteKey {
                        source,
                        outbound: outbound_route,
                    },
                    boundary,
                    source,
                    source_port: outbound.local_port(),
                    destination,
                    destination_port: inbound.local_port(),
                    timing_domain: outbound.timing_domain(),
                    delay_nanos: outbound.delay_nanos(),
                });
            }
        }
    }
    Ok(endpoints)
}

/// Validates complete Enclave and route bindings before any user initializer runs.
fn preflight_owned_federate<'image>(
    deployment: &CompiledDeploymentImage<'image>,
    federate: FederateIndex,
    bindings: &FederateBindings<'_>,
) -> Result<Vec<ResolvedLocalRoute<'image>>, ExecuteOwnedFederateError> {
    CompiledDeploymentView::new(deployment).map_err(|error| {
        ExecuteOwnedFederateError::ImageValidation {
            message: error.to_string(),
        }
    })?;
    let selected = deployment
        .federates
        .get(federate)
        .copied()
        .ok_or(ExecuteOwnedFederateError::FederateNotFound { federate })?;

    for enclave in bindings.enclaves.keys() {
        if !federate_contains_enclave(selected, enclave) {
            return Err(ExecuteOwnedFederateError::UnexpectedEnclaveBinding { enclave, federate });
        }
    }
    if let Some(enclave) = bindings.duplicate_enclaves.keys().next() {
        return Err(ExecuteOwnedFederateError::DuplicateEnclaveBinding { enclave });
    }

    let start = selected.enclaves().start();
    let end = start + selected.enclaves().len();
    for raw in start..end {
        let enclave = EnclaveIndex::new(raw);
        let enclave_bindings = bindings
            .enclaves
            .get(enclave)
            .ok_or(ExecuteOwnedFederateError::MissingEnclaveBinding { enclave })?;
        let image = EnclaveImageView::new(&deployment.enclaves[enclave]).map_err(|error| {
            ExecuteOwnedFederateError::ImageValidation {
                message: error.to_string(),
            }
        })?;
        OwnedStorage::validate_image_bindings(&image, enclave_bindings)
            .map_err(|source| ExecuteOwnedFederateError::EnclavePreflight { enclave, source })?;
    }

    let endpoints = local_route_endpoints(deployment, federate, selected)?;
    for route in &bindings.routes {
        let matches = bindings
            .routes
            .iter()
            .filter(|candidate| candidate.boundary() == route.boundary())
            .count();
        if matches > 1 {
            return Err(ExecuteOwnedFederateError::DuplicateRouteBinding {
                boundary: route.boundary().as_str().to_owned(),
            });
        }
        if !endpoints
            .iter()
            .any(|endpoint| endpoint.boundary == route.boundary())
        {
            return Err(ExecuteOwnedFederateError::UnexpectedRouteBinding {
                boundary: route.boundary().as_str().to_owned(),
                federate,
            });
        }
    }
    for endpoint in &endpoints {
        let route = bindings
            .routes
            .iter()
            .find(|route| route.boundary() == endpoint.boundary)
            .ok_or_else(|| ExecuteOwnedFederateError::MissingRouteBinding {
                boundary: endpoint.boundary.as_str().to_owned(),
            })?;
        if endpoint.delay_nanos > i64::MAX as u64 {
            return Err(ExecuteOwnedFederateError::RouteDelayOutOfRange {
                boundary: endpoint.boundary.as_str().to_owned(),
                delay_nanos: endpoint.delay_nanos,
            });
        }
        for (direction, enclave, port) in [
            (
                RouteDirection::Outbound,
                endpoint.source,
                endpoint.source_port,
            ),
            (
                RouteDirection::Inbound,
                endpoint.destination,
                endpoint.destination_port,
            ),
        ] {
            let image = EnclaveImageView::new(&deployment.enclaves[enclave])
                .expect("root validation checked endpoint images");
            let slot = image.ports()[port].binding();
            let (found_id, found) = bindings
                .enclaves
                .get(enclave)
                .and_then(|owned| owned.port_payload_type(slot))
                .expect("owned storage preflight checked endpoint port bindings");
            let (expected_id, expected) = route.payload_type();
            if found_id != expected_id {
                return Err(ExecuteOwnedFederateError::RoutePayloadTypeMismatch {
                    boundary: endpoint.boundary.as_str().to_owned(),
                    direction,
                    enclave,
                    port,
                    expected,
                    found,
                });
            }
        }
    }
    Ok(endpoints)
}

/// Executes every Enclave in one validated Federate with direct typed local routes.
///
/// All root, binding, route, timer, and timing checks complete before any user initializer runs.
/// The first execution failure triggers Federate-wide shutdown; every scheduler thread is joined.
/// Automatic quiescence covers executor-owned scheduler and local-route work. Callers that admit
/// exogenous events must set [`Config::keep_alive`] and request shutdown explicitly.
pub fn execute_owned_federate(
    deployment: &CompiledDeploymentImage<'_>,
    federate: FederateIndex,
    bindings: FederateBindings<'_>,
    config: Config,
) -> Result<FederateExecution, ExecuteOwnedFederateError> {
    execute_owned_federate_with_spawn_guard(deployment, federate, bindings, config, |_| false)
}

/// Executes one owned Federate while consulting a deterministic scoped-spawn failure seam.
///
/// The guard receives `None` for the quiescence coordinator and `Some(enclave)` for each
/// scheduler. Production always returns `false`; unit tests use the guard to exercise failures
/// that cannot be induced safely through operating-system resource exhaustion.
fn execute_owned_federate_with_spawn_guard(
    deployment: &CompiledDeploymentImage<'_>,
    federate: FederateIndex,
    bindings: FederateBindings<'_>,
    config: Config,
    mut fail_spawn: impl FnMut(Option<EnclaveIndex>) -> bool,
) -> Result<FederateExecution, ExecuteOwnedFederateError> {
    let endpoints = preflight_owned_federate(deployment, federate, &bindings)?;
    let FederateBindings {
        enclaves,
        duplicate_enclaves: _,
        routes,
    } = bindings;
    let route_bindings = endpoints
        .iter()
        .map(|route| {
            let binding = routes
                .iter()
                .position(|binding| binding.boundary() == route.boundary)
                .expect("Federate preflight required every local route binding");
            (route.key, binding)
        })
        .collect::<BTreeMap<_, _>>();
    let origin = Instant::now();
    let mut storages = Vec::with_capacity(enclaves.len());
    for (enclave, owned) in enclaves {
        let image = EnclaveImageView::new(&deployment.enclaves[enclave])
            .expect("Federate preflight validated every selected Enclave image");
        let enclave_key = crate::EnclaveKey::from(enclave.as_u32() as usize);
        let storage =
            OwnedStorage::new_for_enclave(image, owned, enclave_key, origin).map_err(|source| {
                ExecuteOwnedFederateError::EnclaveInitialization { enclave, source }
            })?;
        storages.push((enclave, storage));
    }

    let event_senders = storages
        .iter()
        .map(|(_, storage)| storage.scheduler_event_tx())
        .collect::<Vec<_>>();
    for endpoint in &endpoints {
        let route = &routes[route_bindings[&endpoint.key]];
        let destination_tx = storages
            .iter()
            .find(|(enclave, _)| *enclave == endpoint.destination)
            .map(|(_, storage)| storage.scheduler_event_tx())
            .expect("Federate preflight required destination storage");
        let source = storages
            .iter_mut()
            .find(|(enclave, _)| *enclave == endpoint.source)
            .map(|(_, storage)| storage)
            .expect("Federate preflight required source storage");
        route.install(source, endpoint, destination_tx);
    }

    let scheduler_contexts = storages
        .iter()
        .map(|(enclave, storage)| (*enclave, storage.scheduler_send_context()))
        .collect::<Vec<_>>();
    let mut coordinations = storages
        .iter()
        .map(|(enclave, _)| {
            (
                *enclave,
                EnclaveDependencies::new(crate::EnclaveKey::from(enclave.as_u32() as usize)),
            )
        })
        .collect::<Vec<_>>();
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| endpoint.timing_domain == TimingDomain::Logical)
    {
        let source_key = crate::EnclaveKey::from(endpoint.source.as_u32() as usize);
        let destination_key = crate::EnclaveKey::from(endpoint.destination.as_u32() as usize);
        let source_context = scheduler_contexts
            .iter()
            .find_map(|(enclave, context)| (*enclave == endpoint.source).then(|| context.clone()))
            .expect("selected route source has a configured scheduler context");
        let destination_context = scheduler_contexts
            .iter()
            .find_map(|(enclave, context)| {
                (*enclave == endpoint.destination).then(|| context.clone())
            })
            .expect("selected route destination has a configured scheduler context");
        let delay = (endpoint.delay_nanos != 0)
            .then(|| crate::Duration::nanoseconds(endpoint.delay_nanos as i64));
        coordinations
            .iter_mut()
            .find(|(enclave, _)| *enclave == endpoint.source)
            .expect("selected route source has scheduler coordination")
            .1
            .add_downstream(destination_key, destination_context);
        coordinations
            .iter_mut()
            .find(|(enclave, _)| *enclave == endpoint.destination)
            .expect("selected route destination has scheduler coordination")
            .1
            .add_upstream(source_key, source_context, delay);
    }
    let enclave_count = storages.len();
    let quiescence = (!config.keep_alive).then(|| {
        FederateQuiescence::new(storages.iter().map(|(enclave, storage)| {
            (
                crate::EnclaveKey::from(enclave.as_u32() as usize),
                storage.scheduler_event_rx(),
            )
        }))
    });
    let (quiescence_handle, quiescence_coordinator, mut quiescence_participants): (
        Option<FederateQuiescenceHandle>,
        Option<FederateQuiescenceCoordinator>,
        _,
    ) = match quiescence {
        Some(quiescence) => (
            Some(quiescence.abort_handle),
            Some(quiescence.coordinator),
            quiescence.participants,
        ),
        None => (None, None, BTreeMap::new()),
    };
    let (results, failure) = std::thread::scope(|scope| {
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let coordinator_thread = match quiescence_coordinator {
            Some(coordinator) => {
                let spawned = if fail_spawn(None) {
                    Err(std::io::Error::other(
                        "injected scoped thread spawn failure",
                    ))
                } else {
                    std::thread::Builder::new()
                        .name("federate-quiescence".to_owned())
                        .spawn_scoped(scope, move || coordinator.run())
                };
                match spawned {
                    Ok(handle) => Some(handle),
                    Err(source) => {
                        drop(quiescence_participants);
                        request_federate_shutdown(&event_senders);
                        return (
                            TinySecondaryMap::with_capacity(enclave_count),
                            Some(ExecuteOwnedFederateError::CoordinatorThreadSpawn { source }),
                        );
                    }
                }
            }
            None => None,
        };
        let abort = || {
            if let Some(handle) = &quiescence_handle {
                handle.abort();
            }
            request_federate_shutdown(&event_senders);
        };
        let mut handles = Vec::with_capacity(enclave_count);
        let mut failure = None;
        for ((enclave, mut storage), (_, coordination)) in storages.into_iter().zip(coordinations) {
            let result_tx = result_tx.clone();
            let config = config.clone();
            let key = crate::EnclaveKey::from(enclave.as_u32() as usize);
            let mut participant = quiescence_participants.remove(&key);
            let spawned = if fail_spawn(Some(enclave)) {
                Err(std::io::Error::other(
                    "injected scoped thread spawn failure",
                ))
            } else {
                std::thread::Builder::new()
                    .name(format!("enclave-{enclave}"))
                    .spawn_scoped(scope, move || {
                        let execution = std::panic::catch_unwind(AssertUnwindSafe(|| {
                            run_owned_scheduler_with_coordination(
                                &mut storage,
                                &config,
                                origin,
                                coordination,
                                participant.as_mut(),
                            )
                        }));
                        drop(participant);
                        let result = match execution {
                            Ok(Ok(final_tag)) => Ok(EnclaveExecution {
                                states: storage.into_states(),
                                final_tag,
                            }),
                            Ok(Err(crate::sched::SchedulerError::Execution(source))) => {
                                Err(ExecuteOwnedFederateError::EnclaveExecution { enclave, source })
                            }
                            Ok(Err(crate::sched::SchedulerError::Coordination(source))) => {
                                Err(ExecuteOwnedFederateError::EnclaveCoordination {
                                    enclave,
                                    source,
                                })
                            }
                            Err(payload) => Err(ExecuteOwnedFederateError::ThreadPanicked {
                                enclave,
                                message: panic_message(payload),
                            }),
                        };
                        let _ = result_tx.send((enclave, result));
                    })
            };
            match spawned {
                Ok(handle) => handles.push((enclave, handle)),
                Err(source) => {
                    abort();
                    failure = Some(ExecuteOwnedFederateError::ThreadSpawn { enclave, source });
                    break;
                }
            }
        }
        drop(quiescence_participants);
        drop(result_tx);

        let started_count = handles.len();
        let mut results = TinySecondaryMap::with_capacity(enclave_count);
        for _ in 0..started_count {
            match result_rx.recv() {
                Ok((enclave, Ok(result))) => {
                    results.insert(enclave, result);
                }
                Ok((_, Err(error))) => {
                    if failure.is_none() {
                        abort();
                        failure = Some(error);
                    }
                }
                Err(_) => {
                    if failure.is_none() {
                        abort();
                        failure = Some(ExecuteOwnedFederateError::ResultChannelClosed);
                    }
                    break;
                }
            }
        }
        for (enclave, handle) in handles {
            if let Err(payload) = handle.join() {
                if failure.is_none() {
                    abort();
                    failure = Some(ExecuteOwnedFederateError::ThreadPanicked {
                        enclave,
                        message: panic_message(payload),
                    });
                }
            }
        }
        if let Some(handle) = coordinator_thread {
            let _ = handle.join();
        }
        (results, failure)
    });

    if let Some(error) = failure {
        Err(error)
    } else {
        Ok(FederateExecution {
            enclaves: results,
            origin,
        })
    }
}

/// Validates and synchronously executes a borrowed compiled enclave image with direct bindings.
/// Consumes `bindings`; the result retains only final owned state and the last work tag.
///
/// # Errors
///
/// Returns [`ExecuteOwnedError`] for validation, storage, coordination, or reaction failures.
pub fn execute_owned<'image>(
    image: &EnclaveImage<'image>,
    bindings: EnclaveBindings,
    config: Config,
) -> Result<EnclaveExecution, ExecuteOwnedError<'image>> {
    let image = crate::image::EnclaveImageView::new(image)?;
    let unsupported_routes = image.routes().len();
    if unsupported_routes != 0 {
        return Err(ExecuteOwnedError::RoutesUnsupported {
            count: unsupported_routes,
        });
    }
    let mut storage = OwnedStorage::new(image, bindings)?;
    let final_tag = run_owned_scheduler(&mut storage, &config)?;
    Ok(EnclaveExecution {
        states: storage.into_states(),
        final_tag,
    })
}

#[cfg(test)]
mod scoped_spawn_tests {
    use std::{io::ErrorKind, time::Duration};

    use tinymap::TinyMapView;

    use super::*;
    use crate::image::{
        BindingKind, BindingSlotIndex, CoordinationProjection, FederateImage,
        GlobalFederationImage, IdentityRange, ReactorImage, ReactorIndex, RequiredBindingImage,
        ScopeImage, ScopeIndex, StorageBounds, TableRange,
    };

    static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
        BindingSlotIndex::new(0),
        StateSlotIndex::new(0),
        ScopeIndex::new(0),
        TableRange::new(0, 0),
        None,
        None,
    )];
    static SCOPES: [ScopeImage; 1] = [ScopeImage::new(
        None,
        ReactorIndex::new(0),
        None,
        TableRange::new(0, 1),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
    )];
    static SCOPE_DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
    static REQUIRED_BINDINGS: [RequiredBindingImage; 1] = [RequiredBindingImage::new(
        IdentityRange::new(5, 5),
        BindingKind::StateInitializer,
    )];

    const fn state_only_image(identity_data: &'static str) -> EnclaveImage<'static> {
        EnclaveImage {
            identity_data,
            enclave_id: IdentityRange::new(0, 5),
            reactors: TinyMapView::new(&REACTORS),
            actions: TinyMapView::new(&[]),
            ports: TinyMapView::new(&[]),
            reactions: TinyMapView::new(&[]),
            modes: TinyMapView::new(&[]),
            scopes: TinyMapView::new(&SCOPES),
            reaction_triggers: &[],
            reaction_use_ports: &[],
            reaction_effect_ports: &[],
            reaction_actions: &[],
            reaction_modes: &[],
            scope_descendants: &SCOPE_DESCENDANTS,
            scope_logical_actions: &[],
            scope_timer_startups: &[],
            scope_reset_reactions: &[],
            scope_startup_reactions: &[],
            scope_shutdown_reactions: &[],
            startup_actions: &[],
            timer_startup_actions: &[],
            shutdown_reactions: &[],
            shutdown_actions: &[],
            routes: TinyMapView::new(&[]),
            required_bindings: TinyMapView::new(&REQUIRED_BINDINGS),
            storage_bounds: StorageBounds::new(1, 0, 1, 0, 0, 0),
        }
    }

    static ENCLAVES: [EnclaveImage<'static>; 3] = [
        state_only_image("alphastate"),
        state_only_image("bravostate"),
        state_only_image("charlstate"),
    ];
    static FEDERATES: [FederateImage; 1] = [FederateImage::new(
        IdentityRange::new(0, 4),
        IdentityRange::new(4, 6),
        IdentityRange::new(10, 7),
        TableRange::new(0, 3),
    )];
    static MEMBERS: [FederateIndex; 1] = [FederateIndex::new(0)];
    static DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
        identity_data: "hosttargetruntime",
        federation: GlobalFederationImage::new(&MEMBERS, &[]),
        federates: TinyMapView::new(&FEDERATES),
        enclaves: TinyMapView::new(&ENCLAVES),
        coordination: CoordinationProjection::Local,
    };

    fn initialize_state() {}

    fn bindings() -> FederateBindings<'static> {
        (0..3).fold(FederateBindings::new(), |bindings, enclave| {
            bindings.bind_enclave(
                EnclaveIndex::new(enclave),
                EnclaveBindings::new().bind_state(BindingSlotIndex::new(0), initialize_state),
            )
        })
    }

    fn execute_with_spawn_failure(failed_spawn: Option<EnclaveIndex>) -> ExecuteOwnedFederateError {
        execute_owned_federate_with_spawn_guard(
            &DEPLOYMENT,
            FederateIndex::new(0),
            bindings(),
            Config::default().with_fast_forward(true),
            move |spawn| spawn == failed_spawn,
        )
        .expect_err("the selected scoped thread creation must fail")
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "the bounded hang regression requires subprocess support"
    )]
    fn scoped_thread_spawn_failure_is_typed_and_bounded() {
        let executable =
            std::env::current_exe().expect("the runtime unit-test executable is available");
        let mut child = std::process::Command::new(executable)
            .args([
                "--exact",
                "reference::scoped_spawn_tests::scoped_thread_spawn_failure_child",
                "--nocapture",
            ])
            .env("BOOMERANG_SCOPED_SPAWN_FAILURE_CHILD", "1")
            .spawn()
            .expect("the scoped-spawn failure child process starts");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);

        loop {
            if let Some(status) = child
                .try_wait()
                .expect("the scoped-spawn failure child can be polled")
            {
                assert!(
                    status.success(),
                    "scoped-spawn failure child failed: {status}"
                );
                break;
            }
            if std::time::Instant::now() >= deadline {
                child
                    .kill()
                    .expect("the timed-out scoped-spawn failure child is killed");
                child
                    .wait()
                    .expect("the killed scoped-spawn failure child is reaped");
                panic!("scoped thread creation failure did not return within two seconds");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn scoped_thread_spawn_failure_child() {
        if std::env::var_os("BOOMERANG_SCOPED_SPAWN_FAILURE_CHILD").is_none() {
            return;
        }

        let coordinator = execute_with_spawn_failure(None);
        assert!(matches!(
            coordinator,
            ExecuteOwnedFederateError::CoordinatorThreadSpawn { source }
                if source.kind() == ErrorKind::Other
        ));

        let failed_enclave = EnclaveIndex::new(1);
        let scheduler = execute_with_spawn_failure(Some(failed_enclave));
        assert!(matches!(
            scheduler,
            ExecuteOwnedFederateError::ThreadSpawn { enclave, source }
                if enclave == failed_enclave && source.kind() == ErrorKind::Other
        ));
    }
}
