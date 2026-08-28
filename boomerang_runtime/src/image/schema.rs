pub use tinymap::{TableRange, TinyMapView};

tinymap::key_type!(pub ReactorIndex);
tinymap::key_type!(pub ActionIndex);
tinymap::key_type!(pub PortIndex);
tinymap::key_type!(pub ReactionIndex);
tinymap::key_type!(pub ModeIndex);
tinymap::key_type!(pub ScopeIndex);
tinymap::key_type!(pub StateSlotIndex);
tinymap::key_type!(pub ActionSlotIndex);
tinymap::key_type!(pub BindingSlotIndex);
tinymap::key_type!(pub RouteIndex);
tinymap::key_type!(pub FederateIndex);
tinymap::key_type!(pub EnclaveIndex);

macro_rules! borrowed_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name<'a>(&'a str);

        impl<'a> $name<'a> {
            /// Creates an unchecked borrowed identity.
            pub const fn new(value: &'a str) -> Self {
                Self(value)
            }

            /// Returns the identity text.
            pub const fn as_str(self) -> &'a str {
                self.0
            }
        }
    };
}

borrowed_id!(EnclaveId, "A stable borrowed Enclave identity.");
borrowed_id!(FederateId, "A stable borrowed Federate identity.");
borrowed_id!(TargetId, "A stable borrowed compilation-target identity.");
borrowed_id!(
    RuntimeBackendId,
    "A stable borrowed runtime-backend identity."
);
borrowed_id!(BoundaryId, "A stable borrowed scheduler-boundary identity.");
borrowed_id!(
    BindingSlotId,
    "A stable borrowed implementation-binding identity."
);

/// A byte range into an Enclave image's UTF-8 identity blob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityRange {
    start: u32,
    len: u32,
}

impl IdentityRange {
    /// Creates an unchecked identity range.
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    /// Returns the first byte offset.
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the byte length.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(self) -> u32 {
        self.len
    }

    /// Returns the referenced UTF-8 substring when the byte range is valid.
    pub fn get(self, value: &str) -> Option<&str> {
        let end = self.start.checked_add(self.len)?;
        value.get(self.start as usize..end as usize)
    }
}

/// A Federate and the contiguous Enclave images it owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FederateImage {
    id: IdentityRange,
    target: IdentityRange,
    runtime: IdentityRange,
    enclaves: TableRange<EnclaveIndex>,
}

impl FederateImage {
    /// Creates an unchecked Federate record.
    pub const fn new(
        id: IdentityRange,
        target: IdentityRange,
        runtime: IdentityRange,
        enclaves: TableRange<EnclaveIndex>,
    ) -> Self {
        Self {
            id,
            target,
            runtime,
            enclaves,
        }
    }

    /// Returns the stable Federate identity range.
    pub const fn id(self) -> IdentityRange {
        self.id
    }

    /// Returns the compilation-target identity range.
    pub const fn target(self) -> IdentityRange {
        self.target
    }

    /// Returns the runtime-backend identity range.
    pub const fn runtime(self) -> IdentityRange {
        self.runtime
    }

    /// Returns the range of owned Enclave images.
    pub const fn enclaves(self) -> TableRange<EnclaveIndex> {
        self.enclaves
    }
}

/// A backend-neutral cross-Federate boundary edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FederationEdgeImage {
    boundary: IdentityRange,
    source: FederateIndex,
    target: FederateIndex,
    delay_nanos: u64,
}

impl FederationEdgeImage {
    /// Creates an unchecked federation edge.
    pub const fn new(
        boundary: IdentityRange,
        source: FederateIndex,
        target: FederateIndex,
        delay_nanos: u64,
    ) -> Self {
        Self {
            boundary,
            source,
            target,
            delay_nanos,
        }
    }

    /// Returns the stable boundary identity range.
    pub const fn boundary(self) -> IdentityRange {
        self.boundary
    }

    /// Returns the source Federate.
    pub const fn source(self) -> FederateIndex {
        self.source
    }

    /// Returns the target Federate.
    pub const fn target(self) -> FederateIndex {
        self.target
    }

    /// Returns the logical delay in nanoseconds.
    pub const fn delay_nanos(self) -> u64 {
        self.delay_nanos
    }
}

/// Backend-neutral immutable federation membership and edges.
#[derive(Clone, Copy, Debug)]
pub struct GlobalFederationImage<'a> {
    /// Federates participating in canonical stable-identity order.
    pub members: &'a [FederateIndex],
    /// Canonically ordered cross-Federate boundary edges.
    pub edges: &'a [FederationEdgeImage],
}

impl<'a> GlobalFederationImage<'a> {
    /// Creates an unchecked global federation image.
    pub const fn new(members: &'a [FederateIndex], edges: &'a [FederationEdgeImage]) -> Self {
        Self { members, edges }
    }
}

/// Selected immutable logical-time coordination projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinationProjection {
    /// No distributed coordinator is required.
    Local,
}

/// An unchecked aggregate of one complete compiled deployment.
#[derive(Clone, Copy, Debug)]
pub struct CompiledDeploymentImage<'a> {
    /// UTF-8 storage for deployment-level stable identities.
    pub identity_data: &'a str,
    /// Backend-neutral global federation structure.
    pub federation: GlobalFederationImage<'a>,
    /// Dense Federate ownership records.
    pub federates: TinyMapView<'a, FederateIndex, FederateImage>,
    /// Federate-grouped Enclave scheduler images.
    pub enclaves: TinyMapView<'a, EnclaveIndex, EnclaveImage<'a>>,
    /// Selected backend-specific coordination projection.
    pub coordination: CoordinationProjection,
}

/// An immutable reactor scheduler record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReactorImage {
    state_binding: BindingSlotIndex,
    state_slot: StateSlotIndex,
    root_scope: ScopeIndex,
    modes: TableRange<ModeIndex>,
    initial_mode: Option<ModeIndex>,
    bank: Option<BankInfoImage>,
}

impl ReactorImage {
    /// Creates an unchecked reactor record.
    pub const fn new(
        state_binding: BindingSlotIndex,
        state_slot: StateSlotIndex,
        root_scope: ScopeIndex,
        modes: TableRange<ModeIndex>,
        initial_mode: Option<ModeIndex>,
        bank: Option<BankInfoImage>,
    ) -> Self {
        Self {
            state_binding,
            state_slot,
            root_scope,
            modes,
            initial_mode,
            bank,
        }
    }

    /// Returns the required state-initializer binding slot.
    pub const fn state_binding(self) -> BindingSlotIndex {
        self.state_binding
    }

    /// Returns the dense mutable-state slot.
    pub const fn state_slot(self) -> StateSlotIndex {
        self.state_slot
    }

    /// Returns the reactor's root scope.
    pub const fn root_scope(self) -> ScopeIndex {
        self.root_scope
    }

    /// Returns the reactor's canonical mode range.
    pub const fn modes(self) -> TableRange<ModeIndex> {
        self.modes
    }

    /// Returns the initially active mode, if any.
    pub const fn initial_mode(self) -> Option<ModeIndex> {
        self.initial_mode
    }

    /// Returns the reactor's bank position, if it belongs to a bank.
    pub const fn bank(self) -> Option<BankInfoImage> {
        self.bank
    }
}

/// A reactor's position in a statically sized bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BankInfoImage {
    index: u32,
    total: u32,
}

impl BankInfoImage {
    /// Creates unchecked reactor-bank metadata.
    pub const fn new(index: u32, total: u32) -> Self {
        Self { index, total }
    }

    /// Returns the zero-based reactor index within the bank.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Returns the number of reactors in the bank.
    pub const fn total(self) -> u32 {
        self.total
    }
}

/// The clock domain used to interpret an action or route delay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimingDomain {
    /// Interpret the delay against logical time.
    Logical,
    /// Interpret the delay against physical time.
    Physical,
}

/// Immutable scheduling semantics for an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionTiming {
    /// A user-scheduled action with a canonical minimum delay.
    Standard {
        /// Clock domain used to schedule the action.
        domain: TimingDomain,
        /// Minimum scheduling delay in nanoseconds.
        min_delay_nanos: u64,
    },
    /// A logical timer with an optional repetition period.
    Timer {
        /// Repetition period in nanoseconds, or `None` for a one-shot timer.
        period_nanos: Option<u64>,
    },
    /// The internal action supplying a shutdown reaction's unit value.
    Shutdown,
}

/// An immutable action scheduler record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionImage {
    scope: ScopeIndex,
    storage_slot: ActionSlotIndex,
    timing: ActionTiming,
    triggers: TableRange<LevelReactionImage>,
}

impl ActionImage {
    /// Creates an unchecked action record.
    pub const fn new(
        scope: ScopeIndex,
        storage_slot: ActionSlotIndex,
        timing: ActionTiming,
        triggers: TableRange<LevelReactionImage>,
    ) -> Self {
        Self {
            scope,
            storage_slot,
            timing,
            triggers,
        }
    }

    /// Returns the action's static scope.
    pub const fn scope(self) -> ScopeIndex {
        self.scope
    }

    /// Returns the dense action-storage slot.
    pub const fn storage_slot(self) -> ActionSlotIndex {
        self.storage_slot
    }

    /// Returns the action's immutable scheduling semantics.
    pub const fn timing(self) -> ActionTiming {
        self.timing
    }

    /// Returns the action's flattened trigger range.
    pub const fn triggers(self) -> TableRange<LevelReactionImage> {
        self.triggers
    }
}

/// An immutable port scheduler record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortImage {
    scope: ScopeIndex,
    triggers: TableRange<LevelReactionImage>,
}

impl PortImage {
    /// Creates an unchecked port record.
    pub const fn new(scope: ScopeIndex, triggers: TableRange<LevelReactionImage>) -> Self {
        Self { scope, triggers }
    }

    /// Returns the port's static scope.
    pub const fn scope(self) -> ScopeIndex {
        self.scope
    }

    /// Returns the port's flattened trigger range.
    pub const fn triggers(self) -> TableRange<LevelReactionImage> {
        self.triggers
    }
}

/// An immutable reaction scheduler record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReactionImage {
    reactor: ReactorIndex,
    scope: ScopeIndex,
    dependency_level: u32,
    binding: BindingSlotIndex,
    use_ports: TableRange<PortIndex>,
    effect_ports: TableRange<PortIndex>,
    actions: TableRange<ActionIndex>,
    enabled_modes: TableRange<ModeIndex>,
}

impl ReactionImage {
    /// Creates an unchecked reaction record.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        reactor: ReactorIndex,
        scope: ScopeIndex,
        dependency_level: u32,
        binding: BindingSlotIndex,
        use_ports: TableRange<PortIndex>,
        effect_ports: TableRange<PortIndex>,
        actions: TableRange<ActionIndex>,
        enabled_modes: TableRange<ModeIndex>,
    ) -> Self {
        Self {
            reactor,
            scope,
            dependency_level,
            binding,
            use_ports,
            effect_ports,
            actions,
            enabled_modes,
        }
    }

    /// Returns the owning reactor.
    pub const fn reactor(self) -> ReactorIndex {
        self.reactor
    }

    /// Returns the static execution scope.
    pub const fn scope(self) -> ScopeIndex {
        self.scope
    }

    /// Returns the precomputed dependency level.
    pub const fn dependency_level(self) -> u32 {
        self.dependency_level
    }

    /// Returns the required reaction binding slot.
    pub const fn binding(self) -> BindingSlotIndex {
        self.binding
    }

    /// Returns the ordered use-port range.
    pub const fn use_ports(self) -> TableRange<PortIndex> {
        self.use_ports
    }

    /// Returns the ordered effect-port range.
    pub const fn effect_ports(self) -> TableRange<PortIndex> {
        self.effect_ports
    }

    /// Returns the ordered action-reference range.
    pub const fn actions(self) -> TableRange<ActionIndex> {
        self.actions
    }

    /// Returns the enabled-mode range.
    pub const fn enabled_modes(self) -> TableRange<ModeIndex> {
        self.enabled_modes
    }
}

/// An immutable mode scheduler record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeImage {
    reactor: ReactorIndex,
    scope: ScopeIndex,
}

impl ModeImage {
    /// Creates an unchecked mode record.
    pub const fn new(reactor: ReactorIndex, scope: ScopeIndex) -> Self {
        Self { reactor, scope }
    }

    /// Returns the owning reactor.
    pub const fn reactor(self) -> ReactorIndex {
        self.reactor
    }

    /// Returns the mode's execution scope.
    pub const fn scope(self) -> ScopeIndex {
        self.scope
    }
}

/// An immutable execution-scope scheduler record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeImage {
    parent: Option<ScopeIndex>,
    reactor: ReactorIndex,
    mode: Option<ModeIndex>,
    descendants: TableRange<ScopeIndex>,
    logical_actions: TableRange<ActionIndex>,
    timer_startups: TableRange<TimerStartupImage>,
    reset_reactions: TableRange<LevelReactionImage>,
    startup_reactions: TableRange<LifecycleReactionImage>,
    shutdown_reactions: TableRange<LifecycleReactionImage>,
}

impl ScopeImage {
    /// Creates an unchecked scope record.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        parent: Option<ScopeIndex>,
        reactor: ReactorIndex,
        mode: Option<ModeIndex>,
        descendants: TableRange<ScopeIndex>,
        logical_actions: TableRange<ActionIndex>,
        timer_startups: TableRange<TimerStartupImage>,
        reset_reactions: TableRange<LevelReactionImage>,
        startup_reactions: TableRange<LifecycleReactionImage>,
        shutdown_reactions: TableRange<LifecycleReactionImage>,
    ) -> Self {
        Self {
            parent,
            reactor,
            mode,
            descendants,
            logical_actions,
            timer_startups,
            reset_reactions,
            startup_reactions,
            shutdown_reactions,
        }
    }

    /// Returns the parent scope, if any.
    pub const fn parent(self) -> Option<ScopeIndex> {
        self.parent
    }

    /// Returns the owning reactor.
    pub const fn reactor(self) -> ReactorIndex {
        self.reactor
    }

    /// Returns the owning mode for a mode scope.
    pub const fn mode(self) -> Option<ModeIndex> {
        self.mode
    }

    /// Returns the precomputed descendant range.
    pub const fn descendants(self) -> TableRange<ScopeIndex> {
        self.descendants
    }

    /// Returns the precomputed logical-action range.
    pub const fn logical_actions(self) -> TableRange<ActionIndex> {
        self.logical_actions
    }

    /// Returns the precomputed timer-startup range.
    pub const fn timer_startups(self) -> TableRange<TimerStartupImage> {
        self.timer_startups
    }

    /// Returns the precomputed reset-reaction range.
    pub const fn reset_reactions(self) -> TableRange<LevelReactionImage> {
        self.reset_reactions
    }

    /// Returns the precomputed startup-reaction range.
    pub const fn startup_reactions(self) -> TableRange<LifecycleReactionImage> {
        self.startup_reactions
    }

    /// Returns the precomputed shutdown-reaction range.
    pub const fn shutdown_reactions(self) -> TableRange<LifecycleReactionImage> {
        self.shutdown_reactions
    }
}

/// A reaction reference paired with its precomputed dependency level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LevelReactionImage {
    level: u32,
    reaction: ReactionIndex,
}

impl LevelReactionImage {
    /// Creates a leveled reaction reference.
    pub const fn new(level: u32, reaction: ReactionIndex) -> Self {
        Self { level, reaction }
    }

    /// Returns the dependency level.
    pub const fn level(self) -> u32 {
        self.level
    }

    /// Returns the referenced reaction.
    pub const fn reaction(self) -> ReactionIndex {
        self.reaction
    }
}

/// A precomputed timer or lifecycle action startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerStartupImage {
    action: ActionIndex,
    logical_delay_nanos: u64,
}

impl TimerStartupImage {
    /// Creates a timer startup entry.
    pub const fn new(action: ActionIndex, logical_delay_nanos: u64) -> Self {
        Self {
            action,
            logical_delay_nanos,
        }
    }

    /// Returns the action to schedule.
    pub const fn action(self) -> ActionIndex {
        self.action
    }

    /// Returns the logical delay in nanoseconds.
    pub const fn logical_delay_nanos(self) -> u64 {
        self.logical_delay_nanos
    }
}

/// A precomputed lifecycle reaction and its unit-valued trigger action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleReactionImage {
    reaction: LevelReactionImage,
    action: ActionIndex,
}

impl LifecycleReactionImage {
    /// Creates a lifecycle reaction entry.
    pub const fn new(reaction: LevelReactionImage, action: ActionIndex) -> Self {
        Self { reaction, action }
    }

    /// Returns the leveled reaction reference.
    pub const fn reaction(self) -> LevelReactionImage {
        self.reaction
    }

    /// Returns the lifecycle trigger action.
    pub const fn action(self) -> ActionIndex {
        self.action
    }
}

/// The local direction of a scheduler boundary route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteDirection {
    /// The route admits events through a local port.
    Inbound,
    /// The route emits events from a local port.
    Outbound,
}

/// An immutable scheduler-boundary route without transport state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteImage {
    boundary: IdentityRange,
    local_port: PortIndex,
    direction: RouteDirection,
    timing_domain: TimingDomain,
    delay_nanos: u64,
}

impl RouteImage {
    /// Creates an unchecked route record.
    pub const fn new(
        boundary: IdentityRange,
        local_port: PortIndex,
        direction: RouteDirection,
        timing_domain: TimingDomain,
        delay_nanos: u64,
    ) -> Self {
        Self {
            boundary,
            local_port,
            direction,
            timing_domain,
            delay_nanos,
        }
    }

    /// Returns the boundary identity's blob range.
    pub const fn boundary(self) -> IdentityRange {
        self.boundary
    }

    /// Returns the local dense port identity.
    pub const fn local_port(self) -> PortIndex {
        self.local_port
    }

    /// Returns the route direction.
    pub const fn direction(self) -> RouteDirection {
        self.direction
    }

    /// Returns the clock domain used to interpret the route delay.
    pub const fn timing_domain(self) -> TimingDomain {
        self.timing_domain
    }

    /// Returns the route delay in nanoseconds.
    pub const fn delay_nanos(self) -> u64 {
        self.delay_nanos
    }
}

/// The implementation contract required at a binding slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    /// A reactor-state initializer is required.
    StateInitializer,
    /// A reaction implementation is required.
    Reaction,
}

/// A required stable implementation-binding slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequiredBindingImage {
    id: IdentityRange,
    kind: BindingKind,
}

impl RequiredBindingImage {
    /// Creates an unchecked required-binding record.
    pub const fn new(id: IdentityRange, kind: BindingKind) -> Self {
        Self { id, kind }
    }

    /// Returns the binding identity's blob range.
    pub const fn id(self) -> IdentityRange {
        self.id
    }

    /// Returns the required implementation kind.
    pub const fn kind(self) -> BindingKind {
        self.kind
    }
}

/// Fixed mutable-storage and scheduler-workspace bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageBounds {
    state_slots: u32,
    action_slots: u32,
    event_capacity: u32,
    payload_bytes: u64,
    state_bytes: u64,
    scratch_bytes: u64,
}

impl StorageBounds {
    /// Creates slot, queue, payload, state, and scratch bounds.
    pub const fn new(
        state_slots: u32,
        action_slots: u32,
        event_capacity: u32,
        payload_bytes: u64,
        state_bytes: u64,
        scratch_bytes: u64,
    ) -> Self {
        Self {
            state_slots,
            action_slots,
            event_capacity,
            payload_bytes,
            state_bytes,
            scratch_bytes,
        }
    }

    /// Returns the state-slot bound.
    pub const fn state_slots(self) -> u32 {
        self.state_slots
    }

    /// Returns the action-slot bound.
    pub const fn action_slots(self) -> u32 {
        self.action_slots
    }

    /// Returns the event-queue capacity.
    pub const fn event_capacity(self) -> u32 {
        self.event_capacity
    }

    /// Returns the payload-storage bound in bytes.
    pub const fn payload_bytes(self) -> u64 {
        self.payload_bytes
    }

    /// Returns the reactor-state storage bound in bytes.
    pub const fn state_bytes(self) -> u64 {
        self.state_bytes
    }

    /// Returns the scheduler scratch-storage bound in bytes.
    pub const fn scratch_bytes(self) -> u64 {
        self.scratch_bytes
    }
}

/// An unchecked aggregate of borrowed immutable scheduler tables.
#[derive(Clone, Copy, Debug)]
pub struct EnclaveImage<'a> {
    /// UTF-8 storage for all stable identities referenced by this image.
    pub identity_data: &'a str,
    /// Stable Enclave identity range.
    pub enclave_id: IdentityRange,
    /// Dense reactor records.
    pub reactors: TinyMapView<'a, ReactorIndex, ReactorImage>,
    /// Dense action records.
    pub actions: TinyMapView<'a, ActionIndex, ActionImage>,
    /// Dense port records; each key is also its storage identity.
    pub ports: TinyMapView<'a, PortIndex, PortImage>,
    /// Dense reaction records.
    pub reactions: TinyMapView<'a, ReactionIndex, ReactionImage>,
    /// Dense mode records.
    pub modes: TinyMapView<'a, ModeIndex, ModeImage>,
    /// Dense execution-scope records.
    pub scopes: TinyMapView<'a, ScopeIndex, ScopeImage>,
    /// Flattened action and port trigger entries.
    pub reaction_triggers: &'a [LevelReactionImage],
    /// Flattened ordered reaction use ports.
    pub reaction_use_ports: &'a [PortIndex],
    /// Flattened ordered reaction effect ports.
    pub reaction_effect_ports: &'a [PortIndex],
    /// Flattened ordered reaction actions.
    pub reaction_actions: &'a [ActionIndex],
    /// Flattened reaction mode filters.
    pub reaction_modes: &'a [ModeIndex],
    /// Flattened precomputed scope descendants.
    pub scope_descendants: &'a [ScopeIndex],
    /// Flattened precomputed logical actions.
    pub scope_logical_actions: &'a [ActionIndex],
    /// Flattened precomputed scope timer startups.
    pub scope_timer_startups: &'a [TimerStartupImage],
    /// Flattened precomputed scope reset reactions.
    pub scope_reset_reactions: &'a [LevelReactionImage],
    /// Flattened precomputed scope startup reactions.
    pub scope_startup_reactions: &'a [LifecycleReactionImage],
    /// Flattened precomputed scope shutdown reactions.
    pub scope_shutdown_reactions: &'a [LifecycleReactionImage],
    /// Global startup action entries.
    pub startup_actions: &'a [TimerStartupImage],
    /// Global timer startup entries.
    pub timer_startup_actions: &'a [TimerStartupImage],
    /// Global shutdown reaction entries.
    pub shutdown_reactions: &'a [LifecycleReactionImage],
    /// Unique actions populated before global shutdown reactions execute.
    pub shutdown_actions: &'a [ActionIndex],
    /// Dense scheduler-boundary routes.
    pub routes: TinyMapView<'a, RouteIndex, RouteImage>,
    /// Dense required implementation bindings.
    pub required_bindings: TinyMapView<'a, BindingSlotIndex, RequiredBindingImage>,
    /// Fixed mutable-storage and workspace bounds.
    pub storage_bounds: StorageBounds,
}
