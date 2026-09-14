//! Executor-independent rules for a reliable, ordered channel within one membership epoch.
//!
//! A projection delivers complete frames once, in submission order, or declares link failure.
//! It owns integrity, acknowledgement, retransmission and reassembly where the medium needs
//! them. Acceptance into a queue is not remote delivery. Deadlines and terminal drain belong
//! to the projection; no transport operation may change logical-time authority.

/// Maximum retained frames at each channel stage.
pub const QUEUE_CAPACITY: usize = 16;
/// Payload frames may not consume the four reserved coordination entries.
pub const PAYLOAD_CAPACITY: usize = QUEUE_CAPACITY - 4;

/// Admission class; it never authorizes reordering accepted frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Logical-time or lifecycle coordination.
    Coordination,
    /// Application data subject to the smaller admission quota.
    Payload,
}

/// Exhaustion of a fixed channel queue or its payload quota.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueueError {
    /// Accepting another frame would exceed a storage or reservation bound.
    #[error("channel queue capacity exhausted")]
    Full,
}

/// Fixed-storage FIFO with reserved coordination capacity.
///
/// The caller must bound each value before retaining it here. For wire frames this means
/// at most [`crate::wire::MAX_FRAME_BYTES`] per entry, hence at most that size times
/// [`QUEUE_CAPACITY`] per stage. Payload admission never allows coordination to overtake
/// an accepted payload: completion and grants remain behind their causally preceding data.
pub struct Queue<T> {
    /// Inline FIFO storage, including each entry's admission class.
    entries: heapless::Deque<(T, Class), QUEUE_CAPACITY>,
    /// Payload entries currently consuming the non-reserved quota.
    payloads: usize,
}
impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            entries: heapless::Deque::new(),
            payloads: 0,
        }
    }
}
impl<T> Queue<T> {
    /// Reports whether one entry of this class can be retained without exceeding either quota.
    pub fn accepts(&self, class: Class) -> bool {
        !self.entries.is_full()
            && (class == Class::Coordination || self.payloads < PAYLOAD_CAPACITY)
    }
    /// Retains one already-bounded value in submission order; rejection drops the supplied value.
    pub fn push(&mut self, value: T, class: Class) -> Result<(), QueueError> {
        if !self.accepts(class) {
            return Err(QueueError::Full);
        }
        self.entries
            .push_back((value, class))
            .map_err(|_| QueueError::Full)?;
        self.payloads += usize::from(class == Class::Payload);
        Ok(())
    }
    /// Removes the oldest value and releases its quota.
    pub fn pop(&mut self) -> Option<T> {
        let (value, class) = self.entries.pop_front()?;
        self.payloads -= usize::from(class == Class::Payload);
        Some(value)
    }
    /// Borrows the oldest value without changing FIFO order or releasing its quota.
    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.entries.front_mut().map(|(value, _)| value)
    }
    /// Returns the oldest entry's class for admission into the next bounded stage.
    pub fn front_class(&self) -> Option<Class> {
        self.entries.front().map(|(_, class)| *class)
    }
    /// Whether the stage has no retained values.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordination_reservation_preserves_fifo_and_reclaims_payload_slots() {
        let mut queue = Queue::default();
        for value in 0..PAYLOAD_CAPACITY {
            queue.push(value, Class::Payload).unwrap();
        }
        assert_eq!(queue.push(99, Class::Payload), Err(QueueError::Full));
        for value in PAYLOAD_CAPACITY..QUEUE_CAPACITY {
            queue.push(value, Class::Coordination).unwrap();
        }
        assert_eq!(queue.push(99, Class::Coordination), Err(QueueError::Full));
        for value in 0..QUEUE_CAPACITY {
            assert_eq!(queue.pop(), Some(value));
        }
        queue.push(42, Class::Payload).unwrap();
        assert_eq!(queue.pop(), Some(42));
    }
}
