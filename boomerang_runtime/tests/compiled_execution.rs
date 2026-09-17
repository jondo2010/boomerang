//! Owned runtime image, binding, scheduler, and backend contracts.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use boomerang_runtime::AsyncEvent;
use boomerang_runtime::{
    execute_owned, execute_owned_federate,
    image::{
        ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotId,
        BindingSlotIndex, BoundaryId, CompiledDeploymentImage, CoordinationProjection, EnclaveId,
        EnclaveImage, EnclaveIndex, FederateId, FederateImage, FederateIndex,
        GlobalFederationImage, ImageValidationError, IndexSpan, LevelReactionImage,
        LifecycleReactionImage, ModeImage, ModeIndex, PortImage, PortIndex, ReactionImage,
        ReactionIndex, ReactorImage, ReactorIndex, RequiredBindingImage, RouteDirection,
        RouteImage, RuntimeBackendId, ScopeImage, ScopeIndex, SliceRange, StateSlotIndex,
        StorageBounds, TargetId, TimerStartupImage, TimingDomain, TinyMapView,
    },
    ActionRef, CommonContext, CompiledModeEffectRef, Config, Context, Duration, EnclaveBindings,
    EnclaveKey, ExecuteOwnedError, ExecuteOwnedFederateError, FederateBindings, InputRef,
    ModeEffectRef, ModeKey, OutputRef, OwnedStorageError, PayloadType, ReactionBindingError,
    ReactionRefs, ReactorData, RuntimeError, StateAccessError, Tag, TransitionKind,
};

macro_rules! r {
    ($start:expr, $len:expr) => {
        SliceRange::new($start, $len)
    };
}

macro_rules! s {
    ($start:expr, $len:expr) => {
        IndexSpan::new($start, $len)
    };
}

const fn fixture_federate(
    id: &'static str,
    target: &'static str,
    runtime: &'static str,
    enclaves: IndexSpan<EnclaveIndex>,
) -> FederateImage<'static> {
    FederateImage::new(
        FederateId::new(id),
        TargetId::new(target),
        RuntimeBackendId::new(runtime),
        enclaves,
    )
}

const fn fixture_route(
    boundary: &'static str,
    local_port: PortIndex,
    direction: RouteDirection,
    timing: TimingDomain,
    after_nanos: u64,
) -> RouteImage<'static> {
    RouteImage::new(
        BoundaryId::new(boundary),
        local_port,
        direction,
        timing,
        after_nanos,
    )
}

const fn fixture_binding(id: &'static str, kind: BindingKind) -> RequiredBindingImage<'static> {
    RequiredBindingImage::new(BindingSlotId::new(id), kind)
}

#[derive(Clone, Default)]
struct TraceOutput(Arc<Mutex<Vec<u8>>>);

struct TraceWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for TraceWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TraceOutput {
    type Writer = TraceWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TraceWriter(self.0.clone())
    }
}

fn capture_runtime<T>(run: impl FnOnce() -> T) -> (T, Vec<serde_json::Value>) {
    let output = TraceOutput::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .without_time()
        .with_writer(output.clone())
        .with_env_filter("boomerang=debug")
        .finish();
    let result = tracing::subscriber::with_default(subscriber, run);
    let bytes = output.0.lock().unwrap();
    let events = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (result, events)
}

fn run_runtime_trace_test(test: &str) -> bool {
    if std::env::var_os("BOOMERANG_RUNTIME_TRACE_CHILD").is_some() {
        return true;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test])
        .env("BOOMERANG_RUNTIME_TRACE_CHILD", "1")
        .status()
        .expect("runtime trace child starts");
    assert!(status.success(), "runtime trace child failed: {status}");
    false
}

fn lifecycle_events(events: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    events
        .iter()
        .filter(|event| {
            event["fields"]["event"].as_str().is_some_and(|name| {
                name.starts_with("runtime.preflight") || name.starts_with("runtime.construction")
            })
        })
        .collect()
}

/// Mutable reactor state whose startup reaction records one execution.
#[derive(Debug)]
struct CounterState {
    count: usize,
    tags: Vec<Tag>,
}

/// Initializes the counter state supplied by the direct state binding.
fn initialize_counter() -> CounterState {
    CounterState {
        count: 0,
        tags: Vec::new(),
    }
}

/// Increments the directly bound reactor state when its startup reaction executes.
fn increment_counter(
    context: &mut Context,
    state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let state = state
        .downcast_mut::<CounterState>()
        .expect("the image's state binding initializes CounterState");
    state.count += 1;
    state.tags.push(context.get_tag());
    Ok(())
}

fn reference_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(BindingSlotIndex::new(1), increment_counter)
}

static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    s!(0, 0),
    None,
    None,
)];
const ACTIONS: [ActionImage; 1] = [ActionImage::new(
    ScopeIndex::new(0),
    ActionSlotIndex::new(0),
    ActionTiming::Timer { period_nanos: None },
    r!(0, 1),
    None,
)];
static COALESCED_ACTIONS: [ActionImage; 2] = [
    {
        let [action] = ACTIONS;
        action
    },
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(1),
        ActionTiming::Timer { period_nanos: None },
        r!(1, 1),
        None,
    ),
];
static PORTS: [boomerang_runtime::image::PortImage; 0] = [];
static REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
    ReactorIndex::new(0),
    ScopeIndex::new(0),
    0,
    BindingSlotIndex::new(1),
    r!(0, 0),
    r!(0, 0),
    r!(0, 0),
    r!(0, 0),
)];
static MODES: [boomerang_runtime::image::ModeImage; 0] = [];
static SCOPES: [ScopeImage; 1] = [ScopeImage::new(
    None,
    ReactorIndex::new(0),
    None,
    r!(0, 1),
    r!(0, 1),
    r!(0, 0),
    r!(0, 0),
    r!(0, 1),
    r!(0, 0),
)];
static REACTION_TRIGGERS: [LevelReactionImage; 1] =
    [LevelReactionImage::new(0, ReactionIndex::new(0))];
static COALESCED_REACTION_TRIGGERS: [LevelReactionImage; 2] = [
    REACTION_TRIGGERS[0],
    LevelReactionImage::new(0, ReactionIndex::new(0)),
];
static SCOPE_DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
static SCOPE_LOGICAL_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
static SCOPE_STARTUP_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
    LevelReactionImage::new(0, ReactionIndex::new(0)),
    ActionIndex::new(0),
)];
static STARTUP_ACTIONS: [TimerStartupImage; 1] = [TimerStartupImage::new(ActionIndex::new(0), 5)];
static COALESCED_STARTUP_ACTIONS: [TimerStartupImage; 2] = [
    TimerStartupImage::new(ActionIndex::new(0), 0),
    TimerStartupImage::new(ActionIndex::new(1), 1),
];
static ROUTES: [boomerang_runtime::image::RouteImage; 0] = [];
static ROUTED_PORTS: [PortImage; 1] = [PortImage::new(
    ScopeIndex::new(0),
    r!(0, 0),
    BindingSlotIndex::new(2),
)];
static ROUTED_ROUTES: [RouteImage; 1] = [fixture_route(
    "compiled/reference",
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    0,
)];
static REQUIRED_BINDINGS: [RequiredBindingImage; 2] = [
    fixture_binding("counter-state", BindingKind::StateInitializer),
    fixture_binding("increment-counter", BindingKind::Reaction),
];
static ROUTED_REQUIRED_BINDINGS: [RequiredBindingImage; 3] = [
    REQUIRED_BINDINGS[0],
    REQUIRED_BINDINGS[1],
    fixture_binding("routed-port", BindingKind::Port),
];

const fn fixture_reaction(
    scope: u32,
    binding: u32,
    use_ports: SliceRange<PortIndex>,
    actions: SliceRange<ActionIndex>,
    modes: SliceRange<ModeIndex>,
) -> ReactionImage {
    ReactionImage::new(
        ReactorIndex::new(0),
        ScopeIndex::new(scope),
        0,
        BindingSlotIndex::new(binding),
        use_ports,
        r!(0, 0),
        actions,
        modes,
    )
}

const fn fixture_timer_action(
    slot: u32,
    period_nanos: Option<u64>,
    triggers: SliceRange<LevelReactionImage>,
) -> ActionImage {
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(slot),
        ActionTiming::Timer { period_nanos },
        triggers,
        None,
    )
}

const fn fixture_scope(
    parent: Option<ScopeIndex>,
    mode: Option<ModeIndex>,
    descendants: SliceRange<ScopeIndex>,
    logical_actions: SliceRange<ActionIndex>,
    timer_startups: SliceRange<TimerStartupImage>,
    startups: SliceRange<LifecycleReactionImage>,
) -> ScopeImage {
    ScopeImage::new(
        parent,
        ReactorIndex::new(0),
        mode,
        descendants,
        logical_actions,
        timer_startups,
        r!(0, 0),
        startups,
        r!(0, 0),
    )
}
static IMAGE: EnclaveImage<'static> = EnclaveImage {
    enclave_id: EnclaveId::new("compiled/reference"),
    reactors: TinyMapView::new(&REACTORS),
    actions: TinyMapView::new(&ACTIONS),
    ports: TinyMapView::new(&PORTS),
    reactions: TinyMapView::new(&REACTIONS),
    modes: TinyMapView::new(&MODES),
    scopes: TinyMapView::new(&SCOPES),
    reaction_triggers: &REACTION_TRIGGERS,
    reaction_use_ports: &[],
    reaction_effect_ports: &[],
    reaction_actions: &[],
    reaction_modes: &[],
    scope_descendants: &SCOPE_DESCENDANTS,
    scope_logical_actions: &SCOPE_LOGICAL_ACTIONS,
    scope_timer_startups: &[],
    scope_reset_reactions: &[],
    scope_startup_reactions: &SCOPE_STARTUP_REACTIONS,
    scope_shutdown_reactions: &[],
    startup_actions: &[],
    timer_startup_actions: &STARTUP_ACTIONS,
    shutdown_reactions: &[],
    shutdown_actions: &[],
    routes: TinyMapView::new(&ROUTES),
    required_bindings: TinyMapView::new(&REQUIRED_BINDINGS),
    storage_bounds: &StorageBounds::new(1, 1, 1, 0, 0, 0),
};

static COALESCED_IMAGE: EnclaveImage<'static> = EnclaveImage {
    actions: TinyMapView::new(&COALESCED_ACTIONS),
    reaction_triggers: &COALESCED_REACTION_TRIGGERS,
    startup_actions: &COALESCED_STARTUP_ACTIONS,
    storage_bounds: &StorageBounds::new(1, 2, 1, 0, 0, 0),
    ..IMAGE
};

static ROUTED_IMAGE: EnclaveImage<'static> = EnclaveImage {
    ports: TinyMapView::new(&ROUTED_PORTS),
    routes: TinyMapView::new(&ROUTED_ROUTES),
    required_bindings: TinyMapView::new(&ROUTED_REQUIRED_BINDINGS),
    ..IMAGE
};
/// Bounds deadlock detection to one second outside Miri and 30 seconds under Miri, whose
/// interpreter overhead would otherwise cause false watchdog failures.
fn owned_federate_watchdog_timeout() -> std::time::Duration {
    #[cfg(miri)]
    {
        std::time::Duration::from_secs(30)
    }

    #[cfg(not(miri))]
    {
        std::time::Duration::from_secs(1)
    }
}

fn bounded<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || tx.send(run()).unwrap());
    let result = rx
        .recv_timeout(owned_federate_watchdog_timeout())
        .expect("owned Federate execution must complete within the watchdog timeout");
    worker.join().unwrap();
    result
}

#[path = "compiled_execution/distributed.rs"]
mod distributed;
#[path = "compiled_execution/failure.rs"]
mod failure;
#[path = "compiled_execution/federate.rs"]
mod federate;
#[path = "compiled_execution/lifecycle.rs"]
mod lifecycle;
#[path = "compiled_execution/modes.rs"]
mod modes;
#[path = "compiled_execution/preflight.rs"]
mod preflight;
#[path = "compiled_execution/source_sink.rs"]
mod source_sink;
#[path = "compiled_execution/timers.rs"]
mod timers;

use source_sink::*;
use timers::PERIODIC_ACTIONS;
