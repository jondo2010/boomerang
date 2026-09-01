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
        federate::EnclaveDependencies, owned_federate_coordination,
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
pub struct FederateBindings {
    /// Caller-supplied Enclave bindings keyed directly by canonical deployment index.
    enclaves: TinySecondaryMap<EnclaveIndex, EnclaveBindings>,
    /// Repeated Enclave indices retained for pre-initialization duplicate validation.
    duplicate_enclaves: TinySecondaryMap<EnclaveIndex, ()>,
    /// Statically typed route adapters retained until image preflight resolves their endpoints.
    routes: Vec<Box<dyn RouteBinding>>,
}

impl FederateBindings {
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
        boundary: BoundaryId<'_>,
        source: PayloadType<T>,
        destination: PayloadType<T>,
    ) -> Self {
        let _ = (source, destination);
        self.routes.push(Box::new(TypedRouteBinding::<T> {
            boundary: boundary.as_str().to_owned(),
            marker: PhantomData,
        }));
        self
    }
}

#[cfg(test)]
#[test]
fn federate_bindings_store_enclaves_by_typed_index() {
    fn assert_typed_map(_bindings: &TinySecondaryMap<EnclaveIndex, EnclaveBindings>) {}

    let enclave = EnclaveIndex::new(3);
    let bindings = FederateBindings::new().bind_enclave(enclave, EnclaveBindings::new());

    assert_typed_map(&bindings.enclaves);
    assert!(bindings.enclaves.contains_key(enclave));
}

/// Private typed-erased route binding installed after structural and payload preflight.
trait RouteBinding: Send {
    /// Returns the stable compiled boundary identity selected by the caller.
    fn boundary(&self) -> &str;
    /// Returns the concrete endpoint payload type selected at compile time.
    fn payload_type(&self) -> (TypeId, &'static str);
    /// Installs this route in its validated source storage.
    fn install(
        &self,
        source: &mut OwnedStorage<'_>,
        route: &ResolvedLocalRoute,
        destination_tx: crate::Sender<AsyncEvent>,
    );
}

/// Compile-time endpoint witness erased only after `T` is identical at both endpoints.
struct TypedRouteBinding<T: ReactorData + Clone> {
    /// Stable boundary identity copied from the caller's borrowed identity.
    boundary: String,
    /// Retains the statically unified endpoint type without allocating a value.
    marker: PhantomData<fn() -> T>,
}

impl<T: ReactorData + Clone> RouteBinding for TypedRouteBinding<T> {
    fn boundary(&self) -> &str {
        &self.boundary
    }

    fn payload_type(&self) -> (TypeId, &'static str) {
        (TypeId::of::<T>(), std::any::type_name::<T>())
    }

    fn install(
        &self,
        source: &mut OwnedStorage<'_>,
        route: &ResolvedLocalRoute,
        destination_tx: crate::Sender<AsyncEvent>,
    ) {
        source.bind_outbound_route::<T>(
            route.source_port,
            route.boundary.clone(),
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
struct ResolvedLocalRoute {
    /// Typed source Enclave and outbound route slot used for internal selection.
    key: LocalRouteKey,
    /// Stable boundary identity shared by both route halves.
    boundary: String,
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
fn local_route_endpoints(
    deployment: &CompiledDeploymentImage<'_>,
    selected_federate: FederateIndex,
    selected: crate::image::FederateImage,
) -> Result<Vec<ResolvedLocalRoute>, ExecuteOwnedFederateError> {
    let mut endpoints = Vec::new();
    for (source, source_image) in deployment.enclaves.iter() {
        for (outbound_route, outbound) in source_image
            .routes
            .iter()
            .filter(|(_, route)| route.direction() == RouteDirection::Outbound)
        {
            let boundary = outbound
                .boundary()
                .get(source_image.identity_data)
                .expect("root validation checked outbound boundary identity");
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
                        && route.boundary().get(image.identity_data) == Some(boundary)
                })
                .map(|(enclave, _, route)| (enclave, *route))
                .expect("root validation paired every outbound route");
            let source_selected = federate_contains_enclave(selected, source);
            let destination_selected = federate_contains_enclave(selected, destination);
            if source_selected != destination_selected {
                return Err(ExecuteOwnedFederateError::CrossFederateRoute {
                    boundary: boundary.to_owned(),
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
                    boundary: boundary.to_owned(),
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
fn preflight_owned_federate(
    deployment: &CompiledDeploymentImage<'_>,
    federate: FederateIndex,
    bindings: &FederateBindings,
) -> Result<Vec<ResolvedLocalRoute>, ExecuteOwnedFederateError> {
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
                boundary: route.boundary().to_owned(),
            });
        }
        if !endpoints
            .iter()
            .any(|endpoint| endpoint.boundary == route.boundary())
        {
            return Err(ExecuteOwnedFederateError::UnexpectedRouteBinding {
                boundary: route.boundary().to_owned(),
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
                boundary: endpoint.boundary.clone(),
            })?;
        if endpoint.delay_nanos > i64::MAX as u64 {
            return Err(ExecuteOwnedFederateError::RouteDelayOutOfRange {
                boundary: endpoint.boundary.clone(),
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
                    boundary: endpoint.boundary.clone(),
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
    bindings: FederateBindings,
    config: Config,
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
    let activity = (!config.keep_alive).then(|| {
        owned_federate_coordination(storages.iter().map(|(enclave, storage)| {
            (
                crate::EnclaveKey::from(enclave.as_u32() as usize),
                storage.scheduler_event_rx(),
            )
        }))
    });
    let (coordinator, coordinator_runner, mut activities) = match activity {
        Some((coordinator, runner, activities)) => (Some(coordinator), Some(runner), activities),
        None => (None, None, BTreeMap::new()),
    };
    let (results, failure) = std::thread::scope(|scope| {
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let coordinator_handle = coordinator_runner.map(|runner| scope.spawn(move || runner.run()));
        let abort = || {
            if let Some(coordinator) = &coordinator {
                coordinator.abort();
            }
            request_federate_shutdown(&event_senders);
        };
        let mut handles = Vec::with_capacity(enclave_count);
        for ((enclave, mut storage), (_, coordination)) in storages.into_iter().zip(coordinations) {
            let result_tx = result_tx.clone();
            let config = config.clone();
            let key = crate::EnclaveKey::from(enclave.as_u32() as usize);
            let mut activity = activities.remove(&key);
            let handle = scope.spawn(move || {
                let execution = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    run_owned_scheduler_with_coordination(
                        &mut storage,
                        &config,
                        origin,
                        coordination,
                        activity.as_mut(),
                    )
                }));
                drop(activity);
                let result = match execution {
                    Ok(Ok(final_tag)) => Ok(EnclaveExecution {
                        states: storage.into_states(),
                        final_tag,
                    }),
                    Ok(Err(crate::sched::SchedulerError::Execution(source))) => {
                        Err(ExecuteOwnedFederateError::EnclaveExecution { enclave, source })
                    }
                    Ok(Err(crate::sched::SchedulerError::Coordination(source))) => {
                        Err(ExecuteOwnedFederateError::EnclaveCoordination { enclave, source })
                    }
                    Err(payload) => Err(ExecuteOwnedFederateError::ThreadPanicked {
                        enclave,
                        message: panic_message(payload),
                    }),
                };
                let _ = result_tx.send((enclave, result));
            });
            handles.push((enclave, handle));
        }
        drop(result_tx);

        let mut results = TinySecondaryMap::with_capacity(enclave_count);
        let mut failure = None;
        for _ in 0..enclave_count {
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
        if let Some(handle) = coordinator_handle {
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
