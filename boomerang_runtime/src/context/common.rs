//! Methods shared by reaction contexts and external send contexts.

use super::send::prepare_event;
use crate::{ActionCommon, AsyncEvent, Duration, EnclaveKey, ReactorData, Tag};

/// Clock access and event delivery for [`Context`](super::Context) and [`SendContext`](super::SendContext).
pub trait CommonContext {
    /// Get the ID of this context's Enclave.
    fn enclave_id(&self) -> EnclaveKey;

    /// Read the selected physical clock.
    ///
    /// The host monotonic clock measures time from the origin set at scheduler startup.
    /// A manual clock returns the time set by its driver.
    /// Before the host origin is set, this method returns [`crate::clock::PhysicalClockError::NotStarted`].
    /// Clock failure, clock closure, and overflow also return errors.
    /// Use host time separately for watchdogs.
    fn try_get_physical_time(
        &self,
    ) -> Result<crate::clock::PhysicalInstant, crate::clock::PhysicalClockError>;

    /// Return `true` if the scheduler has shut down.
    fn is_shutdown(&self) -> bool;

    /// Request scheduler shutdown after the given offset.
    ///
    /// [`Context`](super::Context) records the request relative to the current logical tag and returns `true`.
    /// [`SendContext`](super::SendContext) sends a shutdown event to the scheduler mailbox.
    /// The scheduler applies its offset relative to the current logical tag when it receives the event.
    /// The send blocks if the mailbox is full. It returns `false` if the mailbox is closed.
    /// An absent offset uses zero delay.
    fn schedule_shutdown(&mut self, offset: Option<Duration>) -> bool;

    /// Send an event to the scheduler mailbox.
    ///
    /// The call blocks if the mailbox is full.
    /// Return `true` if the mailbox accepts the event.
    /// Return `false` if the scheduler has stopped or the mailbox is closed.
    fn schedule_external(&self, event: AsyncEvent) -> bool;

    /// Try to send an event without blocking.
    ///
    /// Return `Some(true)` if the mailbox accepts the event.
    /// Return `Some(false)` if the scheduler has stopped or the mailbox is closed.
    /// Return `None` if the mailbox is full. An unsuccessful send drops the event.
    /// Use [`SendContext::prepare_action`](super::SendContext::prepare_action) to retain an action value for retries.
    fn try_schedule_async(&self, event: AsyncEvent) -> Option<bool>;

    /// Prepare a physical action and send it to the scheduler mailbox.
    ///
    /// Preparation reads the selected clock and adds the minimum and requested delays.
    /// A clock or arithmetic error returns an error without sending the event.
    /// The call blocks if the mailbox is full.
    /// Return `Ok(true)` if the mailbox accepts the event.
    /// Return `Ok(false)` if the scheduler has stopped or the mailbox is closed.
    ///
    /// # Panics
    /// Panics if `action` is a logical action.
    fn try_schedule_action_async<T: ReactorData>(
        &self,
        action: &impl ActionCommon<T>,
        value: T,
        delay: Option<Duration>,
    ) -> Result<bool, crate::clock::PhysicalClockError> {
        Ok(self.schedule_external(prepare_event(self, action, value, delay)?))
    }

    /// Send a provisional-tag release for the given Enclave.
    ///
    /// Blocking and return behavior match [`Self::schedule_external`].
    fn release_provisional(&self, enclave: EnclaveKey, tag: Tag) -> bool {
        self.schedule_external(AsyncEvent::provisional(enclave, tag))
    }
}
