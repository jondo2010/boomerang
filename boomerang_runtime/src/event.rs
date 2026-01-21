use std::fmt::{Debug, Display};

use crate::{ActionKey, Duration, EnclaveKey, ReactionSet, ReactorData, Tag};

/// `ScheduledEvent` is used internally by the scheduler loop in the event queue. The dependent reactions are already expanded into a single reaction set.
#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    /// The [`Tag`] at which the reactions in this event should be executed.
    pub(crate) tag: Tag,
    /// The set of Reactions to be executed at this tag.
    pub(crate) reactions: ReactionSet,
    /// Whether the scheduler should terminate after processing this event.
    pub(crate) terminal: bool,
}

impl Display for ScheduledEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "L[tag={},terminal={}]", self.tag, self.terminal)
    }
}

impl Eq for ScheduledEvent {}

impl PartialEq for ScheduledEvent {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.terminal == other.terminal
    }
}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tag
            .cmp(&other.tag)
            .then(self.terminal.cmp(&other.terminal))
            .reverse()
    }
}

/// `AsyncEvent` is used to inject events into the scheduler from outside of the normal event loop.
pub enum AsyncEvent {
    /// A release event is used by upstream enclaves to signal that they have completed processing the tag.
    TagRelease {
        /// The key of the enclave that is releasing the `Tag``.
        enclave: EnclaveKey,
        /// The tag that is being released.
        tag: Tag,
    },
    /// An empty event is used by upstream enclaves to signal that they are ready to process the next event.
    TagReleaseProvisional {
        /// The key of the enclave that is waiting
        enclave: EnclaveKey,
        /// The tag that is being waited on.
        tag: Tag,
    },
    /// A Logical event has its `tag` set to the current logical time (+ an optional delay).
    Logical {
        /// The tag at which the Action should be executed
        tag: Tag,
        /// The key of the action that triggered this event.
        key: ActionKey,
        /// The value associated with this event.
        value: Box<dyn ReactorData>,
    },

    /// A Physical event has its `tag` set to the current physical time (+ an optional delay).
    Physical {
        /// The instant at which the Action should be executed
        time: std::time::Instant,
        /// The [`ActionKey`] of the action that triggered this event.
        key: ActionKey,
        /// The value associated with this event.
        value: Box<dyn ReactorData>,
    },

    /// The scheduler should terminate after processing this event.
    Shutdown {
        /// The delay after which the scheduler should terminate.
        delay: Duration,
    },
}

impl Debug for AsyncEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TagRelease { enclave, tag } => f
                .debug_struct("TagRelease")
                .field("enclave", enclave)
                .field("tag", tag)
                .finish(),
            Self::TagReleaseProvisional { enclave, tag } => f
                .debug_struct("TagReleaseProvisional")
                .field("enclave", enclave)
                .field("tag", tag)
                .finish(),
            Self::Logical { tag, key, value } => f
                .debug_struct("Logical")
                .field("tag", tag)
                .field("key", key)
                .field(
                    "value",
                    &format!("Box<{}>", std::any::type_name_of_val(&**value)),
                )
                .finish(),
            Self::Physical { time, key, value } => f
                .debug_struct("Physical")
                .field("time", time)
                .field("key", key)
                .field(
                    "value",
                    &format!("Box<{}>", std::any::type_name_of_val(&**value)),
                )
                .finish(),
            Self::Shutdown { delay } => f.debug_struct("Shutdown").field("delay", delay).finish(),
        }
    }
}

impl Display for AsyncEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsyncEvent::TagRelease { enclave, tag } => {
                write!(f, "TagRelease[enclave={enclave:?},tag={tag:.3}]")
            }
            AsyncEvent::TagReleaseProvisional { enclave, tag } => {
                write!(f, "TagReleaseProvisional[enclave={enclave:?},tag={tag:.3}]")
            }
            AsyncEvent::Logical { tag, key, value: _ } => {
                write!(f, "Logical[tag={tag:.3},key={key:?},value=..]",)
            }
            AsyncEvent::Physical {
                time,
                key,
                value: _,
            } => {
                write!(f, "Physical[tag={time:?},key={key:?},value=..]",)
            }
            AsyncEvent::Shutdown { delay } => {
                write!(f, "Shutdown[delay={delay:.3}]")
            }
        }
    }
}

impl AsyncEvent {
    /// Create a release event.
    pub(crate) fn release(enclave: EnclaveKey, tag: Tag) -> Self {
        AsyncEvent::TagRelease { enclave, tag }
    }

    /// Create a provisional release event.
    pub(crate) fn provisional(enclave: EnclaveKey, tag: Tag) -> Self {
        AsyncEvent::TagReleaseProvisional { enclave, tag }
    }

    /// Create a logical event.
    #[allow(dead_code)]
    pub(crate) fn logical(key: ActionKey, tag: Tag, value: Box<dyn ReactorData>) -> Self {
        AsyncEvent::Logical { tag, key, value }
    }

    /// Create a physical event.
    pub(crate) fn physical(
        key: ActionKey,
        time: std::time::Instant,
        value: Box<dyn ReactorData>,
    ) -> Self {
        AsyncEvent::Physical { time, key, value }
    }

    /// Create a shutdown event.
    pub(crate) fn shutdown(delay: Duration) -> Self {
        AsyncEvent::Shutdown { delay }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BinaryHeap;

    #[test]
    fn test_scheduled_event_order() {
        // ScheduledEvent is used in a BinaryHeap, which by design is a max-heap. This means that our implementation of Ord
        // must be reversed to achieve a min-heap behavior.
        // Additionally, we want to ensure that shutdown events are processed last given multiple events with the same tag.
        let mut heap = BinaryHeap::new();
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(1), 0),
            reactions: ReactionSet::default(),
            terminal: false,
        });
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(1), 0),
            reactions: ReactionSet::default(),
            terminal: true,
        });
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(0), 0),
            reactions: ReactionSet::default(),
            terminal: false,
        });

        // The top event should NOT be the shutdown event
        let ev0 = heap.pop().unwrap();
        assert_eq!(ev0.tag.offset(), Duration::seconds(0));
        assert!(!ev0.terminal);
        let ev1 = heap.pop().unwrap();
        assert!(!ev1.terminal);
        assert_eq!(ev1.tag.offset(), Duration::seconds(1));
        let ev2 = heap.pop().unwrap();
        assert!(ev2.terminal);
        assert_eq!(ev2.tag.offset(), Duration::seconds(1));
    }
}
