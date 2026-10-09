use crate::{
    event::AsyncEvent, image::ModeIndex, keepalive, ActionCommon, ActionKey, ActionRef, BankInfo,
    Duration, EnclaveKey, ModeKey, ReactionGraph, ReactionKey, ReactorData, Tag, TransitionKind,
};

/// A mode transition requested by a reaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeTransitionRequest {
    pub target: ModeKey,
    pub transition: TransitionKind,
}

/// A typed mode transition effect passed into a reaction closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeEffectRef {
    target: ModeKey,
    transition: TransitionKind,
}

impl ModeEffectRef {
    pub fn new_key(target: ModeKey, transition: TransitionKind) -> Self {
        Self { target, transition }
    }

    /// Request this mode transition after the current reaction finishes.
    pub fn set(&self, ctx: &mut Context) {
        ctx.set_mode_transition(ModeTransitionRequest {
            target: self.target,
            transition: self.transition,
        });
    }
}

/// A canonical compiled mode transition effect passed only to an owned reaction adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledModeEffectRef {
    /// Dense target mode in the validated compiled Enclave image.
    pub target: ModeIndex,
    /// Reset or history semantics declared for the transition.
    pub transition: TransitionKind,
}

impl CompiledModeEffectRef {
    /// Request this canonical transition after the current compiled reaction finishes.
    pub fn set(self, ctx: &mut Context) {
        ctx.set_compiled_mode_transition(self);
    }
}

/// Result from a reaction trigger
#[derive(Debug, Clone)]
pub(crate) struct TriggerRes {
    /// Actions that have been scheduled to trigger at a future time
    pub scheduled_actions: Vec<(ActionKey, Tag)>,
    /// A shutdown was scheduled
    pub scheduled_shutdown: Option<Tag>,
    /// A mode transition was scheduled
    pub scheduled_mode: Option<ModeTransitionRequest>,
    /// A canonical compiled mode transition was scheduled.
    pub scheduled_compiled_mode: Option<CompiledModeEffectRef>,
}

/// A logical or physical action could not be scheduled without changing existing state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ActionScheduleError {
    /// The minimum and requested delays add up to a negative duration.
    #[error("effective action delay must be nonnegative")]
    InvalidDelay,
    /// Delay arithmetic, the logical offset, or the microstep cursor is exhausted.
    #[error("action delay or tag cannot be represented")]
    Overflow,
    /// The selected physical clock failed or its epoch offset could not be represented.
    #[error(transparent)]
    PhysicalClock(#[from] crate::clock::PhysicalClockError),
}

/// Scheduler context passed into reactor functions.
#[derive(Debug)]
pub struct Context {
    pub(crate) physical_clock: crate::clock::RuntimeClock,
    /// The EnclaveId of this context
    enclave_key: EnclaveKey,
    /// Logical time of the currently executing epoch
    pub(crate) tag: Tag,
    /// Bank index and node count for a multi-bank reactor
    pub(crate) bank_info: Option<BankInfo>,

    /// Channel for asynchronous events
    pub(crate) async_tx: crate::Sender<AsyncEvent>,
    /// Shutdown channel
    pub(crate) shutdown_rx: keepalive::Receiver,

    /// Trigger result
    pub(crate) trigger_res: TriggerRes,
}

/// Common methods for both `Context` and `SendContext`
pub trait CommonContext {
    /// Get this Enclave ID
    fn enclave_id(&self) -> EnclaveKey;

    /// Reads the selected execution clock as a [`crate::clock::PhysicalInstant`].
    ///
    /// Native execution measures from the host origin bound at startup; manual
    /// execution reads the driver's integer timeline directly. Native send contexts
    /// created before startup return [`crate::clock::PhysicalClockError::NotStarted`]
    /// until their scheduler binds the origin. Clock failure, closure, and overflow
    /// also return errors. Watchdogs must use host time independently.
    fn try_get_physical_time(
        &self,
    ) -> Result<crate::clock::PhysicalInstant, crate::clock::PhysicalClockError>;

    /// Has the scheduler already been shutdown?
    fn is_shutdown(&self) -> bool;

    /// Schedule a shutdown event at some future time.
    ///
    /// Returns `true` when the request was accepted and `false` when an external scheduler channel
    /// has already closed. Calls made from a scheduler-owned [`Context`] are always accepted.
    fn schedule_shutdown(&mut self, offset: Option<Duration>) -> bool;

    /// Schedule an event externally by sending it to the scheduler through a channel.
    ///
    /// Returns true if the event was successfully scheduled, false if the channel was disconnected.
    fn schedule_external(&self, event: AsyncEvent) -> bool;

    /// Try to schedule an asynchronous event without blocking
    ///
    /// Returns `Some(true)` if the event was successfully scheduled, `Some(false)` if the channel was disconnected, and `None` if the channel would have blocked.
    fn try_schedule_async(&self, event: AsyncEvent) -> Option<bool>;

    /// Schedules a physical action with checked delay in the selected execution epoch.
    /// Returns an error without publication if clock reads or delay arithmetic fail,
    /// including native reads before startup. Successful publication returns `Ok(true)`;
    /// a stopped scheduler or disconnected channel returns `Ok(false)`.
    /// A full channel blocks until space is available.
    ///
    /// # Panics
    /// Panics if `action` is logical; asynchronous action scheduling requires a physical action.
    fn try_schedule_action_async<T: ReactorData>(
        &self,
        action: &impl ActionCommon<T>,
        value: T,
        delay: Option<Duration>,
    ) -> Result<bool, crate::clock::PhysicalClockError> {
        use crate::clock::PhysicalClockError::Overflow;
        assert!(
            !action.is_logical(),
            "logical actions are not supported asynchronously"
        );
        let delay = action
            .min_delay()
            .checked_add(delay.unwrap_or_default())
            .ok_or(Overflow)?;
        let delay = std::time::Duration::try_from(delay).map_err(|_| Overflow)?;
        let time = self.try_get_physical_time()?.checked_add(delay)?;
        Ok(self.schedule_external(AsyncEvent::physical(action.key(), time, Box::new(value))))
    }

    fn release_provisional(&self, enclave: EnclaveKey, tag: Tag) -> bool {
        self.schedule_external(AsyncEvent::provisional(enclave, tag))
    }
}

impl Context {
    /// Creates a reaction context sharing its Enclave's selected clock.
    pub(crate) fn new(
        enclave_key: EnclaveKey,
        physical_clock: crate::clock::RuntimeClock,
        bank_info: Option<BankInfo>,
        async_tx: crate::Sender<AsyncEvent>,
        shutdown_rx: keepalive::Receiver,
    ) -> Self {
        Self {
            physical_clock,
            enclave_key,
            tag: Tag::NEVER,
            bank_info,
            async_tx,
            shutdown_rx,
            trigger_res: TriggerRes {
                scheduled_actions: Vec::new(),
                scheduled_shutdown: None,
                scheduled_mode: None,
                scheduled_compiled_mode: None,
            },
        }
    }

    pub(crate) fn reset_for_reaction(&mut self, tag: Tag) {
        self.tag = tag;
        self.trigger_res.scheduled_actions.clear();
        self.trigger_res.scheduled_shutdown = None;
        self.trigger_res.scheduled_mode = None;
        self.trigger_res.scheduled_compiled_mode = None;
    }

    /// Returns the origin of this execution's physical timeline (always zero).
    /// This is a coordinate, not a clock read or a startup-readiness check.
    pub fn get_start_time(&self) -> crate::clock::PhysicalInstant {
        crate::clock::PhysicalInstant::default()
    }

    /// Get the bank index for a multi-bank reactor
    pub fn get_bank_index(&self) -> Option<usize> {
        self.bank_info.as_ref().map(|BankInfo { idx, .. }| *idx)
    }

    /// Get the number of nodes in a multi-bank reactor
    pub fn get_bank_total(&self) -> Option<usize> {
        self.bank_info.as_ref().map(|BankInfo { total, .. }| *total)
    }

    pub fn get_tag(&self) -> Tag {
        self.tag
    }

    /// Projects the current logical tag onto this execution's physical timeline.
    ///
    /// Ignores the microstep and does not read the clock. Returns overflow for
    /// negative offsets or offsets beyond the physical range. Use
    /// [`Self::get_elapsed_logical_time`] or [`Self::get_tag`] for the full logical range.
    pub fn try_get_logical_time(
        &self,
    ) -> Result<crate::clock::PhysicalInstant, crate::clock::PhysicalClockError> {
        crate::clock::PhysicalInstant::from_tag(self.tag)
    }

    /// Get the logical time elapsed since the start of the program.
    pub fn get_elapsed_logical_time(&self) -> Duration {
        self.tag.offset()
    }

    pub fn get_microstep(&self) -> usize {
        self.tag.microstep()
    }

    /// Create a new SendContext that can be shared across threads.
    /// This is used to schedule asynchronous events.
    pub fn make_send_context(&self) -> SendContext {
        SendContext {
            physical_clock: self.physical_clock.clone(),
            enclave_key: self.enclave_key,
            async_tx: self.async_tx.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
        }
    }

    /// Get value for an action at the current logical time
    pub fn get_action_value<'a, T: ReactorData>(
        &self,
        action: &'a mut ActionRef<T>,
    ) -> Option<&'a T> {
        action.get_value_at(self.tag)
    }

    /// Schedules a value using the checked action-scheduling path.
    ///
    /// # Panics
    /// Panics if [`Self::try_schedule_action`] rejects the delay, tag, or clock state.
    pub fn schedule_action<T: ReactorData>(
        &mut self,
        action: &mut ActionRef<T>,
        value: T,
        delay: Option<Duration>,
    ) {
        self.try_schedule_action(action, value, delay)
            .expect("action scheduling failed");
    }

    /// Schedules a logical or physical action without changing state on rejection.
    ///
    /// The minimum and requested delays are added with overflow checking; a negative
    /// effective delay is rejected. Logical actions are relative to the current tag,
    /// while physical actions use the selected clock and advance past completed work.
    /// Both paths reserve a distinct microstep before storing the value and trigger.
    pub fn try_schedule_action<T: ReactorData>(
        &mut self,
        action: &mut ActionRef<T>,
        value: T,
        delay: Option<Duration>,
    ) -> Result<(), ActionScheduleError> {
        use ActionScheduleError::{InvalidDelay, Overflow};
        let delay = action
            .min_delay()
            .checked_add(delay.unwrap_or_default())
            .ok_or(Overflow)?;
        if delay.is_negative() {
            return Err(InvalidDelay);
        }
        let base = if action.is_logical() {
            if delay.is_zero() {
                Tag::new(
                    self.tag.offset(),
                    self.tag.microstep().checked_add(1).ok_or(Overflow)?,
                )
            } else {
                Tag::new(self.tag.offset().checked_add(delay).ok_or(Overflow)?, 0)
            }
        } else {
            let mapped = self.physical_clock.now()?.to_tag(delay)?;
            Tag::new(mapped.offset(), usize::from(delay.is_zero()))
                .checked_after(self.tag)
                .map_err(|_| Overflow)?
        };
        let tag = action.try_next_tag_for_offset(base)?;
        action.set_value(tag, value);
        self.trigger_res.scheduled_actions.push((action.key(), tag));
        Ok(())
    }

    pub(crate) fn set_mode_transition(&mut self, request: ModeTransitionRequest) {
        self.trigger_res.scheduled_mode = Some(request);
    }

    /// Records a validated-image transition for the compiled scheduler adapter.
    fn set_compiled_mode_transition(&mut self, request: CompiledModeEffectRef) {
        self.trigger_res.scheduled_compiled_mode = Some(request);
    }
}

impl CommonContext for Context {
    fn try_get_physical_time(
        &self,
    ) -> Result<crate::clock::PhysicalInstant, crate::clock::PhysicalClockError> {
        self.physical_clock.now()
    }

    fn enclave_id(&self) -> EnclaveKey {
        self.enclave_key
    }

    /// Has the scheduler already been shutdown?
    fn is_shutdown(&self) -> bool {
        self.shutdown_rx.is_shutdwon()
    }

    fn schedule_shutdown(&mut self, offset: Option<Duration>) -> bool {
        let tag = self.tag.delay(offset.unwrap_or_default());

        self.trigger_res.scheduled_shutdown = self
            .trigger_res
            .scheduled_shutdown
            .map_or(Some(tag), |prev| Some(prev.min(tag)));
        true
    }

    /// Schedule an asynchronous event
    fn schedule_external(&self, event: AsyncEvent) -> bool {
        if self.shutdown_rx.is_shutdwon() {
            return false;
        }
        self.async_tx.send(event).is_ok()
    }

    fn try_schedule_async(&self, event: AsyncEvent) -> Option<bool> {
        if self.is_shutdown() {
            return Some(false);
        }

        match self.async_tx.try_send(event) {
            Ok(true) => Some(true),
            Ok(false) => None,
            Err(_) => Some(false),
        }
    }
}

/// SendContext can be shared across threads and allows asynchronous events to be scheduled.
#[derive(Debug, Clone)]
pub struct SendContext {
    pub(crate) physical_clock: crate::clock::RuntimeClock,
    /// Enclave ID for this context
    pub(crate) enclave_key: EnclaveKey,
    /// Channel for asynchronous events
    pub(crate) async_tx: crate::Sender<AsyncEvent>,
    /// Shutdown channel
    pub(crate) shutdown_rx: keepalive::Receiver,
}

impl CommonContext for SendContext {
    fn try_get_physical_time(
        &self,
    ) -> Result<crate::clock::PhysicalInstant, crate::clock::PhysicalClockError> {
        self.physical_clock.now()
    }

    fn enclave_id(&self) -> EnclaveKey {
        self.enclave_key
    }

    /// Has the scheduler already been shutdown?
    fn is_shutdown(&self) -> bool {
        self.shutdown_rx.is_shutdwon()
    }

    /// Schedule a shutdown event at some future time.
    fn schedule_shutdown(&mut self, offset: Option<Duration>) -> bool {
        let event = AsyncEvent::shutdown(offset.unwrap_or_default());
        self.async_tx.send(event).is_ok()
    }

    /// Send an external event to the scheduler.
    fn schedule_external(&self, event: AsyncEvent) -> bool {
        if self.is_shutdown() {
            return false;
        }
        self.async_tx.send(event).is_ok()
    }

    fn try_schedule_async(&self, event: AsyncEvent) -> Option<bool> {
        if self.is_shutdown() {
            return Some(false);
        }

        match self.async_tx.try_send(event) {
            Ok(true) => Some(true),
            Ok(false) => None,
            Err(_) => Some(false),
        }
    }
}

/// Build contexts for each reaction
pub(crate) fn build_reaction_contexts(
    enclave_key: EnclaveKey,
    reaction_graph: &ReactionGraph,
    physical_clock: crate::clock::RuntimeClock,
    event_tx: crate::Sender<AsyncEvent>,
    shutdown_rx: keepalive::Receiver,
) -> tinymap::TinySecondaryMap<ReactionKey, Context> {
    reaction_graph
        .reaction_reactors
        .iter()
        .map(|(reaction_key, reactor_key)| {
            let bank_info = &reaction_graph.reactor_bank_infos[*reactor_key];
            let ctx = Context::new(
                enclave_key,
                physical_clock.clone(),
                bank_info.clone(),
                event_tx.clone(),
                shutdown_rx.clone(),
            );
            (reaction_key, ctx)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{action::Action, event::AsyncEvent, ActionKey, BaseAction, DynActionRefMut};

    /// Checks that a rejected logical scheduling attempt preserves existing payloads and triggers.
    fn assert_logical_rejected(
        current: Tag,
        minimum: Duration,
        delay: Duration,
        expected: ActionScheduleError,
    ) {
        let (tx, _rx) = kanal::unbounded();
        let (_shutdown, shutdown_rx) = keepalive::channel();
        let mut ctx = Context::new(
            EnclaveKey::from(0),
            crate::clock::RuntimeClock::native(std::time::Instant::now()),
            None,
            tx,
            shutdown_rx,
        );
        ctx.reset_for_reaction(current);
        let mut action = Action::<u32>::new("logical", ActionKey::from(0), Some(minimum), true);
        let mut action =
            ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction))
                .unwrap();
        action.set_value(Tag::ZERO, 7);
        assert_eq!(
            ctx.try_schedule_action(&mut action, 9, Some(delay)),
            Err(expected)
        );
        assert!(ctx.trigger_res.scheduled_actions.is_empty());
        assert_eq!(action.get_value_at(Tag::ZERO), Some(&7));
    }

    /// Negative effective delays must not enqueue an event before the current tag.
    #[test]
    fn checked_logical_rejects_negative_delay() {
        assert_logical_rejected(
            Tag::new(Duration::nanoseconds(10), 0),
            Duration::ZERO,
            Duration::nanoseconds(-1),
            ActionScheduleError::InvalidDelay,
        );
    }

    /// Adding the minimum and requested delays must return an error instead of panicking.
    #[test]
    fn checked_logical_rejects_delay_sum_overflow() {
        assert_logical_rejected(
            Tag::ZERO,
            Duration::nanoseconds(1),
            Duration::MAX,
            ActionScheduleError::Overflow,
        );
    }

    /// A valid delay can still exceed the remaining range of the logical offset.
    #[test]
    fn checked_logical_rejects_offset_overflow() {
        assert_logical_rejected(
            Tag::new(Duration::MAX, 0),
            Duration::ZERO,
            Duration::nanoseconds(1),
            ActionScheduleError::Overflow,
        );
    }

    /// Zero-delay scheduling cannot wrap an exhausted current microstep.
    #[test]
    fn checked_logical_rejects_microstep_overflow() {
        assert_logical_rejected(
            Tag::new(Duration::ZERO, usize::MAX),
            Duration::ZERO,
            Duration::ZERO,
            ActionScheduleError::Overflow,
        );
    }

    /// The store's saturated cursor must be rejected before attempting to insert a value.
    #[test]
    fn checked_logical_rejects_saturated_cursor() {
        assert_logical_rejected(
            Tag::new(Duration::ZERO, usize::MAX - 1),
            Duration::ZERO,
            Duration::ZERO,
            ActionScheduleError::Overflow,
        );
    }

    /// Logical delays retain their full range and repeated values retain separate microsteps.
    #[test]
    fn checked_logical_preserves_large_offsets_and_values() {
        let (tx, _rx) = kanal::unbounded();
        let (_shutdown, shutdown_rx) = keepalive::channel();
        let mut ctx = Context::new(
            EnclaveKey::from(0),
            crate::clock::RuntimeClock::native(std::time::Instant::now()),
            None,
            tx,
            shutdown_rx,
        );
        ctx.reset_for_reaction(Tag::new(Duration::nanoseconds(10), 3));
        let mut action = Action::<u32>::new(
            "logical",
            ActionKey::from(0),
            Some(Duration::nanoseconds(2)),
            true,
        );
        let mut action =
            ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction))
                .unwrap();
        let delay = Duration::seconds(20_000_000_000);
        ctx.try_schedule_action(&mut action, 1, Some(delay))
            .unwrap();
        ctx.schedule_action(&mut action, 2, Some(delay));
        let offset = Duration::seconds(20_000_000_000) + Duration::nanoseconds(12);
        assert_eq!(
            ctx.trigger_res.scheduled_actions,
            [
                (ActionKey::from(0), Tag::new(offset, 0)),
                (ActionKey::from(0), Tag::new(offset, 1))
            ]
        );
        assert_eq!(action.get_value_at(Tag::new(offset, 0)), Some(&1));
        assert_eq!(action.get_value_at(Tag::new(offset, 1)), Some(&2));
        ctx.reset_for_reaction(Tag::new(offset, 1));
        assert_eq!(ctx.get_elapsed_logical_time(), offset);
        assert_eq!(
            ctx.try_get_logical_time(),
            Err(crate::clock::PhysicalClockError::Overflow)
        );
        ctx.reset_for_reaction(Tag::new(Duration::nanoseconds(42), 7));
        assert_eq!(
            ctx.try_get_logical_time(),
            Ok(crate::clock::PhysicalInstant(42))
        );
    }

    /// Native physical actions cannot reenter a completed tag when logical time runs ahead.
    #[test]
    fn physical_actions_advance_past_current_tag_with_checked_overflow() {
        let (tx, _rx) = kanal::unbounded();
        let (_shutdown, shutdown_rx) = keepalive::channel();
        let mut ctx = Context::new(
            EnclaveKey::from(0),
            crate::clock::RuntimeClock::native(std::time::Instant::now()),
            None,
            tx,
            shutdown_rx,
        );
        let offset = Duration::seconds(1_000_000_000);
        ctx.reset_for_reaction(Tag::new(offset, 7));
        let mut action = Action::<u32>::new("physical", ActionKey::from(0), None, false);
        let mut action =
            ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction))
                .unwrap();
        ctx.try_schedule_action(&mut action, 1, None).unwrap();
        ctx.try_schedule_action(&mut action, 2, None).unwrap();
        assert_eq!(ctx.trigger_res.scheduled_actions[0].1, Tag::new(offset, 8));
        assert_eq!(ctx.trigger_res.scheduled_actions[1].1, Tag::new(offset, 9));
        ctx.reset_for_reaction(Tag::new(offset, usize::MAX));
        assert_eq!(
            ctx.try_schedule_action(&mut action, 3, None),
            Err(ActionScheduleError::Overflow)
        );
        assert!(ctx.trigger_res.scheduled_actions.is_empty());
        assert_eq!(action.get_value_at(Tag::new(offset, 8)), Some(&1));
        assert_eq!(action.get_value_at(Tag::new(offset, 9)), Some(&2));
    }

    #[test]
    fn schedule_action_advances_microsteps_for_same_delay() {
        let (async_tx, _async_rx) = kanal::unbounded::<AsyncEvent>();
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();

        let mut ctx = Context::new(
            EnclaveKey::from(0),
            crate::clock::RuntimeClock::native(std::time::Instant::now()),
            None,
            async_tx,
            shutdown_rx,
        );

        ctx.reset_for_reaction(Tag::ZERO);

        let mut action = Action::<u32>::new("test", ActionKey::from(0), None, true);
        let mut action_ref =
            ActionRef::<u32>::try_from(DynActionRefMut(&mut action as &mut dyn BaseAction))
                .expect("action ref");

        ctx.schedule_action(&mut action_ref, 1, None);
        ctx.try_schedule_action(&mut action_ref, 2, None).unwrap();

        let tags: Vec<Tag> = ctx
            .trigger_res
            .scheduled_actions
            .iter()
            .map(|(_, tag)| *tag)
            .collect();

        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].offset(), tags[1].offset());
        assert_eq!(tags[1].microstep(), tags[0].microstep() + 1);
    }

    #[test]
    fn nonblocking_sends_distinguish_acceptance_backpressure_and_disconnection() {
        let (async_tx, async_rx) = kanal::bounded::<AsyncEvent>(1);
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();
        let ctx = Context::new(
            EnclaveKey::from(0),
            crate::clock::RuntimeClock::native(std::time::Instant::now()),
            None,
            async_tx,
            shutdown_rx,
        );
        let sender = ctx.make_send_context();
        let event = || AsyncEvent::shutdown(Duration::ZERO);

        assert_eq!(ctx.try_schedule_async(event()), Some(true));
        assert_eq!(ctx.try_schedule_async(event()), None);
        assert_eq!(sender.try_schedule_async(event()), None);
        assert!(matches!(
            async_rx.recv().unwrap(),
            AsyncEvent::Shutdown { .. }
        ));
        assert_eq!(sender.try_schedule_async(event()), Some(true));
        assert!(matches!(
            async_rx.recv().unwrap(),
            AsyncEvent::Shutdown { .. }
        ));
        drop(async_rx);
        assert_eq!(ctx.try_schedule_async(event()), Some(false));
        assert_eq!(sender.try_schedule_async(event()), Some(false));
    }

    #[test]
    fn send_context_reports_closed_scheduler_when_requesting_shutdown() {
        let (async_tx, async_rx) = kanal::unbounded::<AsyncEvent>();
        drop(async_rx);
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();
        let mut ctx = SendContext {
            physical_clock: Default::default(),
            enclave_key: EnclaveKey::from(0),
            async_tx,
            shutdown_rx,
        };

        assert!(!ctx.schedule_shutdown(None));
    }
}
