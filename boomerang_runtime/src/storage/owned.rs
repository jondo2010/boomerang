//! Owned payload bindings and mutable storage for compiled scheduler images.

use std::{fmt, marker::PhantomData, ptr::NonNull, time::Instant};

use tinymap::{TinyMap, TinySecondaryMap};

use crate::{
    action::{Action, ActionKey, BaseAction},
    image::{
        ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex, EnclaveImageView, PortIndex,
        ReactionIndex, ReactorIndex, StateSlotIndex, TimingDomain,
    },
    port::{BasePort, Port, PortKey},
    Context, Duration, EnclaveKey, ReactionRefs, ReactorData, Refs, RefsMut, Tag, TriggerRes,
};

/// Errors returned by direct reaction implementations.
pub type ReactionBindingError = crate::ReactionRefsError;

/// Heap-backed factories and invokers bound directly to compiled-image slots.
pub struct OwnedBindings {
    /// Factories and invokers for the image's typed required binding slots.
    bindings: TinySecondaryMap<BindingSlotIndex, Binding>,
    /// Payload action factories keyed by compiled action storage slot.
    actions: TinySecondaryMap<ActionSlotIndex, Box<dyn ActionFactory>>,
    /// Payload port factories keyed by compiled port slot.
    ports: TinySecondaryMap<PortIndex, Box<dyn PortFactory>>,
}

impl OwnedBindings {
    /// Creates an empty set of direct bindings.
    pub fn new() -> Self {
        Self {
            bindings: TinySecondaryMap::new(),
            actions: TinySecondaryMap::new(),
            ports: TinySecondaryMap::new(),
        }
    }

    /// Binds a state initializer to its compiled required binding slot.
    pub fn bind_state<T: ReactorData>(mut self, slot: BindingSlotIndex, init: fn() -> T) -> Self {
        self.bindings.insert(
            slot,
            Binding::State(Box::new(TypedStateInitializer::<T> {
                init,
                marker: PhantomData,
            })),
        );
        self
    }

    /// Binds a payload action type to its compiled storage slot.
    pub fn bind_action<T: ReactorData>(mut self, slot: ActionSlotIndex) -> Self {
        self.actions.insert(
            slot,
            Box::new(TypedActionFactory::<T> {
                marker: PhantomData,
            }),
        );
        self
    }

    /// Binds a payload port type to its compiled storage slot.
    pub fn bind_port<T: ReactorData>(mut self, slot: PortIndex) -> Self {
        self.ports.insert(
            slot,
            Box::new(TypedPortFactory::<T> {
                marker: PhantomData,
            }),
        );
        self
    }

    /// Binds a directly generated reaction implementation to its required binding slot.
    pub fn bind_reaction<F>(mut self, slot: BindingSlotIndex, function: F) -> Self
    where
        F: for<'store> FnMut(
                &mut Context,
                &mut dyn ReactorData,
                ReactionRefs<'store>,
            ) -> Result<(), ReactionBindingError>
            + Send
            + Sync
            + 'static,
    {
        self.bindings
            .insert(slot, Binding::Reaction(Box::new(function)));
        self
    }
}

impl Default for OwnedBindings {
    /// Creates an empty set of direct bindings.
    fn default() -> Self {
        Self::new()
    }
}

/// The two heterogeneous values that may occupy a required image binding slot.
enum Binding {
    /// A factory for one reactor's concrete state value.
    State(Box<dyn StateInitializer>),
    /// An invoker for one generated reaction implementation.
    Reaction(Box<dyn ReactionInvoker>),
}

impl Binding {
    /// Returns the image binding kind represented by this value.
    fn kind(&self) -> BindingKind {
        match self {
            Self::State(_) => BindingKind::StateInitializer,
            Self::Reaction(_) => BindingKind::Reaction,
        }
    }
}

/// Object-safe state construction behind the public generic binding method.
trait StateInitializer: Send + Sync {
    /// Builds a fresh dynamically typed reactor state.
    fn initialize(&self) -> StoredState;
}

/// A dynamically stored state value retaining its concrete type for checked diagnostics.
struct StoredState {
    /// The heterogeneous reactor state payload.
    value: Box<dyn ReactorData>,
    /// The payload's concrete Rust type name captured before type erasure.
    type_name: &'static str,
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
    /// Builds an action for the supplied compiled slot and timing semantics.
    fn create(
        &self,
        slot: ActionSlotIndex,
        timing: ActionTiming,
    ) -> Result<Box<dyn BaseAction>, OwnedStorageError>;
}

/// A concrete payload action factory.
struct TypedActionFactory<T: ReactorData> {
    /// Retains the erased concrete payload type.
    marker: PhantomData<fn() -> T>,
}

impl<T: ReactorData> ActionFactory for TypedActionFactory<T> {
    /// Builds a typed action using the compiled timing semantics.
    fn create(
        &self,
        slot: ActionSlotIndex,
        timing: ActionTiming,
    ) -> Result<Box<dyn BaseAction>, OwnedStorageError> {
        let (min_delay, is_logical) = match timing {
            ActionTiming::Standard {
                domain,
                min_delay_nanos,
            } => (
                Duration::nanoseconds(
                    i64::try_from(min_delay_nanos)
                        .map_err(|_| OwnedStorageError::DelayOutOfRange { min_delay_nanos })?,
                ),
                matches!(domain, TimingDomain::Logical),
            ),
            ActionTiming::Timer { .. } => (Duration::ZERO, true),
            ActionTiming::Shutdown => {
                return Err(OwnedStorageError::UnexpectedActionFactory { slot })
            }
        };

        Ok(Action::<T>::new(
            &format!("action-{}", slot.as_u32()),
            ActionKey::new(slot.as_u32()),
            Some(min_delay),
            is_logical,
        )
        .boxed())
    }
}

/// Object-safe port construction behind the public generic binding method.
trait PortFactory: Send + Sync {
    /// Builds a port for the supplied compiled slot.
    fn create(&self, slot: PortIndex) -> Box<dyn BasePort>;
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
}

/// Object-safe direct invocation of a generated reaction implementation.
trait ReactionInvoker: Send + Sync {
    /// Invokes the bound reaction with its exact state and references.
    fn invoke(
        &mut self,
        context: &mut Context,
        state: &mut dyn ReactorData,
        refs: ReactionRefs<'_>,
    ) -> Result<(), ReactionBindingError>;
}

impl<F> ReactionInvoker for F
where
    F: for<'store> FnMut(
            &mut Context,
            &mut dyn ReactorData,
            ReactionRefs<'store>,
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
    ) -> Result<(), ReactionBindingError> {
        self(context, state, refs)
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
    /// An action factory was supplied for an internally constructed shutdown action.
    #[error("unexpected action factory at shutdown slot {slot}")]
    UnexpectedActionFactory {
        /// The compiled shutdown action storage slot.
        slot: ActionSlotIndex,
    },
    /// The set of explicit action factories has incorrect coverage.
    #[error("action factory coverage mismatch: expected {expected}, found {found}")]
    ActionFactoryCoverageMismatch {
        /// Number of non-shutdown action slots in the image.
        expected: usize,
        /// Number of explicitly supplied action factories.
        found: usize,
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
    /// A checked state lookup referred to no initialized state.
    #[error("state slot {slot} is missing")]
    StateMissing {
        /// The requested compiled state storage slot.
        slot: StateSlotIndex,
    },
    /// A checked state lookup used the wrong concrete payload type.
    #[error("state slot {slot} has type {found}, not {expected}")]
    StateTypeMismatch {
        /// The requested compiled state storage slot.
        slot: StateSlotIndex,
        /// The requested concrete Rust type.
        expected: &'static str,
        /// The stored concrete Rust type.
        found: &'static str,
    },
    /// An action delay could not be represented by the runtime duration type.
    #[error("action minimum delay {min_delay_nanos}ns exceeds the runtime duration range")]
    DelayOutOfRange {
        /// The compiled minimum delay in nanoseconds.
        min_delay_nanos: u64,
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
    /// A reaction refers to a state slot that was not initialized.
    #[error("reaction state slot {slot} is missing")]
    MissingReactionState {
        /// The compiled state storage slot.
        slot: StateSlotIndex,
    },
    /// A reaction refers to a context that was not initialized.
    #[error("reaction context for {reactor} is missing")]
    MissingReactionContext {
        /// The compiled reactor index.
        reactor: ReactorIndex,
    },
    /// A reaction refers to a port that was not initialized.
    #[error("reaction port slot {slot} is missing")]
    MissingReactionPort {
        /// The compiled port slot.
        slot: PortIndex,
    },
    /// A reaction refers to an action storage slot that was not initialized.
    #[error("reaction action storage slot {slot} is missing")]
    MissingReactionAction {
        /// The compiled action storage slot.
        slot: ActionSlotIndex,
    },
    /// A directly bound reaction returned a reference-extraction error.
    #[error(transparent)]
    Reaction(#[from] ReactionBindingError),
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
    /// Keeps the per-enclave event channel open for stored reaction contexts.
    event_rx: crate::Receiver<crate::event::AsyncEvent>,
    /// Keeps stored reaction contexts alive until the compiled executor shuts down.
    shutdown_tx: crate::keepalive::Sender,
}

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

impl<'image> OwnedStorage<'image> {
    /// Validates direct bindings and constructs every owned storage collection for `image`.
    pub fn new(
        image: EnclaveImageView<'image>,
        bindings: OwnedBindings,
    ) -> Result<Self, OwnedStorageError> {
        validate_bindings(&image, &bindings)?;

        let (state_bindings, action_images) = validate_storage_layout(&image)?;
        let OwnedBindings {
            bindings,
            actions: action_factories,
            ports: port_factories,
        } = bindings;

        validate_action_delays(&action_images)?;
        let states = initialize_states(
            image.storage_bounds().state_slots(),
            &state_bindings,
            &bindings,
        )?;
        let actions = initialize_actions(
            image.storage_bounds().action_slots(),
            &action_images,
            &action_factories,
        )?;
        let ports = initialize_ports(&image, &port_factories)?;
        let reactions = initialize_reactions(bindings);
        let (contexts, event_rx, shutdown_tx) = initialize_contexts(&image)?;

        Ok(Self {
            image,
            states,
            actions,
            ports,
            contexts,
            reactions,
            event_rx,
            shutdown_tx,
        })
    }

    /// Returns an immutable checked state reference for later execution-result delegation.
    pub(crate) fn state<T: ReactorData>(
        &self,
        slot: StateSlotIndex,
    ) -> Result<&T, OwnedStorageError> {
        let state = self
            .states
            .get(slot)
            .ok_or(OwnedStorageError::StateMissing { slot })?;
        state
            .value
            .downcast_ref::<T>()
            .ok_or(OwnedStorageError::StateTypeMismatch {
                slot,
                expected: std::any::type_name::<T>(),
                found: state.type_name,
            })
    }

    /// Returns mutable access to the exact action storage slot for scheduler-owned scheduling.
    pub(crate) fn action_mut(
        &mut self,
        slot: ActionSlotIndex,
    ) -> Result<&mut dyn BaseAction, OwnedStorageError> {
        Ok(self.actions[slot].as_mut())
    }

    /// Clears every compiled port after its reactions have completed for an execution tag.
    pub(crate) fn reset_ports(&mut self) {
        self.ports.values_mut().for_each(|port| port.cleanup());
    }

    /// Invokes a directly bound reaction using the image's ordered port and action references.
    pub(crate) fn invoke_reaction(
        &mut self,
        reaction: ReactionIndex,
        tag: Tag,
    ) -> Result<TriggerRes, OwnedStorageError> {
        let reaction_image = self.image.reactions()[reaction];
        let reactor = reaction_image.reactor();
        let state_slot = self.image.reactors()[reactor].state_slot();

        let use_slots = self.image.reaction_use_ports(reaction);
        let effect_slots = self.image.reaction_effect_ports(reaction);
        let action_slots = self
            .image
            .reaction_actions(reaction)
            .iter()
            .map(|action| self.image.actions()[*action].storage_slot());

        ensure_unaliased_references(reaction, use_slots, effect_slots, action_slots.clone())?;

        let mut use_ports = port_pointers(&mut self.ports, use_slots)?;
        let mut effect_ports = port_pointers(&mut self.ports, effect_slots)?;
        let action_slots: Vec<_> = action_slots.collect();
        let mut actions = action_pointers(&mut self.actions, &action_slots)?;

        let context = &mut self.contexts[reactor];
        context.reset_for_reaction(tag);
        let state = &mut self.states[state_slot];
        let invoker = self.reactions.get_mut(reaction_image.binding()).ok_or(
            OwnedStorageError::MissingReactionInvoker {
                slot: reaction_image.binding(),
            },
        )?;

        let refs = ReactionRefs {
            ports: Refs::new(&mut use_ports),
            ports_mut: RefsMut::new(&mut effect_ports),
            actions: RefsMut::new(&mut actions),
        };
        invoker.invoke(context, state.value.as_mut(), refs)?;
        Ok(context.trigger_res.clone())
    }

    /// Returns the immutable image defining this storage's dense key domains.
    pub(crate) const fn image(&self) -> &EnclaveImageView<'image> {
        &self.image
    }
}

/// Validates every required binding before any state initializer can run.
fn validate_bindings(
    image: &EnclaveImageView<'_>,
    bindings: &OwnedBindings,
) -> Result<(), OwnedStorageError> {
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

    let mut expected_actions = 0;
    for (_, action) in image.actions().iter() {
        if matches!(action.timing(), ActionTiming::Shutdown) {
            continue;
        }
        expected_actions += 1;
        let slot = action.storage_slot();
        if !bindings.actions.contains_key(slot) {
            return Err(OwnedStorageError::MissingActionFactory { slot });
        }
    }
    if bindings.actions.len() != expected_actions {
        return Err(OwnedStorageError::ActionFactoryCoverageMismatch {
            expected: expected_actions,
            found: bindings.actions.len(),
        });
    }
    for slot in image.ports().keys() {
        if !bindings.ports.contains_key(slot) {
            return Err(OwnedStorageError::MissingPortFactory { slot });
        }
    }
    if bindings.ports.len() != image.ports().len() {
        return Err(OwnedStorageError::PortFactoryCoverageMismatch {
            expected: image.ports().len(),
            found: bindings.ports.len(),
        });
    }
    Ok(())
}

/// Verifies dense state and action slot coverage without initializing payload values.
fn validate_storage_layout(
    image: &EnclaveImageView<'_>,
) -> Result<
    (
        TinySecondaryMap<StateSlotIndex, BindingSlotIndex>,
        TinySecondaryMap<ActionSlotIndex, crate::image::ActionImage>,
    ),
    OwnedStorageError,
> {
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

/// Initializes the dense state map only after all binding and slot validation succeeds.
fn initialize_states(
    count: u32,
    state_bindings: &TinySecondaryMap<StateSlotIndex, BindingSlotIndex>,
    bindings: &TinySecondaryMap<BindingSlotIndex, Binding>,
) -> Result<TinyMap<StateSlotIndex, StoredState>, OwnedStorageError> {
    let mut states = TinyMap::with_capacity(count as usize);
    for raw_slot in 0..count {
        let slot = StateSlotIndex::new(raw_slot);
        let binding_slot = state_bindings[slot];
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

/// Rejects action delays that cannot fit the runtime duration type before state initialization.
fn validate_action_delays(
    action_images: &TinySecondaryMap<ActionSlotIndex, crate::image::ActionImage>,
) -> Result<(), OwnedStorageError> {
    for (_, action) in action_images.iter() {
        if let ActionTiming::Standard {
            min_delay_nanos, ..
        } = action.timing()
        {
            i64::try_from(min_delay_nanos)
                .map_err(|_| OwnedStorageError::DelayOutOfRange { min_delay_nanos })?;
        }
    }
    Ok(())
}

/// Initializes the dense action map and internally supplies shutdown unit actions.
fn initialize_actions(
    count: u32,
    action_images: &TinySecondaryMap<ActionSlotIndex, crate::image::ActionImage>,
    factories: &TinySecondaryMap<ActionSlotIndex, Box<dyn ActionFactory>>,
) -> Result<TinyMap<ActionSlotIndex, Box<dyn BaseAction>>, OwnedStorageError> {
    let mut actions = TinyMap::with_capacity(count as usize);
    for raw_slot in 0..count {
        let slot = ActionSlotIndex::new(raw_slot);
        let action = action_images[slot];
        let value = match action.timing() {
            ActionTiming::Shutdown => {
                Action::<()>::new("shutdown", ActionKey::new(slot.as_u32()), None, true).boxed()
            }
            timing => factories
                .get(slot)
                .ok_or(OwnedStorageError::MissingActionFactory { slot })?
                .create(slot, timing)?,
        };
        let inserted = actions.insert(value);
        debug_assert_eq!(inserted, slot);
    }
    Ok(actions)
}

/// Initializes the dense port map from exact image port slots.
fn initialize_ports(
    image: &EnclaveImageView<'_>,
    factories: &TinySecondaryMap<PortIndex, Box<dyn PortFactory>>,
) -> Result<TinyMap<PortIndex, Box<dyn BasePort>>, OwnedStorageError> {
    let mut ports = TinyMap::with_capacity(image.ports().len());
    for slot in image.ports().keys() {
        let value = factories
            .get(slot)
            .ok_or(OwnedStorageError::MissingPortFactory { slot })?
            .create(slot);
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

/// Initializes one context per reactor plus the channels that keep it schedulable.
fn initialize_contexts(
    image: &EnclaveImageView<'_>,
) -> Result<
    (
        TinyMap<ReactorIndex, Context>,
        crate::Receiver<crate::event::AsyncEvent>,
        crate::keepalive::Sender,
    ),
    OwnedStorageError,
> {
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
            EnclaveKey::default(),
            start_time,
            bank_info,
            event_tx.clone(),
            shutdown_rx.clone(),
        ));
        debug_assert_eq!(inserted, reactor);
    }
    Ok((contexts, event_rx, shutdown_tx))
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
    slots: &[ActionSlotIndex],
) -> Result<Vec<NonNull<dyn BaseAction>>, OwnedStorageError> {
    slots
        .iter()
        .map(|&slot| Ok(NonNull::from(actions[slot].as_mut())))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{
        image::{
            ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex,
            EnclaveImage, EnclaveImageView, IdentityRange, PortImage, PortIndex, ReactionImage,
            ReactionIndex, ReactorImage, ReactorIndex, RequiredBindingImage, ScopeImage,
            ScopeIndex, StateSlotIndex, StorageBounds, TableRange, TimingDomain, TinyMapView,
        },
        CommonContext, Context, Duration, OwnedBindings, OwnedStorage, OwnedStorageError,
        ReactionBindingError, ReactionRefs, ReactorData, Tag,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        _refs: ReactionRefs<'_>,
    ) -> Result<(), ReactionBindingError> {
        if REACTION_CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
            context.schedule_shutdown(Some(Duration::nanoseconds(1)));
        }
        Ok(())
    }

    static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
        BindingSlotIndex::new(0),
        StateSlotIndex::new(0),
        ScopeIndex::new(0),
        TableRange::new(0, 0),
        None,
        None,
    )];
    static ACTIONS: [ActionImage; 1] = [ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(0),
        ActionTiming::Standard {
            domain: TimingDomain::Logical,
            min_delay_nanos: 0,
        },
        TableRange::new(0, 0),
    )];
    static UNREPRESENTABLE_ACTIONS: [ActionImage; 1] = [ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(0),
        ActionTiming::Standard {
            domain: TimingDomain::Logical,
            min_delay_nanos: u64::MAX,
        },
        TableRange::new(0, 0),
    )];
    static PORTS: [PortImage; 1] = [PortImage::new(ScopeIndex::new(0), TableRange::new(0, 0))];
    static REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
        ReactorIndex::new(0),
        ScopeIndex::new(0),
        0,
        BindingSlotIndex::new(1),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
    )];
    static SCOPES: [ScopeImage; 1] = [ScopeImage::new(
        None,
        ReactorIndex::new(0),
        None,
        TableRange::new(0, 1),
        TableRange::new(0, 1),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
    )];
    static SCOPE_DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
    static SCOPE_LOGICAL_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
    static REQUIRED_BINDINGS: [RequiredBindingImage; 2] = [
        RequiredBindingImage::new(IdentityRange::new(7, 7), BindingKind::StateInitializer),
        RequiredBindingImage::new(IdentityRange::new(14, 10), BindingKind::Reaction),
    ];
    static IMAGE: EnclaveImage<'static> = EnclaveImage {
        identity_data: "enclavea-stateb-reaction",
        enclave_id: IdentityRange::new(0, 7),
        reactors: TinyMapView::new(&REACTORS),
        actions: TinyMapView::new(&ACTIONS),
        ports: TinyMapView::new(&PORTS),
        reactions: TinyMapView::new(&REACTIONS),
        modes: TinyMapView::new(&[]),
        scopes: TinyMapView::new(&SCOPES),
        reaction_triggers: &[],
        reaction_use_ports: &[],
        reaction_effect_ports: &[],
        reaction_actions: &[],
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

    /// Returns a fresh validated view of the immutable test image.
    fn image() -> EnclaveImageView<'static> {
        EnclaveImageView::new(&IMAGE).expect("test image is valid")
    }

    /// Returns a validated image whose action delay cannot fit the runtime duration type.
    fn unrepresentable_action_image() -> EnclaveImageView<'static> {
        EnclaveImageView::new(&UNREPRESENTABLE_ACTION_IMAGE).expect("test image is valid")
    }

    /// Returns bindings for every non-lifecycle storage slot in [`IMAGE`].
    fn complete_bindings() -> OwnedBindings {
        OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_action::<u32>(ActionSlotIndex::new(0))
            .bind_port::<u32>(PortIndex::new(0))
            .bind_reaction(BindingSlotIndex::new(1), reaction)
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
    fn missing_action_factory_is_rejected() {
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_port::<u32>(PortIndex::new(0))
            .bind_reaction(BindingSlotIndex::new(1), reaction);

        let error = OwnedStorage::new(image(), bindings).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::MissingActionFactory {
                slot,
            } if slot == ActionSlotIndex::new(0)
        ));
    }

    #[test]
    fn missing_port_factory_is_rejected() {
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_action::<u32>(ActionSlotIndex::new(0))
            .bind_reaction(BindingSlotIndex::new(1), reaction);

        let error = OwnedStorage::new(image(), bindings).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::MissingPortFactory {
                slot,
            } if slot == PortIndex::new(0)
        ));
    }

    #[test]
    fn state_accessor_rejects_a_wrong_concrete_type() {
        let storage = OwnedStorage::new(image(), complete_bindings()).unwrap();

        let error = storage.state::<u32>(StateSlotIndex::new(0)).unwrap_err();

        assert!(matches!(
            error,
            OwnedStorageError::StateTypeMismatch {
                slot,
                expected,
                found,
            } if slot == StateSlotIndex::new(0)
                && expected == std::any::type_name::<u32>()
                && found == std::any::type_name::<TestState>()
        ));
    }

    #[test]
    fn rejects_unrepresentable_action_delay_before_initializing_state() {
        INITIALIZER_CALLS.store(0, Ordering::SeqCst);
        let bindings = OwnedBindings::new()
            .bind_state(BindingSlotIndex::new(0), counted_initializer)
            .bind_action::<u32>(ActionSlotIndex::new(0))
            .bind_port::<u32>(PortIndex::new(0))
            .bind_reaction(BindingSlotIndex::new(1), reaction);

        let error = OwnedStorage::new(unrepresentable_action_image(), bindings).unwrap_err();

        assert!(matches!(error, OwnedStorageError::DelayOutOfRange { .. }));
        assert_eq!(INITIALIZER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn invocation_uses_the_given_tag_and_returns_a_fresh_trigger_result() {
        REACTION_CALLS.store(0, Ordering::SeqCst);
        let bindings =
            complete_bindings().bind_reaction(BindingSlotIndex::new(1), schedule_shutdown_once);
        let mut storage = OwnedStorage::new(image(), bindings).unwrap();
        let first_tag = Tag::new(Duration::nanoseconds(7), 2);
        let second_tag = Tag::new(Duration::nanoseconds(9), 0);

        let first = storage
            .invoke_reaction(ReactionIndex::new(0), first_tag)
            .unwrap();
        let second = storage
            .invoke_reaction(ReactionIndex::new(0), second_tag)
            .unwrap();

        assert_eq!(
            first.scheduled_shutdown,
            Some(Tag::new(Duration::nanoseconds(8), 0))
        );
        assert!(second.scheduled_shutdown.is_none());
    }
}
