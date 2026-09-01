//! Owned payload bindings and mutable storage for compiled scheduler images.

use std::{fmt, marker::PhantomData, ptr::NonNull, time::Instant};

use tinymap::{TinyMap, TinySecondaryMap};

use crate::{
    action::{Action, ActionKey, BaseAction},
    image::{
        ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex, EnclaveImageView,
        EnclaveIndex, PortIndex, ReactionIndex, ReactorIndex, StateSlotIndex, TimingDomain,
    },
    port::{BasePort, Port, PortKey},
    CompiledModeEffectRef, Context, Duration, EnclaveKey, PayloadType, ReactionRefs, ReactorData,
    Refs, RefsMut, Tag, TriggerRes,
};

/// Errors returned by direct reaction implementations.
pub type ReactionBindingError = crate::ReactionRefsError;

/// Validated state-binding and action-image maps keyed by their storage slots.
type StorageLayout = (
    TinySecondaryMap<StateSlotIndex, BindingSlotIndex>,
    TinySecondaryMap<ActionSlotIndex, crate::image::ActionImage>,
);
/// Initialized reactor contexts and their paired event and shutdown channels.
type InitializedContexts = (
    TinyMap<ReactorIndex, Context>,
    crate::Sender<crate::event::AsyncEvent>,
    crate::Receiver<crate::event::AsyncEvent>,
    crate::keepalive::Sender,
);
/// Heap-backed factories and invokers bound directly to compiled-image slots.
#[derive(Default)]
pub struct OwnedBindings {
    /// Typed state initializers, reaction invokers, and port/action factories by required slot.
    bindings: TinySecondaryMap<BindingSlotIndex, Binding>,
    /// Repeated caller-supplied slots retained for pre-initialization duplicate validation.
    duplicate_slots: TinySecondaryMap<BindingSlotIndex, ()>,
}

impl OwnedBindings {
    /// Creates an empty set of direct bindings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds a state initializer to its compiled required binding slot.
    pub fn bind_state<T: ReactorData>(mut self, slot: BindingSlotIndex, init: fn() -> T) -> Self {
        self.insert_binding(
            slot,
            Binding::State(Box::new(TypedStateInitializer::<T> {
                init,
                marker: PhantomData,
            })),
        );
        self
    }
    /// Binds a payload action type to its stable required binding slot.
    pub fn bind_action<T: ReactorData>(
        mut self,
        slot: BindingSlotIndex,
        payload: PayloadType<T>,
    ) -> Self {
        let _ = payload;
        self.insert_binding(
            slot,
            Binding::Action(Box::new(TypedActionFactory::<T> {
                marker: PhantomData,
            })),
        );
        self
    }
    /// Binds a payload port type to its stable required binding slot.
    pub fn bind_port<T: ReactorData>(
        mut self,
        slot: BindingSlotIndex,
        payload: PayloadType<T>,
    ) -> Self {
        let _ = payload;
        self.insert_binding(
            slot,
            Binding::Port(Box::new(TypedPortFactory::<T> {
                marker: PhantomData,
            })),
        );
        self
    }
    /// Binds a generated reaction and its optional image-owned mode effect to a required slot.
    pub fn bind_reaction<F>(mut self, slot: BindingSlotIndex, function: F) -> Self
    where
        F: for<'store> FnMut(
                &mut Context,
                &mut dyn ReactorData,
                ReactionRefs<'store>,
                Option<CompiledModeEffectRef>,
            ) -> Result<(), ReactionBindingError>
            + Send
            + Sync
            + 'static,
    {
        self.insert_binding(
            slot,
            Binding::Reaction(Box::new(ErasedReactionInvoker(function))),
        );
        self
    }

    /// Records one typed binding while retaining duplicate-slot evidence for validation.
    fn insert_binding(&mut self, slot: BindingSlotIndex, binding: Binding) {
        if self.bindings.insert(slot, binding).is_some() {
            self.duplicate_slots.insert(slot, ());
        }
    }

    /// Returns the concrete payload type bound to one compiled port slot.
    pub(crate) fn port_payload_type(&self, slot: BindingSlotIndex) -> Option<&'static str> {
        match self.bindings.get(slot) {
            Some(Binding::Port(factory)) => Some(factory.type_name()),
            _ => None,
        }
    }
}

/// A heterogeneous implementation value occupying a required image binding slot.
enum Binding {
    /// A factory for one reactor's concrete state value.
    State(Box<dyn StateInitializer>),
    /// An invoker for one generated reaction implementation.
    Reaction(Box<dyn ReactionInvoker>),
    /// A factory for one concrete port payload type.
    Port(Box<dyn PortFactory>),
    /// A factory for one concrete action payload type.
    Action(Box<dyn ActionFactory>),
}

impl Binding {
    /// Returns the image binding kind represented by this value.
    fn kind(&self) -> BindingKind {
        match self {
            Self::State(_) => BindingKind::StateInitializer,
            Self::Reaction(_) => BindingKind::Reaction,
            Self::Port(_) => BindingKind::Port,
            Self::Action(_) => BindingKind::Action,
        }
    }
}

/// Object-safe state construction behind the public generic binding method.
trait StateInitializer: Send + Sync {
    /// Builds a fresh dynamically typed reactor state.
    fn initialize(&self) -> StoredState;
}

/// A dynamically stored state value retaining its concrete type for checked diagnostics.
pub(crate) struct StoredState {
    /// The heterogeneous reactor state payload.
    pub(crate) value: Box<dyn ReactorData>,
    /// The payload's concrete Rust type name captured before type erasure.
    pub(crate) type_name: &'static str,
}

/// A concrete function-pointer state initializer.
struct TypedStateInitializer<T: ReactorData> {
    /// The caller-supplied initializer function.
    init: fn() -> T,
    /// Retains the erased concrete state type.
    marker: PhantomData<fn() -> T>,
}

impl<T: ReactorData> StateInitializer for TypedStateInitializer<T> {
    /// Builds the concrete state and erases only its payload type.
    fn initialize(&self) -> StoredState {
        StoredState {
            value: Box::new((self.init)()),
            type_name: std::any::type_name::<T>(),
        }
    }
}

/// Object-safe action construction behind the public generic binding method.
trait ActionFactory: Send + Sync {
    /// Builds a standard action for the supplied slot and timing domain.
    fn create(
        &self,
        slot: ActionSlotIndex,
        domain: TimingDomain,
        min_delay_nanos: u64,
    ) -> Result<Box<dyn BaseAction>, OwnedStorageError>;
}

/// A concrete payload action factory.
struct TypedActionFactory<T: ReactorData> {
    /// Retains the erased concrete payload type.
    marker: PhantomData<fn() -> T>,
}

impl<T: ReactorData> ActionFactory for TypedActionFactory<T> {
    /// Builds a typed standard action using validated compiled timing.
    fn create(
        &self,
        slot: ActionSlotIndex,
        domain: TimingDomain,
        min_delay_nanos: u64,
    ) -> Result<Box<dyn BaseAction>, OwnedStorageError> {
        let min_delay = Duration::nanoseconds(
            i64::try_from(min_delay_nanos)
                .map_err(|_| OwnedStorageError::DelayOutOfRange { min_delay_nanos })?,
        );
        Ok(Action::<T>::new(
            &format!("action-{}", slot.as_u32()),
            ActionKey::new(slot.as_u32()),
            Some(min_delay),
            matches!(domain, TimingDomain::Logical),
        )
        .boxed())
    }
}

/// Object-safe port construction behind the public generic binding method.
trait PortFactory: Send + Sync {
    /// Builds a port for the supplied compiled slot.
    fn create(&self, slot: PortIndex) -> Box<dyn BasePort>;
    /// Returns the concrete payload type produced by this factory.
    fn type_name(&self) -> &'static str;
}

/// A concrete payload port factory.
struct TypedPortFactory<T: ReactorData> {
    /// Retains the erased concrete payload type.
    marker: PhantomData<fn() -> T>,
}

impl<T: ReactorData> PortFactory for TypedPortFactory<T> {
    /// Builds a typed port identified by its compiled slot.
    fn create(&self, slot: PortIndex) -> Box<dyn BasePort> {
        Port::<T>::new(
            &format!("port-{}", slot.as_u32()),
            PortKey::new(slot.as_u32()),
        )
        .boxed()
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }
}

/// Type-erased outbound route installed only after both typed endpoints pass preflight.
trait OutboundRoute: Send {
    /// Clones and admits one present source value at its destination timing boundary.
    fn emit(&mut self, source: &dyn BasePort, tag: Tag) -> Result<(), OwnedStorageError>;
}

/// Direct typed outbound route whose generic parameter is unified by `bind_route`.
struct TypedOutboundRoute<T: ReactorData + Clone> {
    /// Stable boundary identity used in route diagnostics.
    boundary: String,
    /// Canonical destination Enclave identity.
    destination: EnclaveIndex,
    /// Dense destination port admitted by the paired inbound route.
    destination_port: PortIndex,
    /// Compiled logical or physical timing interpretation.
    timing_domain: TimingDomain,
    /// Compiled non-negative delay applied exactly once during emission.
    delay_nanos: u64,
    /// Destination scheduler event channel.
    destination_tx: crate::Sender<crate::event::AsyncEvent>,
    /// Retains the statically unified endpoint payload type.
    marker: PhantomData<fn() -> T>,
}

impl<T: ReactorData + Clone> OutboundRoute for TypedOutboundRoute<T> {
    fn emit(&mut self, source: &dyn BasePort, tag: Tag) -> Result<(), OwnedStorageError> {
        let typed = source.downcast_ref::<Port<T>>().ok_or_else(|| {
            OwnedStorageError::OutboundRoutePayloadTypeMismatch {
                boundary: self.boundary.clone(),
                port: source.get_key(),
                expected: std::any::type_name::<T>(),
                found: source.type_name(),
            }
        })?;
        let Some(value) = typed.get().as_ref() else {
            return Ok(());
        };
        let target = crate::event::AsyncEventTarget::BoundaryPort(PortKey::new(
            self.destination_port.as_u32(),
        ));
        let event = match self.timing_domain {
            TimingDomain::Logical => {
                let tag = if self.delay_nanos == 0 {
                    tag
                } else {
                    tag.checked_delay(Duration::nanoseconds(self.delay_nanos as i64))
                        .ok_or_else(|| OwnedStorageError::OutboundRouteTagOverflow {
                            boundary: self.boundary.clone(),
                            tag,
                            delay_nanos: self.delay_nanos,
                        })?
                };
                crate::event::AsyncEvent::Logical {
                    tag,
                    target,
                    value: Box::new(value.clone()),
                }
            }
            TimingDomain::Physical => {
                let time = Instant::now()
                    .checked_add(std::time::Duration::from_nanos(self.delay_nanos))
                    .ok_or_else(|| OwnedStorageError::OutboundRouteTimeOverflow {
                        boundary: self.boundary.clone(),
                        delay_nanos: self.delay_nanos,
                    })?;
                crate::event::AsyncEvent::Physical {
                    time,
                    target,
                    value: Box::new(value.clone()),
                }
            }
        };
        match self.destination_tx.try_send(event) {
            Ok(true) => Ok(()),
            Ok(false) => Err(OwnedStorageError::OutboundRouteChannelFull {
                boundary: self.boundary.clone(),
                destination: self.destination,
            }),
            Err(_) => Err(OwnedStorageError::OutboundRouteChannelClosed {
                boundary: self.boundary.clone(),
                destination: self.destination,
            }),
        }
    }
}

/// Object-safe direct invocation of a generated reaction implementation.
trait ReactionInvoker: Send + Sync {
    /// Invokes the bound reaction with its exact state and references.
    fn invoke(
        &mut self,
        context: &mut Context,
        state: &mut dyn ReactorData,
        refs: ReactionRefs<'_>,
        mode_effect: Option<CompiledModeEffectRef>,
    ) -> Result<(), ReactionBindingError>;
}

/// Type-erasing adapter for the single owned-reaction callback shape.
struct ErasedReactionInvoker<F>(F);

impl<F> ReactionInvoker for ErasedReactionInvoker<F>
where
    F: for<'store> FnMut(
            &mut Context,
            &mut dyn ReactorData,
            ReactionRefs<'store>,
            Option<CompiledModeEffectRef>,
        ) -> Result<(), ReactionBindingError>
        + Send
        + Sync
        + 'static,
{
    /// Forwards to the caller-supplied generated reaction function.
    fn invoke(
        &mut self,
        context: &mut Context,
        state: &mut dyn ReactorData,
        refs: ReactionRefs<'_>,
        mode_effect: Option<CompiledModeEffectRef>,
    ) -> Result<(), ReactionBindingError> {
        (self.0)(context, state, refs, mode_effect)
    }
}

/// Errors building or accessing heap-backed compiled-image storage.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum OwnedStorageError {
    /// A required image binding did not receive an implementation.
    #[error("missing {kind:?} binding at {slot}")]
    MissingBinding {
        /// The required compiled binding slot.
        slot: BindingSlotIndex,
        /// The implementation kind required by the image.
        kind: BindingKind,
    },
    /// A binding was supplied at the correct slot with the wrong kind.
    #[error("binding kind mismatch at {slot}: expected {expected:?}, found {found:?}")]
    BindingKindMismatch {
        /// The compiled binding slot.
        slot: BindingSlotIndex,
        /// The image-required binding kind.
        expected: BindingKind,
        /// The caller-supplied binding kind.
        found: BindingKind,
    },
    /// A stable binding slot was supplied more than once.
    #[error("duplicate binding at {slot}")]
    DuplicateBinding {
        /// The duplicated stable binding slot.
        slot: BindingSlotIndex,
    },
    /// The set of required bindings does not have exact one-to-one coverage.
    #[error("required binding coverage mismatch: expected {expected}, found {found}")]
    BindingCoverageMismatch {
        /// Number of required binding slots in the image.
        expected: usize,
        /// Number of supplied state or reaction bindings.
        found: usize,
    },
    /// A required action storage slot had no payload factory.
    #[error("missing action factory at {slot}")]
    MissingActionFactory {
        /// The compiled action storage slot.
        slot: ActionSlotIndex,
    },
    /// An action factory was supplied for an executor-owned timer or shutdown action.
    #[error("unexpected action factory at executor-owned slot {slot}")]
    UnexpectedActionFactory {
        /// The compiled timer or shutdown action storage slot.
        slot: ActionSlotIndex,
    },
    /// Multiple image actions claim one mutable action storage slot.
    #[error("duplicate action storage slot {slot}")]
    DuplicateActionSlot {
        /// The duplicated compiled action storage slot.
        slot: ActionSlotIndex,
    },
    /// An action storage slot required by the image was absent.
    #[error("missing action image for storage slot {slot}")]
    MissingActionSlot {
        /// The absent compiled action storage slot.
        slot: ActionSlotIndex,
    },
    /// A required port slot had no payload factory.
    #[error("missing port factory at {slot}")]
    MissingPortFactory {
        /// The compiled port slot.
        slot: PortIndex,
    },
    /// The set of port factories has incorrect coverage.
    #[error("port factory coverage mismatch: expected {expected}, found {found}")]
    PortFactoryCoverageMismatch {
        /// Number of port slots in the image.
        expected: usize,
        /// Number of supplied port factories.
        found: usize,
    },
    /// Multiple image reactors claim one mutable state slot.
    #[error("duplicate state storage slot {slot}")]
    DuplicateStateSlot {
        /// The duplicated compiled state storage slot.
        slot: StateSlotIndex,
    },
    /// A state storage slot required by the image was absent.
    #[error("missing state image for storage slot {slot}")]
    MissingStateSlot {
        /// The absent compiled state storage slot.
        slot: StateSlotIndex,
    },
    /// An action delay could not be represented by the runtime duration type.
    #[error("action minimum delay {min_delay_nanos}ns exceeds the runtime duration range")]
    DelayOutOfRange {
        /// The compiled minimum delay in nanoseconds.
        min_delay_nanos: u64,
    },
    /// A periodic timer cannot make logical progress with a zero period.
    #[error("periodic timer at action slot {slot} has a zero period")]
    ZeroPeriodTimer {
        /// Compiled timer action storage slot.
        slot: ActionSlotIndex,
    },
    /// A periodic timer period exceeds the runtime duration representation.
    #[error("periodic timer at action slot {slot} has unrepresentable period {period_nanos}ns")]
    TimerPeriodOutOfRange {
        /// Compiled timer action storage slot.
        slot: ActionSlotIndex,
        /// Unrepresentable period in nanoseconds.
        period_nanos: u64,
    },
    /// A periodic timer's first successor exceeds the runtime logical tag range.
    #[error("periodic timer at action slot {slot} overflows after startup {startup_nanos}ns plus period {period_nanos}ns")]
    PeriodicTimerTagOverflow {
        /// Compiled timer action storage slot.
        slot: ActionSlotIndex,
        /// Initial logical timer tag in nanoseconds.
        startup_nanos: u64,
        /// Positive recurrence period in nanoseconds.
        period_nanos: u64,
    },
    /// A reaction enables modes other than its statically owning scope's mode.
    #[error("reaction {reaction} has an enabled-mode filter that does not match its static scope")]
    ReactionModeFilterMismatch {
        /// The compiled reaction whose filter requires scheduler support not present in owned execution.
        reaction: ReactionIndex,
    },
    /// A compiled startup delay exceeds the runtime duration representation.
    #[error("compiled startup delay {delay_nanos}ns cannot fit the runtime duration")]
    StartupDelayOutOfRange {
        /// The compiled logical startup delay in nanoseconds.
        delay_nanos: u64,
    },
    /// A direct reaction requested a dynamic mode transition without a stable compiled mode identity.
    #[error("reaction {reaction} requested an unsupported dynamic mode transition")]
    LegacyModeTransition {
        /// The reaction awaiting generated compiled-mode identities.
        reaction: ReactionIndex,
    },
    /// A reaction requested a canonical mode transition other than its image-declared effect.
    #[error("reaction {reaction} requested a compiled mode transition that does not match its declared effect")]
    CompiledModeTransitionMismatch {
        /// The reaction that attempted to forge or substitute a transition capability.
        reaction: ReactionIndex,
        /// Canonical effect declared by the validated image, if any.
        declared: Option<CompiledModeEffectRef>,
        /// Canonical effect requested by the bound reaction implementation.
        requested: CompiledModeEffectRef,
    },
    /// A reaction's port or action references alias mutably within one invocation.
    #[error("reaction {reaction} has aliased mutable references")]
    AliasedReactionReferences {
        /// The reaction with conflicting reference slots.
        reaction: ReactionIndex,
    },
    /// A reaction's required invoker is absent from storage.
    #[error("reaction binding at {slot} is missing")]
    MissingReactionInvoker {
        /// The compiled reaction binding slot.
        slot: BindingSlotIndex,
    },
    /// A directly bound reaction returned a reference-extraction error.
    #[error(transparent)]
    Reaction(#[from] ReactionBindingError),
    /// An async event named a port outside this compiled Enclave image.
    #[error("boundary port key {key} is not present in the compiled image")]
    BoundaryPortNotFound {
        /// Runtime-facing port key supplied by the boundary event.
        key: PortKey,
    },
    /// An async event named an ordinary port without an inbound compiled route.
    #[error("compiled port {port} is not authorized by an inbound scheduler route")]
    BoundaryPortNotInbound {
        /// Dense compiled port lacking inbound route provenance.
        port: PortIndex,
    },
    /// An async boundary payload did not match the compiled port binding type.
    #[error("boundary port {port} requires payload type {expected}")]
    BoundaryPortPayloadTypeMismatch {
        /// Dense compiled boundary port target.
        port: PortIndex,
        /// Concrete payload type required by its direct binding.
        expected: &'static str,
    },
    /// A typed outbound adapter did not match its compiled source port factory.
    #[error("route '{boundary}' source port {port} requires {expected}, found {found}")]
    OutboundRoutePayloadTypeMismatch {
        /// Stable boundary identity.
        boundary: String,
        /// Runtime source port identity.
        port: PortKey,
        /// Payload type selected by the typed route binding.
        expected: &'static str,
        /// Payload type produced by the source port binding.
        found: &'static str,
    },
    /// Applying a logical route delay exceeded the runtime tag range.
    #[error("route '{boundary}' overflows logical tag {tag} with delay {delay_nanos}ns")]
    OutboundRouteTagOverflow {
        /// Stable boundary identity.
        boundary: String,
        /// Source logical tag.
        tag: Tag,
        /// Compiled route delay.
        delay_nanos: u64,
    },
    /// Applying a physical route delay exceeded the platform instant range.
    #[error("route '{boundary}' overflows physical time with delay {delay_nanos}ns")]
    OutboundRouteTimeOverflow {
        /// Stable boundary identity.
        boundary: String,
        /// Compiled route delay.
        delay_nanos: u64,
    },
    /// The destination scheduler closed before it admitted an outbound value.
    #[error("route '{boundary}' destination Enclave {destination} is closed")]
    OutboundRouteChannelClosed {
        /// Stable boundary identity.
        boundary: String,
        /// Canonical destination Enclave index.
        destination: EnclaveIndex,
    },
    /// The bounded destination scheduler mailbox could not immediately admit a routed value.
    #[error("route '{boundary}' destination Enclave {destination} mailbox is full")]
    OutboundRouteChannelFull {
        /// Stable boundary identity.
        boundary: String,
        /// Canonical destination Enclave index.
        destination: EnclaveIndex,
    },
}

/// Mutable, heap-backed storage for one validated compiled enclave image.
pub struct OwnedStorage<'image> {
    /// The validated immutable image that defines every dense storage domain.
    image: EnclaveImageView<'image>,
    /// Concrete reactor payloads keyed by exact image state slots.
    states: TinyMap<StateSlotIndex, StoredState>,
    /// Concrete actions keyed by exact image action storage slots.
    actions: TinyMap<ActionSlotIndex, Box<dyn BaseAction>>,
    /// Concrete ports keyed by exact image port slots.
    ports: TinyMap<PortIndex, Box<dyn BasePort>>,
    /// Per-reactor contexts keyed by exact image reactor indices.
    contexts: TinyMap<ReactorIndex, Context>,
    /// Generated reaction invokers keyed by exact image binding slots.
    reactions: TinySecondaryMap<BindingSlotIndex, Box<dyn ReactionInvoker>>,
    /// Alias-checked references to stable boxed targets.
    reaction_refs: TinyMap<ReactionIndex, ReactionReferenceLayout>,
    /// Boundary payloads retained until their declared logical tag is processed.
    pending_boundary_values: Vec<(Tag, PortIndex, Box<dyn ReactorData>)>,
    /// Dense ports admitted by one or more inbound image routes.
    inbound_boundary_ports: TinySecondaryMap<PortIndex, ()>,
    /// Typed outbound route adapters grouped by source port.
    outbound_routes: TinySecondaryMap<PortIndex, Vec<Box<dyn OutboundRoute>>>,
    /// Ports already emitted during the current processing tag.
    emitted_outbound_ports: TinySecondaryMap<PortIndex, ()>,
    /// Keeps a sender for executor-owned route and shutdown admission.
    event_tx: crate::Sender<crate::event::AsyncEvent>,
    /// Executor-supplied shared origin retained across scheduler attachment.
    configured_origin: Option<Instant>,
    /// Keeps the per-enclave event channel open for stored reaction contexts.
    event_rx: crate::Receiver<crate::event::AsyncEvent>,
    /// Holds the keepalive sender until a compiled scheduler takes responsibility for shutdown.
    shutdown_tx: Option<crate::keepalive::Sender>,
}

/// Preallocated typed-erased reference pointers for one compiled reaction invocation.
struct ReactionReferenceLayout {
    /// Ordered immutable port pointers.
    use_ports: Vec<NonNull<dyn BasePort>>,
    /// Ordered mutable port pointers.
    effect_ports: Vec<NonNull<dyn BasePort>>,
    /// Ordered mutable action pointers.
    actions: Vec<NonNull<dyn BaseAction>>,
}

// SAFETY: `BasePort` and `BaseAction` are `Send`. These pointers target boxes owned by the same
// `OwnedStorage`; moves preserve their allocations, and the boxes are never replaced. Construction
// rejects mutable aliases, and invocation requires exclusive access to the storage.
unsafe impl Send for ReactionReferenceLayout {}

impl fmt::Debug for OwnedStorage<'_> {
    /// Formats structural storage information without requiring erased payloads to be debuggable.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedStorage")
            .field("states", &self.states.len())
            .field("actions", &self.actions.len())
            .field("ports", &self.ports.len())
            .field("contexts", &self.contexts.len())
            .field("reactions", &self.reactions.len())
            .finish_non_exhaustive()
    }
}

/// Defines direct typed scheduler-storage adapters without repetitive forwarding boilerplate.
macro_rules! owned_scheduler_readers {
    ($(fn $name:ident($($argument:ident: $argument_type:ty),*) -> $return_type:ty |$storage:ident| $body:expr;)*) => {
        $(pub(crate) fn $name(&self $(, $argument: $argument_type)*) -> $return_type { let $storage = self; $body })*
    };
}

/// Defines mutable direct typed scheduler-storage adapters without repetitive forwarding boilerplate.
macro_rules! owned_scheduler_writers {
    ($(fn $name:ident($($argument:ident: $argument_type:ty),*) $(-> $return_type:ty)? |$storage:ident| $body:expr;)*) => {
        $(pub(crate) fn $name(&mut self $(, $argument: $argument_type)*) $(-> $return_type)? { let $storage = self; $body })*
    };
}

impl<'image> OwnedStorage<'image> {
    /// Validates one image and its direct bindings without invoking any initializer.
    pub(crate) fn validate_image_bindings(
        image: &EnclaveImageView<'_>,
        bindings: &OwnedBindings,
    ) -> Result<(), OwnedStorageError> {
        let (_, action_images) = validate_storage_layout(image)?;
        validate_action_timing(&action_images)?;
        validate_bindings(image, bindings)?;
        validate_startup_delays(image)?;
        validate_periodic_timer_startups(image)?;
        validate_reaction_mode_filters(image)?;
        validate_reaction_references(image)?;
        Ok(())
    }

    /// Validates direct bindings and constructs every owned storage collection for `image`.
    pub fn new(
        image: EnclaveImageView<'image>,
        bindings: OwnedBindings,
    ) -> Result<Self, OwnedStorageError> {
        Self::new_for_enclave(image, bindings, EnclaveKey::default())
    }

    /// Constructs owned storage whose reaction and send contexts use `enclave_key`.
    pub(crate) fn new_for_enclave(
        image: EnclaveImageView<'image>,
        bindings: OwnedBindings,
        enclave_key: EnclaveKey,
    ) -> Result<Self, OwnedStorageError> {
        Self::validate_image_bindings(&image, &bindings)?;
        let (state_bindings, action_images) = validate_storage_layout(&image)?;
        let OwnedBindings {
            bindings,
            duplicate_slots: _,
        } = bindings;

        let mut actions = initialize_actions(&action_images, &bindings)?;
        let mut ports = initialize_ports(&image, &bindings)?;
        let reaction_refs = initialize_reaction_refs(&image, &mut ports, &mut actions)?;
        let states = initialize_states(&state_bindings, &bindings)?;
        let reactions = initialize_reactions(bindings);
        let (contexts, event_tx, event_rx, shutdown_tx) = initialize_contexts(&image, enclave_key)?;
        let event_capacity = image.storage_bounds().event_capacity() as usize;
        let inbound_boundary_ports = image
            .routes()
            .values()
            .filter(|route| route.direction() == crate::image::RouteDirection::Inbound)
            .map(|route| (route.local_port(), ()))
            .collect();
        Ok(Self {
            image,
            states,
            actions,
            ports,
            contexts,
            reactions,
            reaction_refs,
            pending_boundary_values: Vec::with_capacity(event_capacity),
            inbound_boundary_ports,
            outbound_routes: TinySecondaryMap::new(),
            emitted_outbound_ports: TinySecondaryMap::new(),
            event_tx,
            configured_origin: None,
            event_rx,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    /// Releases the final owned states after scheduler-owned image data is no longer needed.
    pub(crate) fn into_states(self) -> TinyMap<StateSlotIndex, StoredState> {
        self.states
    }

    /// Clears every compiled port after its reactions have completed for an execution tag.
    pub(crate) fn reset_ports(&mut self) {
        self.ports.values_mut().for_each(|port| port.cleanup());
        self.emitted_outbound_ports = TinySecondaryMap::new();
    }

    /// Initializes every owned reaction context with the scheduler's startup-time origin.
    pub(crate) fn initialize_reaction_context_origins(&mut self, origin: Instant) {
        let origin = self.configured_origin.unwrap_or(origin);
        self.contexts
            .values_mut()
            .for_each(|context| context.start_time = origin);
    }

    /// Applies the optional Federate origin to both the scheduler clock and reaction contexts.
    pub(crate) fn prepare_scheduler_origin(&mut self, origin: &mut Instant) {
        if let Some(configured) = self.configured_origin {
            *origin = configured;
        }
        self.initialize_reaction_context_origins(*origin);
    }

    /// Configures the shared Federate origin before the scheduler takes ownership.
    pub(crate) fn configure_scheduler_origin(&mut self, origin: Instant) {
        self.configured_origin = Some(origin);
        self.initialize_reaction_context_origins(origin);
    }

    /// Returns a thread-safe context for local logical-time coordination with this scheduler.
    pub(crate) fn scheduler_send_context(&self) -> crate::SendContext {
        self.contexts[ReactorIndex::new(0)].make_send_context()
    }

    owned_scheduler_readers! {
        fn scheduler_action(action: ActionKey) -> crate::image::ActionIndex |storage| storage.image.actions().iter().find_map(|(index, image)| (storage.actions[image.storage_slot()].key() == action).then_some(index)).expect("owned action key must belong to the validated compiled image");
        fn scheduler_event_rx() -> crate::Receiver<crate::event::AsyncEvent> |storage| storage.event_rx.clone();
        fn scheduler_set_ports() -> impl Iterator<Item = PortIndex> + '_ |storage| storage.ports.iter().filter_map(|(port, value)| value.is_set().then_some(port));
        fn scheduler_event_tx() -> crate::Sender<crate::event::AsyncEvent> |storage| storage.event_tx.clone();
    }

    /// Installs one outbound route after the Federate executor validates its paired endpoints.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_outbound_route<T: ReactorData + Clone>(
        &mut self,
        source_port: PortIndex,
        boundary: String,
        destination: EnclaveIndex,
        destination_port: PortIndex,
        timing_domain: TimingDomain,
        delay_nanos: u64,
        destination_tx: crate::Sender<crate::event::AsyncEvent>,
    ) {
        let route = Box::new(TypedOutboundRoute::<T> {
            boundary,
            destination,
            destination_port,
            timing_domain,
            delay_nanos,
            destination_tx,
            marker: PhantomData,
        });
        if let Some(routes) = self.outbound_routes.get_mut(source_port) {
            routes.push(route);
        } else {
            self.outbound_routes.insert(source_port, vec![route]);
        }
    }

    /// Emits each present routed port once before its source reaction completes.
    fn emit_outbound_routes(&mut self, tag: Tag) -> Result<(), OwnedStorageError> {
        let ready = self
            .outbound_routes
            .keys()
            .filter(|&port| {
                self.ports[port].is_set() && !self.emitted_outbound_ports.contains_key(port)
            })
            .collect::<Vec<_>>();
        for port in ready {
            let source = self.ports[port].as_ref();
            for route in &mut self.outbound_routes[port] {
                route.emit(source, tag)?;
            }
            self.emitted_outbound_ports.insert(port, ());
        }
        Ok(())
    }

    owned_scheduler_writers! {
        fn scheduler_push_action(action: crate::image::ActionIndex, tag: Tag, value: Box<dyn ReactorData>) |storage| { let slot = storage.image.actions()[action].storage_slot(); storage.actions[slot].push_value(tag, value); };
        fn scheduler_clear_action(action: crate::image::ActionIndex) |storage| { let slot = storage.image.actions()[action].storage_slot(); storage.actions[slot].clear_values(); };
        fn scheduler_reschedule_action(action: crate::image::ActionIndex, from: Tag, to: Tag) |storage| if from != to { let slot = storage.image.actions()[action].storage_slot(); storage.actions[slot].reschedule_value(from, to); };
        fn take_scheduler_shutdown_tx() -> crate::keepalive::Sender |storage| storage.shutdown_tx.take().expect("owned storage can be attached to only one scheduler");
    }

    /// Stages one validated inbound-boundary value and returns its dense compiled port.
    pub(crate) fn stage_inbound_boundary_value(
        &mut self,
        key: PortKey,
        tag: Tag,
        value: Box<dyn ReactorData>,
    ) -> Result<PortIndex, OwnedStorageError> {
        let port = PortIndex::new(key.as_u32());
        self.ports
            .get(port)
            .ok_or(OwnedStorageError::BoundaryPortNotFound { key })?;
        if !self.inbound_boundary_ports.contains_key(port) {
            return Err(OwnedStorageError::BoundaryPortNotInbound { port });
        }
        self.pending_boundary_values.push((tag, port, value));
        Ok(port)
    }

    /// Writes all retained boundary values for `tag` immediately before its reactions execute.
    pub(crate) fn scheduler_commit_boundary_ports(
        &mut self,
        tag: Tag,
    ) -> Result<(), OwnedStorageError> {
        let mut index = 0;
        while index < self.pending_boundary_values.len() {
            if self.pending_boundary_values[index].0 != tag {
                index += 1;
                continue;
            }
            let (_, port, value) = self.pending_boundary_values.remove(index);
            let storage = &mut self.ports[port];
            let expected = storage.type_name();
            storage.set_erased(value).map_err(|_| {
                OwnedStorageError::BoundaryPortPayloadTypeMismatch { port, expected }
            })?;
        }
        Ok(())
    }

    /// Invokes a directly bound reaction using the image's ordered port and action references.
    pub(crate) fn invoke_reaction(
        &mut self,
        reaction: ReactionIndex,
        tag: Tag,
    ) -> Result<(), OwnedStorageError> {
        let reaction_image = self.image.reactions()[reaction];
        let reactor = reaction_image.reactor();
        let state_slot = self.image.reactors()[reactor].state_slot();
        let context = &mut self.contexts[reactor];
        context.reset_for_reaction(tag);
        let state = &mut self.states[state_slot];
        let references = &mut self.reaction_refs[reaction];
        let invoker = self.reactions.get_mut(reaction_image.binding()).ok_or(
            OwnedStorageError::MissingReactionInvoker {
                slot: reaction_image.binding(),
            },
        )?;

        let refs = ReactionRefs {
            ports: Refs::new(&mut references.use_ports),
            ports_mut: RefsMut::new(&mut references.effect_ports),
            actions: RefsMut::new(&mut references.actions),
        };
        invoker.invoke(
            context,
            state.value.as_mut(),
            refs,
            reaction_image.mode_effect(),
        )?;
        if let Some(requested) = context.trigger_res.scheduled_compiled_mode {
            let declared = reaction_image.mode_effect();
            if declared != Some(requested) {
                return Err(OwnedStorageError::CompiledModeTransitionMismatch {
                    reaction,
                    declared,
                    requested,
                });
            }
        }
        if context.trigger_res.scheduled_mode.is_some() {
            return Err(OwnedStorageError::LegacyModeTransition { reaction });
        }
        self.emit_outbound_routes(tag)?;
        Ok(())
    }

    /// Borrows the reusable trigger result from a reaction's owning reactor context.
    pub(crate) fn reaction_trigger_res(&self, reaction: ReactionIndex) -> &TriggerRes {
        let reactor = self.image.reactions()[reaction].reactor();
        &self.contexts[reactor].trigger_res
    }

    /// Copies the borrowed image descriptor so scheduler composition uses this storage's exact image.
    pub(crate) fn scheduler_image(&self) -> EnclaveImageView<'image> {
        copy_borrowed_image_view(&self.image)
    }
}

/// Copies the present borrowed-only image descriptor without requiring a public `Clone` impl.
fn copy_borrowed_image_view<'image>(image: &EnclaveImageView<'image>) -> EnclaveImageView<'image> {
    // SAFETY: `EnclaveImageView` currently contains only Copy borrowed image data and no destructor.
    unsafe { std::ptr::read(image) }
}

/// Validates every required binding before any state initializer can run.
fn validate_bindings(
    image: &EnclaveImageView<'_>,
    bindings: &OwnedBindings,
) -> Result<(), OwnedStorageError> {
    if let Some(slot) = bindings.duplicate_slots.keys().next() {
        return Err(OwnedStorageError::DuplicateBinding { slot });
    }
    for (slot, required) in image.required_bindings().iter() {
        let binding = bindings
            .bindings
            .get(slot)
            .ok_or(OwnedStorageError::MissingBinding {
                slot,
                kind: required.kind(),
            })?;
        let found = binding.kind();
        if found != required.kind() {
            return Err(OwnedStorageError::BindingKindMismatch {
                slot,
                expected: required.kind(),
                found,
            });
        }
    }
    if bindings.bindings.len() != image.required_bindings().len() {
        return Err(OwnedStorageError::BindingCoverageMismatch {
            expected: image.required_bindings().len(),
            found: bindings.bindings.len(),
        });
    }

    let referenced_action_bindings = image
        .actions()
        .values()
        .filter_map(|action| action.binding())
        .collect::<Vec<_>>();
    let has_unreferenced_action_binding = bindings.bindings.iter().any(|(slot, binding)| {
        matches!(binding, Binding::Action(_)) && !referenced_action_bindings.contains(&slot)
    });
    if has_unreferenced_action_binding {
        if let Some(slot) = image
            .actions()
            .values()
            .find_map(|action| action.binding().is_none().then_some(action.storage_slot()))
        {
            return Err(OwnedStorageError::UnexpectedActionFactory { slot });
        }
    }
    Ok(())
}

/// Verifies dense state and action slot coverage without initializing payload values.
fn validate_storage_layout(
    image: &EnclaveImageView<'_>,
) -> Result<StorageLayout, OwnedStorageError> {
    let mut state_bindings = TinySecondaryMap::new();
    for (_, reactor) in image.reactors().iter() {
        let slot = reactor.state_slot();
        if state_bindings
            .insert(slot, reactor.state_binding())
            .is_some()
        {
            return Err(OwnedStorageError::DuplicateStateSlot { slot });
        }
    }
    for raw_slot in 0..image.storage_bounds().state_slots() {
        let slot = StateSlotIndex::new(raw_slot);
        if !state_bindings.contains_key(slot) {
            return Err(OwnedStorageError::MissingStateSlot { slot });
        }
    }

    let mut action_images = TinySecondaryMap::new();
    for (_, action) in image.actions().iter() {
        let slot = action.storage_slot();
        if action_images.insert(slot, *action).is_some() {
            return Err(OwnedStorageError::DuplicateActionSlot { slot });
        }
    }
    for raw_slot in 0..image.storage_bounds().action_slots() {
        let slot = ActionSlotIndex::new(raw_slot);
        if !action_images.contains_key(slot) {
            return Err(OwnedStorageError::MissingActionSlot { slot });
        }
    }
    Ok((state_bindings, action_images))
}

/// Rejects filters that require dynamic enabled-mode evaluation before state initialization.
fn validate_reaction_mode_filters(
    schedule: &EnclaveImageView<'_>,
) -> Result<(), OwnedStorageError> {
    schedule
        .reactions()
        .iter()
        .find(|&(reaction, reaction_image)| {
            let modes = schedule.reaction_modes(reaction);
            !modes.is_empty()
                && (modes.len() != 1
                    || Some(modes[0]) != schedule.scopes()[reaction_image.scope()].mode())
        })
        .map_or(Ok(()), |(reaction, _)| {
            Err(OwnedStorageError::ReactionModeFilterMismatch { reaction })
        })
}

/// Rejects aliasing reference layouts without constructing actions, ports, or user state.
fn validate_reaction_references(image: &EnclaveImageView<'_>) -> Result<(), OwnedStorageError> {
    for (reaction, _) in image.reactions().iter() {
        let action_slots = image
            .reaction_actions(reaction)
            .iter()
            .map(|action| image.actions()[*action].storage_slot());
        ensure_unaliased_references(
            reaction,
            image.reaction_use_ports(reaction),
            image.reaction_effect_ports(reaction),
            action_slots,
        )?;
    }
    Ok(())
}

/// Rejects global and modal startup delays that cannot fit the runtime duration type.
fn validate_startup_delays(image: &EnclaveImageView<'_>) -> Result<(), OwnedStorageError> {
    image
        .startup_actions()
        .iter()
        .chain(image.timer_startup_actions())
        .chain(
            image
                .scopes()
                .keys()
                .flat_map(|scope| image.scope_timer_startups(scope)),
        )
        .try_for_each(|startup| {
            let delay_nanos = startup.logical_delay_nanos();
            i64::try_from(delay_nanos)
                .map(|_| ())
                .map_err(|_| OwnedStorageError::StartupDelayOutOfRange { delay_nanos })
        })
}

/// Initializes the dense state map only after all binding and slot validation succeeds.
fn initialize_states(
    state_bindings: &TinySecondaryMap<StateSlotIndex, BindingSlotIndex>,
    bindings: &TinySecondaryMap<BindingSlotIndex, Binding>,
) -> Result<TinyMap<StateSlotIndex, StoredState>, OwnedStorageError> {
    let mut states = TinyMap::with_capacity(state_bindings.len());
    for (slot, binding_slot) in state_bindings.iter() {
        let binding_slot = *binding_slot;
        let binding = &bindings[binding_slot];
        let Binding::State(initializer) = binding else {
            return Err(OwnedStorageError::BindingKindMismatch {
                slot: binding_slot,
                expected: BindingKind::StateInitializer,
                found: binding.kind(),
            });
        };
        let inserted = states.insert(initializer.initialize());
        debug_assert_eq!(inserted, slot);
    }
    Ok(states)
}

/// Rejects unsupported or unrepresentable action timing before state initialization.
fn validate_action_timing(
    action_images: &TinySecondaryMap<ActionSlotIndex, crate::image::ActionImage>,
) -> Result<(), OwnedStorageError> {
    for (slot, action) in action_images.iter() {
        match action.timing() {
            ActionTiming::Standard {
                min_delay_nanos, ..
            } => i64::try_from(min_delay_nanos)
                .map(|_| ())
                .map_err(|_| OwnedStorageError::DelayOutOfRange { min_delay_nanos })?,
            ActionTiming::Timer {
                period_nanos: Some(0),
            } => return Err(OwnedStorageError::ZeroPeriodTimer { slot }),
            ActionTiming::Timer {
                period_nanos: Some(period_nanos),
            } => {
                i64::try_from(period_nanos)
                    .map_err(|_| OwnedStorageError::TimerPeriodOutOfRange { slot, period_nanos })?;
            }
            ActionTiming::Timer { .. } | ActionTiming::Shutdown => {}
        }
    }
    Ok(())
}

/// Validates each periodic timer's first recurrence before any user state is initialized.
fn validate_periodic_timer_startups(image: &EnclaveImageView<'_>) -> Result<(), OwnedStorageError> {
    image
        .timer_startup_actions()
        .iter()
        .chain(
            image
                .scopes()
                .keys()
                .flat_map(|scope| image.scope_timer_startups(scope)),
        )
        .try_for_each(|startup| {
            let action = image.actions()[startup.action()];
            let ActionTiming::Timer {
                period_nanos: Some(period_nanos),
            } = action.timing()
            else {
                return Ok(());
            };
            let startup_nanos = startup.logical_delay_nanos();
            startup_nanos
                .checked_add(period_nanos)
                .filter(|&successor| successor <= i64::MAX as u64)
                .map(|_| ())
                .ok_or(OwnedStorageError::PeriodicTimerTagOverflow {
                    slot: action.storage_slot(),
                    startup_nanos,
                    period_nanos,
                })
        })
}

/// Initializes standard payload actions and executor-owned timer or shutdown unit actions.
fn initialize_actions(
    action_images: &TinySecondaryMap<ActionSlotIndex, crate::image::ActionImage>,
    bindings: &TinySecondaryMap<BindingSlotIndex, Binding>,
) -> Result<TinyMap<ActionSlotIndex, Box<dyn BaseAction>>, OwnedStorageError> {
    let mut actions = TinyMap::with_capacity(action_images.len());
    for (slot, action) in action_images.iter() {
        let value = match action.timing() {
            ActionTiming::Timer { .. } | ActionTiming::Shutdown => {
                Action::<()>::new("internal", ActionKey::new(slot.as_u32()), None, true).boxed()
            }
            ActionTiming::Standard {
                domain,
                min_delay_nanos,
            } => {
                let binding_slot = action
                    .binding()
                    .expect("validated standard action has a payload binding");
                let Binding::Action(factory) = &bindings[binding_slot] else {
                    unreachable!("validated action binding has the required kind")
                };
                factory.create(slot, domain, min_delay_nanos)?
            }
        };
        let inserted = actions.insert(value);
        debug_assert_eq!(inserted, slot);
    }
    Ok(actions)
}

/// Initializes the dense port map from exact image port slots.
fn initialize_ports(
    image: &EnclaveImageView<'_>,
    bindings: &TinySecondaryMap<BindingSlotIndex, Binding>,
) -> Result<TinyMap<PortIndex, Box<dyn BasePort>>, OwnedStorageError> {
    let mut ports = TinyMap::with_capacity(image.ports().len());
    for (slot, port) in image.ports().iter() {
        let Binding::Port(factory) = &bindings[port.binding()] else {
            unreachable!("validated port binding has the required kind")
        };
        let value = factory.create(slot);
        let inserted = ports.insert(value);
        debug_assert_eq!(inserted, slot);
    }
    Ok(ports)
}

/// Separates validated reaction invokers from state initializers by their exact binding slots.
fn initialize_reactions(
    bindings: TinySecondaryMap<BindingSlotIndex, Binding>,
) -> TinySecondaryMap<BindingSlotIndex, Box<dyn ReactionInvoker>> {
    let mut reactions = TinySecondaryMap::new();
    for (slot, binding) in bindings {
        if let Binding::Reaction(invoker) = binding {
            reactions.insert(slot, invoker);
        }
    }
    reactions
}

/// Precomputes alias-checked reference pointers after their boxed storage targets are initialized.
fn initialize_reaction_refs(
    image: &EnclaveImageView<'_>,
    ports: &mut TinyMap<PortIndex, Box<dyn BasePort>>,
    actions: &mut TinyMap<ActionSlotIndex, Box<dyn BaseAction>>,
) -> Result<TinyMap<ReactionIndex, ReactionReferenceLayout>, OwnedStorageError> {
    let mut references = TinyMap::with_capacity(image.reactions().len());
    for (reaction, _) in image.reactions().iter() {
        let use_ports = image.reaction_use_ports(reaction);
        let effect_ports = image.reaction_effect_ports(reaction);
        let action_slots = image
            .reaction_actions(reaction)
            .iter()
            .map(|action| image.actions()[*action].storage_slot());
        ensure_unaliased_references(reaction, use_ports, effect_ports, action_slots.clone())?;
        let inserted = references.insert(ReactionReferenceLayout {
            use_ports: port_pointers(ports, use_ports)?,
            effect_ports: port_pointers(ports, effect_ports)?,
            actions: action_pointers(actions, action_slots)?,
        });
        debug_assert_eq!(inserted, reaction);
    }
    Ok(references)
}

/// Initializes one context per reactor plus the channels that keep it schedulable.
fn initialize_contexts(
    image: &EnclaveImageView<'_>,
    enclave_key: EnclaveKey,
) -> Result<InitializedContexts, OwnedStorageError> {
    let (event_tx, event_rx) = kanal::bounded(image.storage_bounds().event_capacity() as usize);
    let (shutdown_tx, shutdown_rx) = crate::keepalive::channel();
    let start_time = Instant::now();
    let mut contexts = TinyMap::with_capacity(image.reactors().len());
    for (reactor, reactor_image) in image.reactors().iter() {
        let bank_info = reactor_image.bank().map(|bank| crate::BankInfo {
            idx: bank.index() as usize,
            total: bank.total() as usize,
        });
        let inserted = contexts.insert(Context::new(
            enclave_key,
            start_time,
            bank_info,
            event_tx.clone(),
            shutdown_rx.clone(),
        ));
        debug_assert_eq!(inserted, reactor);
    }
    Ok((contexts, event_tx, event_rx, shutdown_tx))
}

/// Rejects repeated mutable references and immutable/mutable port overlap before pointer creation.
fn ensure_unaliased_references(
    reaction: ReactionIndex,
    use_ports: &[PortIndex],
    effect_ports: &[PortIndex],
    actions: impl IntoIterator<Item = ActionSlotIndex>,
) -> Result<(), OwnedStorageError> {
    let mut used_ports = TinySecondaryMap::<PortIndex, ()>::new();
    for &slot in use_ports {
        if used_ports.insert(slot, ()).is_some() {
            return Err(OwnedStorageError::AliasedReactionReferences { reaction });
        }
    }
    for &slot in effect_ports {
        if used_ports.contains_key(slot) || used_ports.insert(slot, ()).is_some() {
            return Err(OwnedStorageError::AliasedReactionReferences { reaction });
        }
    }
    let mut used_actions = TinySecondaryMap::<ActionSlotIndex, ()>::new();
    for slot in actions {
        if used_actions.insert(slot, ()).is_some() {
            return Err(OwnedStorageError::AliasedReactionReferences { reaction });
        }
    }
    Ok(())
}

/// Converts exact port slots into temporary ordered pointers for reaction reference extraction.
fn port_pointers(
    ports: &mut TinyMap<PortIndex, Box<dyn BasePort>>,
    slots: &[PortIndex],
) -> Result<Vec<NonNull<dyn BasePort>>, OwnedStorageError> {
    slots
        .iter()
        .map(|&slot| Ok(NonNull::from(ports[slot].as_mut())))
        .collect()
}

/// Converts exact action slots into temporary ordered pointers for reaction reference extraction.
fn action_pointers(
    actions: &mut TinyMap<ActionSlotIndex, Box<dyn BaseAction>>,
    slots: impl IntoIterator<Item = ActionSlotIndex>,
) -> Result<Vec<NonNull<dyn BaseAction>>, OwnedStorageError> {
    slots
        .into_iter()
        .map(|slot| Ok(NonNull::from(actions[slot].as_mut())))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{
        image::{
            ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex,
            EnclaveImage, EnclaveImageView, EnclaveIndex, IdentityRange, LevelReactionImage,
            ModeImage, PortImage, PortIndex, ReactionImage, ReactionIndex, ReactorImage,
            ReactorIndex, RequiredBindingImage, RouteDirection, RouteImage, ScopeImage, ScopeIndex,
            StateSlotIndex, StorageBounds, TableRange, TimerStartupImage, TimingDomain,
            TinyMapView,
        },
        AsyncEvent, CommonContext, CompiledModeEffectRef, Config, Context, Duration,
        ModeTransitionRequest, OwnedBindings, OwnedStorage, OwnedStorageError, PayloadType,
        PortKey, ReactionBindingError, ReactionRefs, ReactorData, Tag, TransitionKind,
    };
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::Instant,
    };

    /// State used to prove the checked state accessor rejects a wrong concrete type.
    struct TestState;

    /// Initializes the state required by [`IMAGE`].
    fn initialize_state() -> TestState {
        TestState
    }

    /// A directly bound reaction implementation for [`IMAGE`].
    fn reaction(
        _context: &mut Context,
        _state: &mut dyn ReactorData,
        _refs: ReactionRefs<'_>,
        _mode_effect: Option<CompiledModeEffectRef>,
    ) -> Result<(), ReactionBindingError> {
        Ok(())
    }

    /// Counts invocations to prove failed construction does not initialize state.
    static INITIALIZER_CALLS: AtomicUsize = AtomicUsize::new(0);

    /// Initializes state while recording whether construction reached this side effect.
    fn counted_initializer() -> TestState {
        INITIALIZER_CALLS.fetch_add(1, Ordering::SeqCst);
        TestState
    }

    /// Counts invocations to prove each reaction starts with a fresh trigger result.
    static REACTION_CALLS: AtomicUsize = AtomicUsize::new(0);

    /// Schedules shutdown only during its first invocation.
    fn schedule_shutdown_once(
        context: &mut Context,
        _state: &mut dyn ReactorData,
        refs: ReactionRefs<'_>,
        _mode_effect: Option<CompiledModeEffectRef>,
    ) -> Result<(), ReactionBindingError> {
        let call = REACTION_CALLS.fetch_add(1, Ordering::SeqCst);
        let mut port: crate::OutputRef<u32> = refs.ports_mut.partition_mut()?;
        let mut action: crate::ActionRef<u32> = refs.actions.partition_mut()?;
        *port = Some(call as u32);
        action.set_value(context.get_tag(), call as u32);
        if call == 0 {
            context.schedule_shutdown(Some(Duration::nanoseconds(1)));
        }
        Ok(())
    }

    /// Requests a dynamic mode change that compiled direct execution intentionally defers.
    fn request_dynamic_mode(
        context: &mut Context,
        _state: &mut dyn ReactorData,
        _refs: ReactionRefs<'_>,
        _mode_effect: Option<CompiledModeEffectRef>,
    ) -> Result<(), ReactionBindingError> {
        context.set_mode_transition(ModeTransitionRequest {
            target: crate::ModeKey::new(0),
            transition: TransitionKind::Reset,
        });
        Ok(())
    }

    static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
        BindingSlotIndex::new(0),
        StateSlotIndex::new(0),
        ScopeIndex::new(0),
        TableRange::new(0, 1),
        Some(crate::image::ModeIndex::new(0)),
        None,
    )];
    /// Creates the test action shared by timing-validation fixtures.
    const fn action(timing: ActionTiming) -> ActionImage {
        let binding = match timing {
            ActionTiming::Standard { .. } => Some(BindingSlotIndex::new(3)),
            ActionTiming::Timer { .. } | ActionTiming::Shutdown => None,
        };
        ActionImage::new(
            ScopeIndex::new(0),
            ActionSlotIndex::new(0),
            timing,
            TableRange::new(0, 0),
            binding,
        )
    }
    static ACTIONS: [ActionImage; 1] = [action(ActionTiming::Standard {
        domain: TimingDomain::Logical,
        min_delay_nanos: 0,
    })];
    static UNREPRESENTABLE_ACTIONS: [ActionImage; 1] = [action(ActionTiming::Standard {
        domain: TimingDomain::Logical,
        min_delay_nanos: u64::MAX,
    })];
    static PORTS: [PortImage; 1] = [PortImage::new(
        ScopeIndex::new(0),
        TableRange::new(0, 0),
        BindingSlotIndex::new(2),
    )];
    static REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
        ReactorIndex::new(0),
        ScopeIndex::new(0),
        0,
        BindingSlotIndex::new(1),
        TableRange::new(0, 0),
        TableRange::new(0, 1),
        TableRange::new(0, 1),
        TableRange::new(0, 0),
    )];
    static FILTERED_REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
        ReactorIndex::new(0),
        ScopeIndex::new(0),
        0,
        BindingSlotIndex::new(1),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 1),
    )];
    static MODES: [ModeImage; 1] = [ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(1))];
    static SCOPES: [ScopeImage; 2] = [
        ScopeImage::new(
            None,
            ReactorIndex::new(0),
            None,
            TableRange::new(0, 2),
            TableRange::new(0, 1),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
        ),
        ScopeImage::new(
            Some(ScopeIndex::new(0)),
            ReactorIndex::new(0),
            Some(crate::image::ModeIndex::new(0)),
            TableRange::new(2, 1),
            TableRange::new(1, 0),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
        ),
    ];
    static SCOPE_DESCENDANTS: [ScopeIndex; 3] =
        [ScopeIndex::new(0), ScopeIndex::new(1), ScopeIndex::new(1)];
    static FILTERED_REACTION_MODES: [crate::image::ModeIndex; 1] =
        [crate::image::ModeIndex::new(0)];
    static SCOPE_LOGICAL_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
    static REQUIRED_BINDINGS: [RequiredBindingImage; 4] = [
        RequiredBindingImage::new(IdentityRange::new(7, 7), BindingKind::StateInitializer),
        RequiredBindingImage::new(IdentityRange::new(14, 10), BindingKind::Reaction),
        RequiredBindingImage::new(IdentityRange::new(24, 6), BindingKind::Port),
        RequiredBindingImage::new(IdentityRange::new(30, 8), BindingKind::Action),
    ];
    static IMAGE: EnclaveImage<'static> = EnclaveImage {
        identity_data: "enclavea-stateb-reactionc-portd-action",
        enclave_id: IdentityRange::new(0, 7),
        reactors: TinyMapView::new(&REACTORS),
        actions: TinyMapView::new(&ACTIONS),
        ports: TinyMapView::new(&PORTS),
        reactions: TinyMapView::new(&REACTIONS),
        modes: TinyMapView::new(&MODES),
        scopes: TinyMapView::new(&SCOPES),
        reaction_triggers: &[],
        reaction_use_ports: &[],
        reaction_effect_ports: &[PortIndex::new(0)],
        reaction_actions: &[ActionIndex::new(0)],
        reaction_modes: &[],
        scope_descendants: &SCOPE_DESCENDANTS,
        scope_logical_actions: &SCOPE_LOGICAL_ACTIONS,
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
        storage_bounds: StorageBounds::new(1, 1, 1, 0, 0, 0),
    };
    static UNREPRESENTABLE_ACTION_IMAGE: EnclaveImage<'static> = EnclaveImage {
        actions: TinyMapView::new(&UNREPRESENTABLE_ACTIONS),
        ..IMAGE
    };
    static UNREPRESENTABLE_STARTUP_IMAGE: EnclaveImage<'static> = EnclaveImage {
        startup_actions: &[TimerStartupImage::new(ActionIndex::new(0), u64::MAX)],
        ..IMAGE
    };
    static FILTERED_MODE_IMAGE: EnclaveImage<'static> = EnclaveImage {
        reactions: TinyMapView::new(&FILTERED_REACTIONS),
        reaction_modes: &FILTERED_REACTION_MODES,
        ..IMAGE
    };
    static INBOUND_ROUTES: [RouteImage; 2] = [
        RouteImage::new(
            IdentityRange::new(7, 7),
            PortIndex::new(0),
            RouteDirection::Inbound,
            TimingDomain::Logical,
            0,
        ),
        RouteImage::new(
            IdentityRange::new(14, 10),
            PortIndex::new(0),
            RouteDirection::Inbound,
            TimingDomain::Logical,
            0,
        ),
    ];
    static ROUTED_IMAGE: EnclaveImage<'static> = EnclaveImage {
        routes: TinyMapView::new(&INBOUND_ROUTES),
        ..IMAGE
    };

    /// Returns a fresh validated view of the immutable test image.
    fn image() -> EnclaveImageView<'static> {
        EnclaveImageView::new(&IMAGE).expect("test image is valid")
    }

    /// Returns a validated image whose action delay cannot fit the runtime duration type.
    fn unrepresentable_action_image() -> EnclaveImageView<'static> {
        EnclaveImageView::new(&UNREPRESENTABLE_ACTION_IMAGE).expect("test image is valid")
    }

    /// Returns a validated image whose global startup delay cannot fit the runtime duration type.
    fn unrepresentable_startup_image() -> EnclaveImageView<'static> {
        EnclaveImageView::new(&UNREPRESENTABLE_STARTUP_IMAGE).expect("test image is valid")
    }

    /// Returns a valid image with a reaction filter broader than its static scope.
    fn filtered_mode_image() -> EnclaveImageView<'static> {
        EnclaveImageView::new(&FILTERED_MODE_IMAGE).expect("test image is valid")
    }

    fn routed_image() -> EnclaveImageView<'static> {
        EnclaveImageView::new(&ROUTED_IMAGE).expect("test image is valid")
    }

    /// Returns bindings for every non-lifecycle storage slot in [`IMAGE`].
    fn complete_bindings() -> OwnedBindings {
        OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(BindingSlotIndex::new(1), reaction)
            .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
            .bind_action(BindingSlotIndex::new(3), PayloadType::<u32>::new())
    }

    /// Returns bindings whose initializer records that construction reached it.
    fn counted_bindings() -> OwnedBindings {
        OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), counted_initializer)
            .bind_reaction(BindingSlotIndex::new(1), reaction)
            .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
            .bind_action(BindingSlotIndex::new(3), PayloadType::<u32>::new())
    }

    #[test]
    fn missing_required_binding_slot_is_rejected() {
        let error = OwnedStorage::new(image(), OwnedBindings::new()).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::MissingBinding {
                slot,
                ..
            } if slot == BindingSlotIndex::new(0)
        ));
    }

    #[test]
    fn wrong_required_binding_kind_is_rejected() {
        let bindings = OwnedBindings::new().bind_reaction(BindingSlotIndex::new(0), reaction);

        let error = OwnedStorage::new(image(), bindings).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::BindingKindMismatch {
                slot,
                expected: BindingKind::StateInitializer,
                found: BindingKind::Reaction,
            } if slot == BindingSlotIndex::new(0)
        ));
    }

    #[test]
    fn duplicate_required_binding_is_rejected_before_initializing_state() {
        INITIALIZER_CALLS.store(0, Ordering::SeqCst);
        let bindings = counted_bindings().bind_state(BindingSlotIndex::new(0), counted_initializer);

        let error = OwnedStorage::new(image(), bindings).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::DuplicateBinding { slot }
                if slot == BindingSlotIndex::new(0)
        ));
        assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn missing_action_factory_is_rejected() {
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(BindingSlotIndex::new(1), reaction)
            .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new());

        let error = OwnedStorage::new(image(), bindings).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::MissingBinding {
                slot,
                kind: BindingKind::Action,
            } if slot == BindingSlotIndex::new(3)
        ));
    }

    #[test]
    fn missing_port_factory_is_rejected() {
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(BindingSlotIndex::new(1), reaction)
            .bind_action(BindingSlotIndex::new(3), PayloadType::<u32>::new());

        let error = OwnedStorage::new(image(), bindings).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::MissingBinding {
                slot,
                kind: BindingKind::Port,
            } if slot == BindingSlotIndex::new(2)
        ));
    }

    #[test]
    fn rejects_unrepresentable_action_delay_before_initializing_state() {
        INITIALIZER_CALLS.store(0, Ordering::SeqCst);
        let error =
            OwnedStorage::new(unrepresentable_action_image(), counted_bindings()).unwrap_err();

        assert!(matches!(error, OwnedStorageError::DelayOutOfRange { .. }));
        assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rejects_zero_period_timer_before_initializing_state() {
        let actions = [action(ActionTiming::Timer {
            period_nanos: Some(0),
        })];
        let periodic_image = EnclaveImage {
            actions: TinyMapView::new(&actions),
            ..IMAGE
        };
        let image = EnclaveImageView::new(&periodic_image).expect("periodic image is structural");
        INITIALIZER_CALLS.store(0, Ordering::SeqCst);

        let error = OwnedStorage::new(image, counted_bindings()).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::ZeroPeriodTimer { slot }
                if slot == ActionSlotIndex::new(0)
        ));
        assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rejects_unrepresentable_timer_period_before_initializing_state() {
        let period_nanos = i64::MAX as u64 + 1;
        let actions = [action(ActionTiming::Timer {
            period_nanos: Some(period_nanos),
        })];
        let periodic_image = EnclaveImage {
            actions: TinyMapView::new(&actions),
            ..IMAGE
        };
        let image = EnclaveImageView::new(&periodic_image).expect("periodic image is structural");
        INITIALIZER_CALLS.store(0, Ordering::SeqCst);

        let error = OwnedStorage::new(image, counted_bindings()).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::TimerPeriodOutOfRange { slot, period_nanos: found }
                if slot == ActionSlotIndex::new(0) && found == period_nanos
        ));
        assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rejects_payload_factory_for_executor_owned_timer() {
        let actions = [action(ActionTiming::Timer { period_nanos: None })];
        let timer_image = EnclaveImage {
            actions: TinyMapView::new(&actions),
            ..IMAGE
        };
        let image = EnclaveImageView::new(&timer_image).expect("timer image is structural");

        let error = OwnedStorage::new(image, complete_bindings()).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::UnexpectedActionFactory { slot }
                if slot == ActionSlotIndex::new(0)
        ));
    }

    #[test]
    fn rejects_unrepresentable_startup_delay_before_initializing_state() {
        INITIALIZER_CALLS.store(0, Ordering::SeqCst);
        let error =
            OwnedStorage::new(unrepresentable_startup_image(), counted_bindings()).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::StartupDelayOutOfRange {
                delay_nanos: u64::MAX
            }
        ));
        assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rejects_reaction_mode_filter_that_does_not_match_scope_before_initializing_state() {
        INITIALIZER_CALLS.store(0, Ordering::SeqCst);
        let error = OwnedStorage::new(filtered_mode_image(), counted_bindings()).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::ReactionModeFilterMismatch { reaction }
                if reaction == ReactionIndex::new(0)
        ));
        assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rejects_aliased_reaction_references_before_initializing_state() {
        let duplicate_actions = [ActionIndex::new(0); 2];
        let reactions = [ReactionImage::new(
            ReactorIndex::new(0),
            ScopeIndex::new(0),
            0,
            BindingSlotIndex::new(1),
            TableRange::new(0, 0),
            TableRange::new(0, 0),
            TableRange::new(0, 2),
            TableRange::new(0, 0),
        )];
        let aliased_image = EnclaveImage {
            reactions: TinyMapView::new(&reactions),
            reaction_actions: &duplicate_actions,
            ..IMAGE
        };
        let image = EnclaveImageView::new(&aliased_image).expect("aliased image is structural");
        INITIALIZER_CALLS.store(0, Ordering::SeqCst);

        let error = OwnedStorage::new(image, counted_bindings()).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::AliasedReactionReferences { .. }
        ));
        assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cached_references_survive_storage_moves_and_reborrows() {
        REACTION_CALLS.store(0, Ordering::SeqCst);
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(BindingSlotIndex::new(1), schedule_shutdown_once)
            .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
            .bind_action(BindingSlotIndex::new(3), PayloadType::<u32>::new());
        let mut storage = Box::new(OwnedStorage::new(image(), bindings).unwrap());
        let tag = Tag::new(Duration::nanoseconds(7), 2);

        storage.invoke_reaction(ReactionIndex::new(0), tag).unwrap();
        assert!(storage
            .reaction_trigger_res(ReactionIndex::new(0))
            .scheduled_shutdown
            .is_some());
        storage.reset_ports();
        storage.scheduler_clear_action(ActionIndex::new(0));
        storage.invoke_reaction(ReactionIndex::new(0), tag).unwrap();
        let second = storage.reaction_trigger_res(ReactionIndex::new(0)).clone();
        assert!(second.scheduled_shutdown.is_none());
    }

    #[test]
    fn rejects_dynamic_mode_transitions_without_compiled_identity() {
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(BindingSlotIndex::new(1), request_dynamic_mode)
            .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
            .bind_action(BindingSlotIndex::new(3), PayloadType::<u32>::new());
        let mut storage = OwnedStorage::new(image(), bindings).unwrap();

        let error = storage
            .invoke_reaction(ReactionIndex::new(0), Tag::ZERO)
            .unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::LegacyModeTransition { reaction }
                if reaction == ReactionIndex::new(0)
        ));
    }

    #[test]
    fn boundary_write_rejects_unknown_port_key() {
        let mut storage = OwnedStorage::new(image(), complete_bindings()).unwrap();
        let error = storage
            .stage_inbound_boundary_value(PortKey::new(7), Tag::ZERO, Box::new(42_u32))
            .unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::BoundaryPortNotFound { key } if key == PortKey::new(7)
        ));
    }

    #[test]
    fn boundary_write_rejects_port_without_inbound_route() {
        let mut storage = OwnedStorage::new(image(), complete_bindings()).unwrap();
        let error = storage
            .stage_inbound_boundary_value(PortKey::new(0), Tag::ZERO, Box::new(42_u32))
            .unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::BoundaryPortNotInbound { port } if port == PortIndex::new(0)
        ));
    }

    #[test]
    fn boundary_write_commits_only_at_declared_tag() {
        let mut storage = OwnedStorage::new(routed_image(), complete_bindings()).unwrap();
        let expected_tag = Tag::new(Duration::nanoseconds(2), 0);
        storage
            .stage_inbound_boundary_value(PortKey::new(0), expected_tag, Box::new(42_u32))
            .unwrap();

        storage.scheduler_commit_boundary_ports(Tag::ZERO).unwrap();
        assert!(!storage.ports[PortIndex::new(0)].is_set());
        storage
            .scheduler_commit_boundary_ports(expected_tag)
            .unwrap();
        assert_eq!(
            storage.ports[PortIndex::new(0)]
                .downcast_ref::<crate::Port<u32>>()
                .unwrap()
                .get(),
            &Some(42)
        );
    }

    #[test]
    fn boundary_write_rejects_wrong_payload_type() {
        let mut storage = OwnedStorage::new(routed_image(), complete_bindings()).unwrap();
        storage
            .stage_inbound_boundary_value(PortKey::new(0), Tag::ZERO, Box::new(42_u64))
            .unwrap();
        let error = storage
            .scheduler_commit_boundary_ports(Tag::ZERO)
            .unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::BoundaryPortPayloadTypeMismatch { port, expected }
                if port == PortIndex::new(0) && expected == std::any::type_name::<u32>()
        ));
    }

    #[test]
    fn outbound_routes_clone_fanout_apply_delays_once_and_do_not_block() {
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(
                BindingSlotIndex::new(1),
                |_context, _state, refs: ReactionRefs<'_>, _mode_effect| {
                    let mut output: crate::OutputRef<String> = refs.ports_mut.partition_mut()?;
                    *output = Some("cloned value".to_owned());
                    Ok(())
                },
            )
            .bind_port(BindingSlotIndex::new(2), PayloadType::<String>::new())
            .bind_action(BindingSlotIndex::new(3), PayloadType::<u32>::new());
        let mut storage = OwnedStorage::new(image(), bindings).unwrap();
        let source_tag = Tag::new(Duration::nanoseconds(7), 3);
        let (logical_tx, logical_rx) = kanal::bounded(1);
        let (physical_tx, physical_rx) = kanal::bounded(1);
        storage.bind_outbound_route::<String>(
            PortIndex::new(0),
            "logical".into(),
            EnclaveIndex::new(1),
            PortIndex::new(0),
            TimingDomain::Logical,
            5,
            logical_tx,
        );
        storage.bind_outbound_route::<String>(
            PortIndex::new(0),
            "physical".into(),
            EnclaveIndex::new(2),
            PortIndex::new(0),
            TimingDomain::Physical,
            1_000_000_000,
            physical_tx,
        );
        let (full_tx, full_rx) = kanal::bounded(1);
        full_tx
            .send(AsyncEvent::Shutdown {
                delay: Duration::ZERO,
            })
            .unwrap();
        storage.bind_outbound_route::<String>(
            PortIndex::new(0),
            "full".into(),
            EnclaveIndex::new(3),
            PortIndex::new(0),
            TimingDomain::Logical,
            0,
            full_tx,
        );
        let before = Instant::now();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            done_tx
                .send(storage.invoke_reaction(ReactionIndex::new(0), source_tag))
                .unwrap();
        });
        let result = done_rx.recv_timeout(std::time::Duration::from_secs(1));
        let after = Instant::now();
        drop(full_rx);
        worker.join().unwrap();
        assert!(matches!(
            result,
            Ok(Err(OwnedStorageError::OutboundRouteChannelFull { destination, .. }))
                if destination == EnclaveIndex::new(3)
        ));

        match logical_rx.try_recv().unwrap().unwrap() {
            AsyncEvent::Logical { tag, value, .. } => {
                assert_eq!(tag, Tag::new(Duration::nanoseconds(12), 0));
                assert_eq!(*value.downcast::<String>().ok().unwrap(), "cloned value");
            }
            event => panic!("unexpected logical route event: {event:?}"),
        }
        match physical_rx.try_recv().unwrap().unwrap() {
            AsyncEvent::Physical { time, value, .. } => {
                assert!(time >= before + std::time::Duration::from_secs(1));
                assert!(time <= after + std::time::Duration::from_secs(1));
                assert_eq!(*value.downcast::<String>().ok().unwrap(), "cloned value");
            }
            event => panic!("unexpected physical route event: {event:?}"),
        }
    }

    #[test]
    fn owned_storage_is_send() {
        fn assert_send<T: Send>() {}

        assert_send::<OwnedStorage<'static>>();
    }

    #[test]
    fn configured_federate_origin_drives_the_paced_scheduler_clock() {
        let actions = [ActionImage::new(
            ScopeIndex::new(0),
            ActionSlotIndex::new(0),
            ActionTiming::Timer { period_nanos: None },
            TableRange::new(0, 1),
            None,
        )];
        let reaction_triggers = [LevelReactionImage::new(0, ReactionIndex::new(0))];
        let startup_actions = [TimerStartupImage::new(ActionIndex::new(0), 1_000_000_000)];
        let required_bindings = [
            REQUIRED_BINDINGS[0],
            REQUIRED_BINDINGS[1],
            REQUIRED_BINDINGS[2],
        ];
        let image = EnclaveImage {
            actions: TinyMapView::new(&actions),
            reaction_triggers: &reaction_triggers,
            startup_actions: &startup_actions,
            required_bindings: TinyMapView::new(&required_bindings),
            ..IMAGE
        };
        let image = EnclaveImageView::new(&image).unwrap();
        let seen_origin = Arc::new(Mutex::new(None));
        let reaction_origin = Arc::clone(&seen_origin);
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(
                BindingSlotIndex::new(1),
                move |context: &mut Context, _state, _refs, _mode_effect| {
                    *reaction_origin.lock().unwrap() = Some(context.get_start_time());
                    context.schedule_shutdown(Some(Duration::ZERO));
                    Ok(())
                },
            )
            .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new());
        let mut storage = OwnedStorage::new(image, bindings).unwrap();
        let origin = Instant::now() - std::time::Duration::from_millis(800);
        storage.configure_scheduler_origin(origin);

        let started = Instant::now();
        crate::sched::run_owned_scheduler(
            &mut storage,
            &Config::default().with_fast_forward(false),
        )
        .unwrap();

        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(*seen_origin.lock().unwrap(), Some(origin));
    }
}
