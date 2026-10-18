//! Reaction state, action scheduling, and mode transitions.

use super::{CommonContext, SendContext};
use crate::{
    event::AsyncEvent, image::ModeIndex, keepalive, ActionCommon, ActionKey, ActionRef, BankInfo,
    Duration, EnclaveKey, ModeKey, ReactionGraph, ReactionKey, ReactorData, Tag, TransitionKind,
};

/// A mode transition requested by a reaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeTransitionRequest {
    /// The target mode.
    pub target: ModeKey,
    /// The transition type.
    pub transition: TransitionKind,
}

/// A mode transition handle supplied to a reaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeEffectRef {
    target: ModeKey,
    transition: TransitionKind,
}

impl ModeEffectRef {
    /// Create a handle for the target mode and transition type.
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

/// A mode transition handle for an owned reaction adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledModeEffectRef {
    /// The target mode index in the validated Enclave image.
    pub target: ModeIndex,
    /// The declared transition type: reset or history.
    pub transition: TransitionKind,
}

impl CompiledModeEffectRef {
    /// Request this transition after the current compiled reaction finishes.
    pub fn set(self, ctx: &mut Context) {
        ctx.set_compiled_mode_transition(self);
    }
}

/// Effects recorded by a reaction.
#[derive(Debug, Clone)]
pub(crate) struct TriggerRes {
    /// Actions and their scheduled tags.
    pub scheduled_actions: Vec<(ActionKey, Tag)>,
    /// The earliest requested shutdown tag.
    pub scheduled_shutdown: Option<Tag>,
    /// The requested mode transition.
    pub scheduled_mode: Option<ModeTransitionRequest>,
    /// The requested compiled mode transition.
    pub scheduled_compiled_mode: Option<CompiledModeEffectRef>,
}

/// An action scheduling error. The rejected request leaves existing state unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ActionScheduleError {
    /// The minimum and requested delays add up to a negative duration.
    #[error("effective action delay must be nonnegative")]
    InvalidDelay,
    /// The delay, logical offset, or microstep exceeds its supported range.
    #[error("action delay or tag cannot be represented")]
    Overflow,
    /// The selected physical clock returned an error.
    #[error(transparent)]
    PhysicalClock(#[from] crate::clock::PhysicalClockError),
}

/// Context supplied to a reaction.
///
/// Use this context to read logical time, schedule actions, and request mode transitions.
/// Use [`Self::make_send_context`] to send events from another thread.
#[derive(Debug)]
pub struct Context {
    pub(crate) clock: crate::clock::ExecutionClock,
    /// The ID of this context's Enclave.
    enclave_key: EnclaveKey,
    /// The logical tag of the current reaction.
    pub(crate) tag: Tag,
    /// The reactor bank index and number of bank members.
    pub(crate) bank_info: Option<BankInfo>,
    /// The sender for the scheduler mailbox.
    pub(crate) async_tx: crate::Sender<AsyncEvent>,
    /// The shutdown notification receiver.
    pub(crate) shutdown_rx: keepalive::Receiver,
    /// The effects recorded by the current reaction.
    pub(crate) trigger_res: TriggerRes,
}

impl Context {
    /// Create a reaction context that shares its Enclave's selected clock.
    pub(crate) fn new(
        enclave_key: EnclaveKey,
        clock: crate::clock::ExecutionClock,
        bank_info: Option<BankInfo>,
        async_tx: crate::Sender<AsyncEvent>,
        shutdown_rx: keepalive::Receiver,
    ) -> Self {
        Self {
            clock,
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

    /// Get the origin of this execution's physical timeline, which is always zero.
    /// This method does not read the clock or check whether startup is complete.
    pub fn get_start_time(&self) -> crate::clock::PhysicalInstant {
        crate::clock::PhysicalInstant::default()
    }

    /// Get the reactor bank index, if this reactor belongs to a bank.
    pub fn get_bank_index(&self) -> Option<usize> {
        self.bank_info.as_ref().map(|BankInfo { idx, .. }| *idx)
    }

    /// Get the number of bank members, if this reactor belongs to a bank.
    pub fn get_bank_total(&self) -> Option<usize> {
        self.bank_info.as_ref().map(|BankInfo { total, .. }| *total)
    }

    /// Get the current logical tag, including its microstep.
    pub fn get_tag(&self) -> Tag {
        self.tag
    }

    /// Convert the current logical tag to a physical time coordinate.
    ///
    /// This method ignores the microstep and does not read the clock.
    /// A negative offset or an offset beyond the physical range returns an overflow error.
    /// Use [`Self::get_elapsed_logical_time`] or [`Self::get_tag`] for the full logical range.
    pub fn try_get_logical_time(
        &self,
    ) -> Result<crate::clock::PhysicalInstant, crate::clock::PhysicalClockError> {
        crate::clock::PhysicalInstant::from_tag(self.tag)
    }

    /// Get the logical time elapsed since the start of the program.
    pub fn get_elapsed_logical_time(&self) -> Duration {
        self.tag.offset()
    }

    /// Get the microstep of the current logical tag.
    pub fn get_microstep(&self) -> usize {
        self.tag.microstep()
    }

    /// Create a [`SendContext`] for this Enclave.
    /// The returned context shares the selected clock and scheduler mailbox.
    /// You can move or clone it into a worker thread.
    pub fn make_send_context(&self) -> SendContext {
        SendContext {
            clock: self.clock.clone(),
            enclave_key: self.enclave_key,
            async_tx: self.async_tx.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
        }
    }

    /// Get the action value at the current logical tag, if a value is present.
    pub fn get_action_value<'a, T: ReactorData>(
        &self,
        action: &'a mut ActionRef<T>,
    ) -> Option<&'a T> {
        action.get_value_at(self.tag)
    }

    /// Schedule a logical or physical action. Panic if scheduling fails.
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

    /// Schedule a logical or physical action. Leave state unchanged if scheduling fails.
    ///
    /// Add the minimum and requested delays with overflow checking.
    /// Reject a negative total delay.
    /// Logical actions use the current tag as their time origin.
    /// Physical actions use the selected clock and advance past the current tag.
    /// Each accepted request reserves a distinct microstep before it stores the value and trigger.
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
            let mapped = self.clock.now()?.to_tag(delay)?;
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

    /// Record a transition from a validated image for the compiled scheduler adapter.
    fn set_compiled_mode_transition(&mut self, request: CompiledModeEffectRef) {
        self.trigger_res.scheduled_compiled_mode = Some(request);
    }
}

impl CommonContext for Context {
    fn try_get_physical_time(
        &self,
    ) -> Result<crate::clock::PhysicalInstant, crate::clock::PhysicalClockError> {
        self.clock.now()
    }

    fn enclave_id(&self) -> EnclaveKey {
        self.enclave_key
    }

    /// Return `true` if the scheduler has shut down.
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

    /// Send an event to the scheduler mailbox.
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

/// Build one context for each reaction.
pub(crate) fn build_reaction_contexts(
    enclave_key: EnclaveKey,
    reaction_graph: &ReactionGraph,
    clock: crate::clock::ExecutionClock,
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
                clock.clone(),
                bank_info.clone(),
                event_tx.clone(),
                shutdown_rx.clone(),
            );
            (reaction_key, ctx)
        })
        .collect()
}
