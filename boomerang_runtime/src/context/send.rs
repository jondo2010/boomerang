//! External event delivery with blocking sends or prepared actions.
//!
//! Preparation checks time and allocates the payload once.
//! A retry uses the same timestamp and payload allocation.
//! Drop the prepared action to cancel delivery.

use super::CommonContext;
use crate::{
    clock::{PhysicalClockError, PhysicalInstant},
    keepalive, ActionCommon, AsyncEvent, Duration, EnclaveKey, ReactorData, Tag,
};
use std::marker::PhantomData;

/// A context for sending events from workers and callbacks.
///
/// You can clone this context and share it across threads.
/// It uses the execution's selected physical clock and scheduler mailbox.
#[derive(Debug, Clone)]
pub struct SendContext {
    pub(crate) clock: crate::clock::ExecutionClock,
    /// The ID of this context's Enclave.
    pub(crate) enclave_key: EnclaveKey,
    /// The sender for the scheduler mailbox.
    pub(crate) async_tx: crate::Sender<AsyncEvent>,
    /// The shutdown notification receiver.
    pub(crate) shutdown_rx: keepalive::Receiver,
}

impl CommonContext for SendContext {
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

    /// Send a shutdown request to the scheduler mailbox.
    fn schedule_shutdown(&mut self, offset: Option<Duration>) -> bool {
        let event = AsyncEvent::shutdown(offset.unwrap_or_default());
        self.async_tx.send(event).is_ok()
    }
}

/// The scheduler has stopped or its mailbox is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("scheduler mailbox is closed")]
pub struct SendError;

/// A shutdown request could not enter the scheduler mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TryShutdownError {
    /// The mailbox is full. Retry the shutdown request.
    #[error("scheduler mailbox is full")]
    Full,
    /// The scheduler has stopped or its mailbox is closed.
    #[error("scheduler mailbox is closed")]
    Closed,
}

/// A physical action value prepared for delivery.
///
/// The prepared action owns its payload, timestamp, and destination.
/// Drop it to cancel delivery before the mailbox accepts the event.
pub struct PreparedAction<T: ReactorData> {
    event: Option<AsyncEvent>,
    sender: SendContext,
    marker: PhantomData<fn() -> T>,
}
impl<T: ReactorData> std::fmt::Debug for PreparedAction<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedAction")
            .field("time", &self.time())
            .finish_non_exhaustive()
    }
}
/// An unsuccessful send that retains the prepared action.
#[derive(thiserror::Error)]
pub enum TrySendError<T: ReactorData> {
    /// The scheduler mailbox is full. Retry with the returned prepared action.
    #[error("scheduler mailbox is full")]
    Full(PreparedAction<T>),
    /// The scheduler has stopped or its mailbox is closed. The event was not sent.
    #[error("scheduler mailbox is closed")]
    Closed(PreparedAction<T>),
}
impl<T: ReactorData> std::fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full(value) => f.debug_tuple("Full").field(value).finish(),
            Self::Closed(value) => f.debug_tuple("Closed").field(value).finish(),
        }
    }
}
impl<T: ReactorData> PreparedAction<T> {
    /// Get the prepared timestamp, including the minimum and requested delays.
    pub fn time(&self) -> PhysicalInstant {
        match self.event.as_ref().expect("owned prepared event") {
            AsyncEvent::Physical { time, .. } => *time,
            _ => unreachable!("prepared actions are physical"),
        }
    }
    /// Send the event. Wait if the scheduler mailbox is full.
    ///
    /// Use the prepared timestamp and payload without reading the clock again.
    /// This method consumes the action, including when it returns [`SendError`].
    /// Use [`Self::try_send`] when you need to retain an unsuccessful send.
    /// Do not block during initialization or while the scheduler waits for this worker.
    pub fn send(mut self) -> Result<(), SendError> {
        self.sender
            .send_event(self.event.take().expect("owned prepared event"))
    }

    /// Try to send the event without blocking.
    ///
    /// This method does not read the clock or allocate a new payload.
    /// An error returns the prepared action with its timestamp and payload unchanged.
    pub fn try_send(mut self) -> Result<(), TrySendError<T>> {
        if self.sender.is_shutdown() {
            return Err(TrySendError::Closed(self));
        }
        match self.sender.async_tx.try_send_option(&mut self.event) {
            Ok(true) => Ok(()),
            Ok(false) => Err(TrySendError::Full(self)),
            Err(_) => Err(TrySendError::Closed(self)),
        }
    }
}
impl SendContext {
    /// Request shutdown without blocking.
    ///
    /// An absent offset uses zero delay. The scheduler applies the offset relative
    /// to its current logical tag when it receives the request.
    /// Retry on [`TryShutdownError::Full`]. Stop retrying on [`TryShutdownError::Closed`].
    pub fn try_shutdown(&self, offset: Option<Duration>) -> Result<(), TryShutdownError> {
        if self.is_shutdown() {
            return Err(TryShutdownError::Closed);
        }
        match self
            .async_tx
            .try_send(AsyncEvent::shutdown(offset.unwrap_or_default()))
        {
            Ok(true) => Ok(()),
            Ok(false) => Err(TryShutdownError::Full),
            Err(_) => Err(TryShutdownError::Closed),
        }
    }

    /// Send a runtime event. Wait if the mailbox is full.
    pub(crate) fn send_event(&self, event: AsyncEvent) -> Result<(), SendError> {
        if self.is_shutdown() {
            return Err(SendError);
        }
        self.async_tx.send(event).map_err(|_| SendError)
    }

    /// Send a provisional-tag release to this context's Enclave.
    pub(crate) fn release_provisional(
        &self,
        enclave: EnclaveKey,
        tag: Tag,
    ) -> Result<(), SendError> {
        self.send_event(AsyncEvent::provisional(enclave, tag))
    }

    /// Prepare a physical action value with the selected clock.
    ///
    /// Read the clock once and add the action's minimum delay and the requested delay.
    /// An absent requested delay uses zero. A clock or arithmetic error returns without sending an event.
    /// Retries keep the prepared timestamp, even if physical time advances.
    /// The scheduler can adjust the logical tag if execution has passed that time.
    /// For source-owned actions, use `crate::input::InputSourceHandle::submit`.
    ///
    /// # Panics
    /// Panics if `action` is a logical action.
    pub fn prepare_action<T: ReactorData>(
        &self,
        action: &impl ActionCommon<T>,
        value: T,
        delay: Option<Duration>,
    ) -> Result<PreparedAction<T>, PhysicalClockError> {
        Ok(PreparedAction {
            event: Some(prepare_event(self, action, value, delay)?),
            sender: self.clone(),
            marker: PhantomData,
        })
    }
}
/// Prepare an event once for either delivery mode.
fn prepare_event<T: ReactorData>(
    context: &(impl CommonContext + ?Sized),
    action: &impl ActionCommon<T>,
    value: T,
    delay: Option<Duration>,
) -> Result<AsyncEvent, PhysicalClockError> {
    use PhysicalClockError::Overflow;
    assert!(
        !action.is_logical(),
        "logical actions are not supported asynchronously"
    );
    let delay = action
        .min_delay()
        .checked_add(delay.unwrap_or_default())
        .ok_or(Overflow)?;
    let delay = std::time::Duration::try_from(delay).map_err(|_| Overflow)?;
    let time = context.try_get_physical_time()?.checked_add(delay)?;
    Ok(AsyncEvent::physical(action.key(), time, Box::new(value)))
}
