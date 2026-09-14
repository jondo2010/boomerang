//! Coordination reservation on standard bounded Tokio channels.
use super::{Class, HostedError, QUEUE_CAPACITY};
use boomerang_federated::channel::PAYLOAD_CAPACITY;
use std::sync::Arc;
use tokio::{
    sync::{mpsc, OwnedSemaphorePermit, Semaphore},
    time::{timeout_at, Instant},
};

/// Failure to reserve payload capacity or submit to the bounded channel.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The payload quota is exhausted or closed.
    #[error("payload reservation: {0}")]
    Payload(#[from] tokio::sync::TryAcquireError),
    /// The underlying channel is full or its receiver has closed.
    #[error("channel submission: {0}")]
    Submission(#[from] mpsc::error::TrySendError<()>),
    /// The payload semaphore closed while asynchronous admission was waiting.
    #[error("payload admission: {0}")]
    PayloadAdmission(#[from] tokio::sync::AcquireError),
    /// The receiver closed while asynchronous admission was waiting.
    #[error("channel admission: {0}")]
    Admission(#[from] mpsc::error::SendError<()>),
}

/// Accepted value and its queue-residence deadline; the permit follows the value until dequeue.
pub(super) struct Envelope<T> {
    /// Bounded application value retained in the Tokio FIFO.
    value: T,
    /// Absolute operation budget including time spent queued.
    pub deadline: Instant,
    /// Payload quota released when dequeued or rejected/dropped.
    payload: Option<OwnedSemaphorePermit>,
}
impl<T> Envelope<T> {
    /// Releases admission capacity when the consumer takes ownership.
    pub fn into_value(self) -> T {
        drop(self.payload);
        self.value
    }
}
/// A Tokio sender paired with the quota shared by its payload submissions.
pub(super) struct Sender<T> {
    /// Tokio owns message storage and wakes the consumer on submission.
    tx: mpsc::Sender<Envelope<T>>,
    /// Limits payload submissions without permitting FIFO overtaking.
    payloads: Arc<Semaphore>,
}
/// Capacity reserved before an asynchronous producer transfers its value.
pub(super) struct Reservation<'a, T> {
    /// Tokio capacity held until the value is submitted or this reservation is dropped.
    slot: mpsc::Permit<'a, Envelope<T>>,
    /// Payload quota released on cancellation, or transferred to the submitted envelope.
    payload: Option<OwnedSemaphorePermit>,
}
impl<T> Reservation<'_, T> {
    /// Transfers the value immediately after capacity admission.
    pub fn send(self, value: T, deadline: Instant) {
        self.slot.send(Envelope {
            value,
            deadline,
            payload: self.payload,
        });
    }
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            payloads: self.payloads.clone(),
        }
    }
}
/// Creates a single FIFO and its reserved coordination capacity.
pub(super) fn bounded<T>() -> (Sender<T>, mpsc::Receiver<Envelope<T>>) {
    let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
    (
        Sender {
            tx,
            payloads: Arc::new(Semaphore::new(PAYLOAD_CAPACITY)),
        },
        rx,
    )
}
impl<T> Sender<T> {
    /// Reserves asynchronous forwarding capacity without taking ownership of the value.
    pub async fn reserve(
        &self,
        class: Class,
        deadline: Instant,
    ) -> Result<Reservation<'_, T>, HostedError> {
        if Instant::now() >= deadline {
            return Err(HostedError::Lifecycle(
                "hosted channel reservation timed out",
            ));
        }
        timeout_at(deadline, async {
            let payload = if class == Class::Payload {
                Some(
                    self.payloads
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(ChannelError::from)?,
                )
            } else {
                None
            };
            let slot = self.tx.reserve().await.map_err(ChannelError::from)?;
            Ok(Reservation { slot, payload })
        })
        .await
        .map_err(|_| HostedError::Lifecycle("hosted channel reservation timed out"))?
    }
    /// Submits without waiting; Tokio owns storage, synchronization, and wakeups.
    pub fn send(&self, value: T, class: Class, deadline: Instant) -> Result<(), HostedError> {
        let payload = if class == Class::Payload {
            Some(
                self.payloads
                    .clone()
                    .try_acquire_owned()
                    .map_err(ChannelError::from)?,
            )
        } else {
            None
        };
        self.tx
            .try_send(Envelope {
                value,
                deadline,
                payload,
            })
            .map_err(|error| {
                ChannelError::Submission(match error {
                    mpsc::error::TrySendError::Full(_) => mpsc::error::TrySendError::Full(()),
                    mpsc::error::TrySendError::Closed(_) => mpsc::error::TrySendError::Closed(()),
                })
                .into()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn cancelled_reservation_preserves_capacity_and_fifo() {
        let (tx, mut rx) = bounded();
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        for i in 0..PAYLOAD_CAPACITY {
            tx.send(i, Class::Payload, deadline).unwrap();
        }
        let mut pending = Box::pin(tx.reserve(Class::Payload, deadline));
        assert!(futures_util::poll!(&mut pending).is_pending());
        drop(pending);
        assert_eq!(rx.try_recv().unwrap().into_value(), 0);
        tx.reserve(Class::Payload, deadline)
            .await
            .unwrap()
            .send(PAYLOAD_CAPACITY, deadline);
        for i in 1..=PAYLOAD_CAPACITY {
            assert_eq!(rx.try_recv().unwrap().into_value(), i);
        }
        for i in 0..QUEUE_CAPACITY {
            tx.send(i, Class::Coordination, deadline).unwrap();
        }
        let mut pending = Box::pin(tx.reserve(Class::Payload, deadline));
        assert!(futures_util::poll!(&mut pending).is_pending());
        assert_eq!(tx.payloads.available_permits(), PAYLOAD_CAPACITY - 1);
        drop(pending);
        assert_eq!(tx.payloads.available_permits(), PAYLOAD_CAPACITY);
    }
    #[tokio::test(start_paused = true)]
    async fn reservation_deadline_survives_cancellation_and_reclaims_permits() {
        let (tx, _rx) = bounded();
        let duration = std::time::Duration::from_secs(1);
        let deadline = Instant::now() + duration;
        for i in 0..QUEUE_CAPACITY {
            tx.send(i, Class::Coordination, deadline).unwrap();
        }
        let mut pending = Box::pin(tx.reserve(Class::Payload, deadline));
        assert!(futures_util::poll!(&mut pending).is_pending());
        tokio::time::advance(duration / 2).await;
        drop(pending);
        assert!(matches!(
            tx.reserve(Class::Payload, deadline).await,
            Err(HostedError::Lifecycle(
                "hosted channel reservation timed out"
            ))
        ));
        assert_eq!(Instant::now(), deadline);
        assert_eq!(tx.payloads.available_permits(), PAYLOAD_CAPACITY);
        assert!(tx.reserve(Class::Coordination, deadline).await.is_err());
    }
    #[test]
    fn payload_reservation_preserves_fifo_and_releases_at_dequeue() {
        let (tx, mut rx) = bounded();
        let deadline = Instant::now();
        for i in 0..PAYLOAD_CAPACITY {
            tx.send(i, Class::Payload, deadline).unwrap();
        }
        assert!(tx.send(99, Class::Payload, deadline).is_err());
        for i in PAYLOAD_CAPACITY..QUEUE_CAPACITY {
            tx.send(i, Class::Coordination, deadline).unwrap();
        }
        assert!(tx.send(99, Class::Coordination, deadline).is_err());
        assert_eq!(rx.try_recv().unwrap().into_value(), 0);
        tx.send(QUEUE_CAPACITY, Class::Payload, deadline).unwrap();
        for i in 1..=QUEUE_CAPACITY {
            assert_eq!(rx.try_recv().unwrap().into_value(), i);
        }
        rx.close();
        assert!(tx.send(99, Class::Coordination, deadline).is_err());
    }
}
