//! Methods shared by reaction contexts and external send contexts.

use crate::{Duration, EnclaveKey};

/// Clock access, identity, and shutdown for [`Context`](super::Context) and [`SendContext`](super::SendContext).
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
}
