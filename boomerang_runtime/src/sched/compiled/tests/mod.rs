//! Compiled schedule, storage, and coordination integration tests.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tinymap::TinyMapRef;

use super::*;
use crate::{
    image::{
        ActionImage, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex, EnclaveImage,
        EnclaveImageView, IndexSpan, LifecycleReactionImage, ReactionImage, ReactorImage,
        RequiredBindingImage, ScopeImage, SliceRange, StateSlotIndex, StorageBounds,
        TimerStartupImage,
    },
    keepalive, AsyncEvent, CompiledModeEffectRef, Context, EnclaveBindings,
    FederateCoordinationError, ReactionBindingError, ReactionRefs, SendContext,
};

use crate::sched::federate::{
    FederateControlAuthorization, FederateIdleWait, FederateTagAcquisition, FederateTermination,
};

/// One observable scheduler-port or compiled-reaction operation.
#[derive(Clone, Copy, Debug)]
enum Call {
    /// A candidate was invalidated before publication.
    Active,
    /// The scheduler requested a tag.
    Acquire(Tag),
    /// The scheduler authorized control-only advancement.
    Authorize(Tag),
    /// The real local barrier requested release from its upstream Enclave.
    LocalBarrier(Tag),
    /// The compiled reaction executed at the recorded physical instant.
    Reaction(Tag, std::time::Instant),
    /// The scheduler entered the idle wait path.
    Wait,
    /// The scheduler completed a logical tag.
    Complete(Tag),
    /// The scheduler reported its shared logical horizon.
    Horizon(Tag),
    /// The scheduler reported successful terminal processing.
    Stopped,
    /// The scheduler reported terminal failure.
    Fail,
}

/// Scripted backend-neutral port used with real compiled image storage.
struct RecordingCoordination {
    /// Ordered operations shared with the compiled reaction binding.
    calls: Arc<Mutex<Vec<Call>>>,
    /// Acquisition results returned in request order.
    acquisitions: VecDeque<FederateTagAcquisition>,
    /// Idle-wait results returned in request order.
    waits: VecDeque<FederateIdleWait>,
    /// Control-authorization results returned in request order.
    authorizations: VecDeque<FederateControlAuthorization>,
    /// Scripted terminal result consumed after the scheduler event channel closes.
    terminal_after_close: Option<Result<Option<FederateTermination>, FederateCoordinationError>>,
    /// Reproduces the local coordinator's mailbox closure on horizon notification.
    #[cfg(feature = "hosted-output")]
    close_on_horizon: Option<crate::Receiver<AsyncEvent>>,
}

impl FederateSchedulerCoordination for RecordingCoordination {
    #[cfg(feature = "hosted-output")]
    fn supports_output_proofs(&self) -> bool {
        self.close_on_horizon.is_some()
    }

    /// Records candidate invalidation.
    fn active(&mut self) {
        self.calls.lock().unwrap().push(Call::Active);
    }

    /// Returns the next scripted idle-wait result.
    fn wait(&mut self) -> Result<FederateIdleWait, FederateCoordinationError> {
        self.calls.lock().unwrap().push(Call::Wait);
        Ok(self.waits.pop_front().expect("scripted idle wait outcome"))
    }

    /// Returns the next scripted acquisition result.
    fn acquire_tag(
        &mut self,
        tag: Tag,
    ) -> Result<FederateTagAcquisition, FederateCoordinationError> {
        self.calls.lock().unwrap().push(Call::Acquire(tag));
        Ok(self
            .acquisitions
            .pop_front()
            .expect("scripted acquisition outcome"))
    }

    /// Returns the next scripted control-authorization result.
    fn authorize_control(
        &mut self,
        tag: Tag,
    ) -> Result<FederateControlAuthorization, FederateCoordinationError> {
        self.calls.lock().unwrap().push(Call::Authorize(tag));
        Ok(self
            .authorizations
            .pop_front()
            .expect("scripted control authorization outcome"))
    }

    /// Returns the scripted Federate terminal result after scheduler wake-channel closure.
    fn terminal_after_event_channel_closed(
        &mut self,
    ) -> Result<Option<FederateTermination>, FederateCoordinationError> {
        self.terminal_after_close.take().unwrap_or(Ok(None))
    }

    /// Records logical completion.
    fn logical_tag_complete(
        &mut self,
        tag: Tag,
        _network_input: bool,
    ) -> Result<(), FederateCoordinationError> {
        self.calls.lock().unwrap().push(Call::Complete(tag));
        Ok(())
    }

    /// Records a shared logical horizon.
    fn logical_horizon_reached(&mut self, tag: Tag) {
        self.calls.lock().unwrap().push(Call::Horizon(tag));
        #[cfg(feature = "hosted-output")]
        if let Some(receiver) = &self.close_on_horizon {
            receiver.close().unwrap();
        }
    }

    /// Records successful terminal processing.
    fn participant_stopped(&mut self) {
        self.calls.lock().unwrap().push(Call::Stopped);
    }

    /// Records terminal failure.
    fn fail(&mut self) {
        self.calls.lock().unwrap().push(Call::Fail);
    }
}

/// State value required by the compiled test reactor.
struct TestState;

/// Initializes the compiled test reactor state.
fn initialize_state() -> TestState {
    TestState
}

/// Reactor table for the compiled scheduler fixture.
static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    IndexSpan::new(0, 0),
    None,
    None,
)];
/// Action table for the compiled scheduler fixture.
static ACTIONS: [ActionImage; 1] = [ActionImage::new(
    ScopeIndex::new(0),
    ActionSlotIndex::new(0),
    ActionTiming::Timer { period_nanos: None },
    SliceRange::new(0, 1),
    None,
)];
/// Reaction table for the compiled scheduler fixture.
static REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
    ReactorIndex::new(0),
    ScopeIndex::new(0),
    0,
    BindingSlotIndex::new(1),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
)];
/// Scope table for the compiled scheduler fixture.
static SCOPES: [ScopeImage; 1] = [ScopeImage::new(
    None,
    ReactorIndex::new(0),
    None,
    SliceRange::new(0, 1),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
)];
/// Trigger table for the compiled scheduler fixture.
static TRIGGERS: [LevelReactionImage; 1] = [LevelReactionImage::new(0, ReactionIndex::new(0))];
/// Scope-descendant table for the compiled scheduler fixture.
static DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
/// Timer-startup table for the compiled scheduler fixture.
static STARTUPS: [TimerStartupImage; 1] = [TimerStartupImage::new(ActionIndex::new(0), 50_000_000)];
/// Required-binding table for the compiled scheduler fixture.
static REQUIRED_BINDINGS: [RequiredBindingImage; 2] = [
    RequiredBindingImage::new(
        crate::image::BindingSlotId::new("astate"),
        BindingKind::StateInitializer,
    ),
    RequiredBindingImage::new(
        crate::image::BindingSlotId::new("breaction"),
        BindingKind::Reaction,
    ),
];
/// Enclave image assembled from the compiled scheduler fixture tables.
static IMAGE: EnclaveImage<'static> = EnclaveImage {
    enclave_id: crate::image::EnclaveId::new("enclave"),
    reactors: TinyMapRef::from_slice(&REACTORS),
    actions: TinyMapRef::from_slice(&ACTIONS),
    ports: TinyMapRef::from_slice(&[]),
    reactions: TinyMapRef::from_slice(&REACTIONS),
    modes: TinyMapRef::from_slice(&[]),
    scopes: TinyMapRef::from_slice(&SCOPES),
    reaction_triggers: &TRIGGERS,
    reaction_use_ports: &[],
    reaction_effect_ports: &[],
    reaction_actions: &[],
    reaction_modes: &[],
    scope_descendants: &DESCENDANTS,
    scope_logical_actions: &[],
    scope_timer_startups: &[],
    scope_reset_reactions: &[],
    scope_startup_reactions: &[],
    scope_shutdown_reactions: &[],
    startup_actions: &[],
    timer_startup_actions: &STARTUPS,
    shutdown_reactions: &[],
    shutdown_actions: &[],
    routes: TinyMapRef::from_slice(&[]),
    required_bindings: TinyMapRef::from_slice(&REQUIRED_BINDINGS),
    storage_bounds: &StorageBounds::new(1, 1, 1, 0, 0, 0),
};
/// Shutdown-reaction table for terminal local-barrier coverage.
static SHUTDOWN_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
    LevelReactionImage::new(0, ReactionIndex::new(0)),
    ActionIndex::new(0),
)];
/// Compiled fixture whose reaction runs only as graceful shutdown work.
static SHUTDOWN_IMAGE: EnclaveImage<'static> = EnclaveImage {
    shutdown_reactions: &SHUTDOWN_REACTIONS,
    ..IMAGE
};

#[test]
fn external_effect_fast_forward_rejects_before_callback() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let reactions = [REACTIONS[0].clone().with_external_effect(true)];
    let image = EnclaveImageView::new(EnclaveImage {
        reactions: TinyMapRef::from_slice(&reactions),
        ..IMAGE.clone()
    })
    .unwrap();
    let observed = calls.clone();
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_reaction(
            BindingSlotIndex::new(1),
            move |_: &mut Context, _, _: ReactionRefs<'_>, _| {
                observed.lock().unwrap().push("physical effect");
                Ok(crate::OutputOutcome::Committed)
            },
        );
    let mut storage = OwnedStorage::new(image, bindings).unwrap();
    let error = run_owned_scheduler(&mut storage, &Config::default().with_fast_forward(true))
        .err()
        .unwrap();
    assert!(matches!(
        error,
        SchedulerError::Execution(OwnedStorageError::ExternalEffectFastForward)
    ));
    assert!(calls.lock().unwrap().is_empty());
}

/// Builds real owned compiled storage with a configurable reaction result.
fn build_storage(calls: Arc<Mutex<Vec<Call>>>, fail_reaction: bool) -> OwnedStorage<'static> {
    build_storage_for_image(&IMAGE, calls, fail_reaction)
}

/// Builds real owned storage for a selected compiled test image.
fn build_storage_for_image(
    image: &'static EnclaveImage<'static>,
    calls: Arc<Mutex<Vec<Call>>>,
    fail_reaction: bool,
) -> OwnedStorage<'static> {
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_reaction(
            BindingSlotIndex::new(1),
            move |context: &mut Context,
                  _state: &mut dyn ReactorData,
                  _refs: ReactionRefs<'_>,
                  _effect: Option<CompiledModeEffectRef>| {
                calls
                    .lock()
                    .unwrap()
                    .push(Call::Reaction(context.get_tag(), std::time::Instant::now()));
                if fail_reaction {
                    Err(ReactionBindingError::missing("injected reaction input"))
                } else {
                    Ok(())
                }
            },
        );
    OwnedStorage::new(EnclaveImageView::new(image.clone()).unwrap(), bindings).unwrap()
}

/// Compiled admission with the host clock retains distinct work for two equal physical timestamps.
#[test]
fn host_clock_physical_events_keep_distinct_compiled_microsteps() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    /// Two mailbox slots hold both observations before scheduler startup.
    static TWO_EVENTS: EnclaveImage<'static> = EnclaveImage {
        storage_bounds: &StorageBounds::new(1, 1, 2, 0, 0, 0),
        ..IMAGE
    };
    let mut storage = build_storage_for_image(&TWO_EVENTS, calls.clone(), false);
    let origin = std::time::Instant::now();
    for _ in 0..2 {
        assert!(storage
            .scheduler_event_tx()
            .try_send(AsyncEvent::Physical {
                time: crate::clock::PhysicalInstant::default(),
                target: crate::AsyncEventTarget::Action(ActionKey::from(0)),
                value: Box::new(()),
            })
            .unwrap());
    }
    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        origin,
        EnclaveDependencies::new(EnclaveKey::default()),
        None,
    )
    .unwrap();
    let tags: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|call| match call {
            Call::Reaction(tag, _) => Some(*tag),
            _ => None,
        })
        .collect();
    assert_eq!(
        tags,
        [
            Tag::ZERO,
            Tag::new(Duration::ZERO, 1),
            Tag::new(Duration::milliseconds(50), 0)
        ]
    );
}

/// Checks that failure during event pumping prevents a subsequent execution grant.
#[cfg(feature = "hosted-manual-clock")]
#[test]
fn physical_failure_during_pump_stops_before_fast_forward_grant() {
    use crate::clock::ExecutionClock;
    use crate::clock::ManualClock;
    use crate::clock::{PhysicalClockDomainId, PhysicalClockError};
    /// Latches clock overflow when a queued test payload is discarded.
    #[derive(Debug)]
    struct FailOnDrop(ManualClock);
    impl Drop for FailOnDrop {
        fn drop(&mut self) {
            self.0.latch(PhysicalClockError::Overflow);
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage(Arc::clone(&calls), false);
    let origin = std::time::Instant::now();
    let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
    storage
        .set_clock(ExecutionClock::manual(clock.clone(), 0))
        .unwrap();
    assert!(storage
        .scheduler_event_tx()
        .try_send(AsyncEvent::Logical {
            tag: Tag::NEVER, // Rejected and dropped inside the event pump.
            target: crate::event::AsyncEventTarget::Action(ActionKey::from(0)),
            value: Box::new(FailOnDrop(clock.clone())),
        })
        .unwrap());
    let mut port = coordination(
        Arc::clone(&calls),
        [FederateTagAcquisition::Granted],
        [],
        [],
    );
    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        origin,
        EnclaveDependencies::new(EnclaveKey::default()),
        Some(&mut port),
    )
    .unwrap();
    assert_eq!(clock.now(), Err(PhysicalClockError::Overflow));
    let calls = calls.lock().unwrap();
    assert!(
        !calls.iter().any(|call| matches!(
            call,
            Call::Acquire(_) | Call::Reaction(..) | Call::Complete(_)
        )),
        "work after failure: {calls:?}"
    );
}

/// Builds a scripted coordination port over one shared call log.
fn coordination(
    calls: Arc<Mutex<Vec<Call>>>,
    acquisitions: impl IntoIterator<Item = FederateTagAcquisition>,
    waits: impl IntoIterator<Item = FederateIdleWait>,
    authorizations: impl IntoIterator<Item = FederateControlAuthorization>,
) -> RecordingCoordination {
    RecordingCoordination {
        calls,
        acquisitions: acquisitions.into_iter().collect(),
        waits: waits.into_iter().collect(),
        authorizations: authorizations.into_iter().collect(),
        terminal_after_close: None,
        #[cfg(feature = "hosted-output")]
        close_on_horizon: None,
    }
}

#[test]
/// Treats a terminal wake-channel close inside a local barrier as a graceful Federate stop.
fn terminal_local_barrier_close_stops_without_local_coordination_error() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
    let upstream = EnclaveKey::from(1);
    let (upstream_tx, upstream_rx) = kanal::unbounded();
    let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
    let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
    dependencies.add_upstream(
        upstream,
        SendContext {
            clock: Default::default(),
            enclave_key: upstream,
            async_tx: upstream_tx,
            shutdown_rx: upstream_shutdown_rx,
        },
        None,
    );
    let event_tx = storage.scheduler_event_tx();
    let closer = std::thread::spawn(move || {
        upstream_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("scheduler must enter the local barrier");
        event_tx.close().expect("scheduler event channel closes");
    });
    let mut port = coordination(
        Arc::clone(&calls),
        [FederateTagAcquisition::Granted],
        [],
        [],
    );
    port.terminal_after_close = Some(Ok(Some(FederateTermination::Graceful { tag: None })));

    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        std::time::Instant::now(),
        dependencies,
        Some(&mut port),
    )
    .unwrap();
    closer.join().unwrap();
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == Tag::ZERO)));
}

#[test]
/// Observes closed-channel terminal coordination before entering an idle wait.
fn closed_event_channel_observes_terminal_before_idle_wait() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
    let event_tx = storage.scheduler_event_tx();
    event_tx.close().unwrap();
    let mut port = coordination(Arc::clone(&calls), [], [], []);
    port.terminal_after_close = Some(Ok(Some(FederateTermination::Graceful { tag: None })));

    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        std::time::Instant::now(),
        EnclaveDependencies::new(EnclaveKey::default()),
        Some(&mut port),
    )
    .unwrap();

    let calls = calls.lock().unwrap();
    assert!(!calls
        .iter()
        .any(|call| matches!(call, Call::Active | Call::Acquire(_))));
    assert!(calls
        .iter()
        .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == Tag::ZERO)));
}

#[test]
/// Preserves an authorized logical-horizon tag through local-barrier channel closure.
fn terminal_local_barrier_close_preserves_logical_horizon() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
    let upstream = EnclaveKey::from(1);
    let (upstream_tx, upstream_rx) = kanal::unbounded();
    let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
    let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
    dependencies.add_upstream(
        upstream,
        SendContext {
            clock: Default::default(),
            enclave_key: upstream,
            async_tx: upstream_tx,
            shutdown_rx: upstream_shutdown_rx,
        },
        None,
    );
    let event_tx = storage.scheduler_event_tx();
    let closer = std::thread::spawn(move || {
        upstream_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("scheduler must enter the local barrier");
        event_tx.close().expect("scheduler event channel closes");
    });
    let horizon = Tag::new(Duration::milliseconds(50), 0);
    let mut port = coordination(
        Arc::clone(&calls),
        [FederateTagAcquisition::Granted],
        [],
        [],
    );
    port.terminal_after_close = Some(Ok(Some(FederateTermination::Graceful {
        tag: Some(horizon),
    })));

    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        std::time::Instant::now(),
        dependencies,
        Some(&mut port),
    )
    .unwrap();
    closer.join().unwrap();
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == horizon)));
}

#[test]
/// Preserves a queued logical horizon while authorizing local control-only work.
fn control_termination_preserves_logical_horizon() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
    let horizon = Tag::new(Duration::milliseconds(25), 0);
    storage
        .scheduler_event_tx()
        .send(AsyncEvent::provisional(EnclaveKey::from(1), horizon))
        .unwrap();
    let mut port = coordination(
        Arc::clone(&calls),
        [],
        [],
        [FederateControlAuthorization::LogicalHorizon(horizon)],
    );

    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        std::time::Instant::now(),
        EnclaveDependencies::new(EnclaveKey::default()),
        Some(&mut port),
    )
    .unwrap();
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == horizon)));
}

#[test]
/// Bypasses shutdown reactions when abort closes a scheduler blocked in a local barrier.
fn aborted_local_barrier_close_bypasses_shutdown_reactions() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
    let upstream = EnclaveKey::from(1);
    let (upstream_tx, upstream_rx) = kanal::unbounded();
    let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
    let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
    dependencies.add_upstream(
        upstream,
        SendContext {
            clock: Default::default(),
            enclave_key: upstream,
            async_tx: upstream_tx,
            shutdown_rx: upstream_shutdown_rx,
        },
        None,
    );
    let event_tx = storage.scheduler_event_tx();
    let closer = std::thread::spawn(move || {
        upstream_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("scheduler must enter the local barrier");
        event_tx.close().expect("scheduler event channel closes");
    });
    let mut port = coordination(
        Arc::clone(&calls),
        [FederateTagAcquisition::Granted],
        [],
        [],
    );
    port.terminal_after_close = Some(Ok(Some(FederateTermination::Abort)));

    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        std::time::Instant::now(),
        dependencies,
        Some(&mut port),
    )
    .unwrap();
    closer.join().unwrap();
    assert!(!calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| matches!(call, Call::Reaction(..))));
}

#[test]
/// Preserves a Federate failure when it closes a scheduler blocked in a local barrier.
fn terminal_local_barrier_close_preserves_federate_failure() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage(Arc::clone(&calls), false);
    let upstream = EnclaveKey::from(1);
    let (upstream_tx, upstream_rx) = kanal::unbounded();
    let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
    let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
    dependencies.add_upstream(
        upstream,
        SendContext {
            clock: Default::default(),
            enclave_key: upstream,
            async_tx: upstream_tx,
            shutdown_rx: upstream_shutdown_rx,
        },
        None,
    );
    let event_tx = storage.scheduler_event_tx();
    let closer = std::thread::spawn(move || {
        upstream_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("scheduler must enter the local barrier");
        event_tx.close().expect("scheduler event channel closes");
    });
    let mut port = coordination(calls, [FederateTagAcquisition::Granted], [], []);
    port.terminal_after_close = Some(Err(FederateCoordinationError::BackendPublish {
        message: "injected coordinator failure".to_owned(),
    }));

    let error = match run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        std::time::Instant::now(),
        dependencies,
        Some(&mut port),
    ) {
        Ok(_) => panic!("the injected Federate failure must be preserved"),
        Err(error) => error,
    };
    closer.join().unwrap();
    assert!(matches!(
        error,
        SchedulerError::FederateFailureReported { source }
            if matches!(
                *source,
                SchedulerError::FederateCoordination(
                    FederateCoordinationError::BackendPublish { ref message }
                ) if message == "injected coordinator failure"
            )
    ));
}

#[test]
/// Exercises ordering, idle, horizon, terminal, and failure ownership in compiled execution.
fn compiled_scheduler_composes_federate_horizon_with_local_barriers() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let origin = std::time::Instant::now();
    let control_tag = Tag::new(Duration::milliseconds(25), 0);
    let reaction_tag = Tag::new(Duration::milliseconds(50), 0);
    let target = origin + reaction_tag.offset();
    let mut storage = build_storage(Arc::clone(&calls), false);
    let upstream = EnclaveKey::from(1);
    let (upstream_tx, upstream_rx) = kanal::unbounded();
    let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
    let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
    dependencies.add_upstream(
        upstream,
        SendContext {
            clock: Default::default(),
            enclave_key: upstream,
            async_tx: upstream_tx,
            shutdown_rx: upstream_shutdown_rx,
        },
        None,
    );
    let event_tx = storage.scheduler_event_tx();
    event_tx
        .send(AsyncEvent::provisional(upstream, control_tag))
        .unwrap();
    let barrier_calls = Arc::clone(&calls);
    let event_thread = std::thread::spawn(move || {
        for expected in [control_tag, reaction_tag] {
            let request = upstream_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap_or_else(|error| {
                    let _ = event_tx.send(AsyncEvent::release(upstream, Tag::FOREVER));
                    panic!("local barrier did not request {expected}: {error}");
                });
            let requested = match request {
                AsyncEvent::TagReleaseProvisional { enclave, tag }
                    if enclave == EnclaveKey::default() =>
                {
                    tag
                }
                other => {
                    let _ = event_tx.send(AsyncEvent::release(upstream, Tag::FOREVER));
                    panic!("unexpected local barrier request: {other}");
                }
            };
            barrier_calls
                .lock()
                .unwrap()
                .push(Call::LocalBarrier(requested));
            event_tx
                .send(AsyncEvent::release(upstream, requested))
                .unwrap();
            assert_eq!(requested, expected);
        }
    });
    let mut port = coordination(
        Arc::clone(&calls),
        [
            FederateTagAcquisition::Granted,
            FederateTagAcquisition::Granted,
            FederateTagAcquisition::Granted,
        ],
        [FederateIdleWait::Interrupted(AsyncEvent::Shutdown {
            delay: Duration::ZERO,
        })],
        [
            FederateControlAuthorization::Authorized,
            FederateControlAuthorization::Authorized,
        ],
    );
    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_keep_alive(true),
        origin,
        dependencies,
        Some(&mut port),
    )
    .unwrap();
    event_thread.join().unwrap();
    let observed = calls.lock().unwrap();
    let reacted_at = observed.iter().find_map(|call| match call {
        Call::Reaction(tag, at) if *tag == reaction_tag => Some(*at),
        _ => None,
    });
    let authorization = observed
        .iter()
        .position(|call| matches!(call, Call::Authorize(tag) if *tag == control_tag))
        .unwrap();
    let control_barrier = observed
        .iter()
        .position(|call| matches!(call, Call::LocalBarrier(tag) if *tag == control_tag))
        .unwrap();
    let acquisition = observed
        .iter()
        .position(|call| matches!(call, Call::Acquire(tag) if *tag == reaction_tag))
        .unwrap();
    let reaction_barrier = observed
        .iter()
        .position(|call| matches!(call, Call::LocalBarrier(tag) if *tag == reaction_tag))
        .unwrap();
    let reaction = observed
        .iter()
        .position(|call| matches!(call, Call::Reaction(tag, _) if *tag == reaction_tag))
        .unwrap();
    let completion = observed
        .iter()
        .position(|call| matches!(call, Call::Complete(tag) if *tag == reaction_tag))
        .unwrap();
    let stopped = observed
        .iter()
        .position(|call| matches!(call, Call::Stopped))
        .unwrap();
    assert!(authorization < control_barrier);
    assert!(acquisition < reaction_barrier);
    assert!(reaction_barrier < reaction);
    assert!(reaction < completion);
    assert!(completion < stopped);
    assert!(!observed
        .iter()
        .any(|call| matches!(call, Call::Acquire(tag) if *tag == control_tag)));
    assert!(reacted_at.unwrap() >= target);
    assert!(observed.iter().any(|call| matches!(call, Call::Wait)));
    assert!(observed
        .iter()
        .any(|call| { matches!(call, Call::Complete(tag) if *tag == reaction_tag) }));
    drop(observed);

    calls.lock().unwrap().clear();
    let mut storage = build_storage(Arc::clone(&calls), false);
    let horizon = Tag::new(Duration::milliseconds(100), 0);
    let mut port = coordination(
        Arc::clone(&calls),
        [
            FederateTagAcquisition::Granted,
            FederateTagAcquisition::Granted,
        ],
        [FederateIdleWait::LogicalHorizon(horizon)],
        [],
    );
    let outcome = run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::milliseconds(100)),
        std::time::Instant::now(),
        EnclaveDependencies::new(EnclaveKey::default()),
        Some(&mut port),
    )
    .unwrap();
    assert_eq!(outcome.stats.processed_tags(), 2);
    let observed = calls.lock().unwrap();
    assert!(!observed
        .iter()
        .any(|call| matches!(call, Call::Horizon(tag) if *tag == horizon)));
    assert!(!observed
        .iter()
        .any(|call| matches!(call, Call::Complete(tag) if *tag == horizon)));
    drop(observed);

    calls.lock().unwrap().clear();
    let mut storage = build_storage(Arc::clone(&calls), false);
    let mut port = coordination(
        Arc::clone(&calls),
        [FederateTagAcquisition::Stopped],
        [],
        [],
    );
    run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        std::time::Instant::now(),
        EnclaveDependencies::new(EnclaveKey::default()),
        Some(&mut port),
    )
    .unwrap();
    let observed = calls.lock().unwrap();
    assert!(!observed
        .iter()
        .any(|call| matches!(call, Call::Reaction(..) | Call::Complete(_))));
    drop(observed);

    calls.lock().unwrap().clear();
    let mut storage = build_storage(Arc::clone(&calls), true);
    let mut port = coordination(
        Arc::clone(&calls),
        [FederateTagAcquisition::Granted],
        [],
        [],
    );
    let error = match run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_fast_forward(true),
        std::time::Instant::now(),
        EnclaveDependencies::new(EnclaveKey::default()),
        Some(&mut port),
    ) {
        Ok(_) => panic!("the injected compiled reaction must fail"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        SchedulerError::FederateFailureReported { source }
            if matches!(*source, SchedulerError::Execution(_))
    ));
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, Call::Fail))
            .count(),
        1
    );
}
#[cfg(feature = "hosted-output")]
mod output;
