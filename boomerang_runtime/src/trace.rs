use std::sync::atomic::{AtomicU64, Ordering};

/// Stable tracing target used by runtime instrumentation.
pub const TRACE_TARGET: &str = "boomerang::trace";

/// Stable event names emitted by runtime instrumentation.
pub mod event {
    pub const ASYNC_INGRESS: &str = "async_ingress";
    pub const TAG_PROCESS: &str = "tag_process";
    pub const REACTION_EXECUTE: &str = "reaction_execute";
    pub const ACTION_SCHEDULE: &str = "action_schedule";
    pub const PORT_WRITE: &str = "port_write";
    pub const PROPAGATION_SEND: &str = "propagation_send";
    pub const PROPAGATION_RECEIVE: &str = "propagation_receive";
    pub const FRONTIER_PUBLISH: &str = "frontier_publish";
    pub const COORDINATION_WAIT: &str = "coordination_wait";
    pub const COORDINATION_GRANT: &str = "coordination_grant";
    pub const TAG_RELEASE: &str = "tag_release";
    pub const TAG_COMPLETE: &str = "tag_complete";
    pub const CAUSAL_LINK: &str = "causal_link";
    pub const SHUTDOWN: &str = "shutdown";
    pub const DIAGNOSTIC: &str = "diagnostic";

    /// All stable event names in canonical order.
    pub const ALL: &[&str] = &[
        ASYNC_INGRESS,
        TAG_PROCESS,
        REACTION_EXECUTE,
        ACTION_SCHEDULE,
        PORT_WRITE,
        PROPAGATION_SEND,
        PROPAGATION_RECEIVE,
        FRONTIER_PUBLISH,
        COORDINATION_WAIT,
        COORDINATION_GRANT,
        TAG_RELEASE,
        TAG_COMPLETE,
        CAUSAL_LINK,
        SHUTDOWN,
        DIAGNOSTIC,
    ];
}

/// Allocates monotonically increasing sequence values for one trace source.
///
/// Allocation is concurrency-safe and never returns zero. Once `u64::MAX` has
/// been allocated, this allocator remains exhausted and [`Self::next`] panics
/// rather than wrapping and returning a duplicate or zero value.
#[derive(Debug, Default)]
pub struct TraceSequence(AtomicU64);

impl TraceSequence {
    /// Returns the next nonzero sequence value.
    ///
    /// # Panics
    ///
    /// Panics after `u64::MAX` has already been allocated.
    pub fn next(&self) -> u64 {
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |previous| {
                previous.checked_add(1)
            })
            .map(|previous| previous + 1)
            .expect("trace sequence exhausted")
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc, thread};

    use super::{event, TraceSequence, TRACE_TARGET};

    #[test]
    fn trace_target_is_stable() {
        assert_eq!(TRACE_TARGET, "boomerang::trace");
    }

    #[test]
    fn event_vocabulary_is_complete_ordered_and_unique() {
        let expected = [
            "async_ingress",
            "tag_process",
            "reaction_execute",
            "action_schedule",
            "port_write",
            "propagation_send",
            "propagation_receive",
            "frontier_publish",
            "coordination_wait",
            "coordination_grant",
            "tag_release",
            "tag_complete",
            "causal_link",
            "shutdown",
            "diagnostic",
        ];
        let defined = [
            event::ASYNC_INGRESS,
            event::TAG_PROCESS,
            event::REACTION_EXECUTE,
            event::ACTION_SCHEDULE,
            event::PORT_WRITE,
            event::PROPAGATION_SEND,
            event::PROPAGATION_RECEIVE,
            event::FRONTIER_PUBLISH,
            event::COORDINATION_WAIT,
            event::COORDINATION_GRANT,
            event::TAG_RELEASE,
            event::TAG_COMPLETE,
            event::CAUSAL_LINK,
            event::SHUTDOWN,
            event::DIAGNOSTIC,
        ];

        assert_eq!(defined, expected);
        assert_eq!(event::ALL, expected);
        assert_eq!(
            event::ALL.iter().copied().collect::<HashSet<_>>().len(),
            expected.len()
        );
    }

    #[test]
    fn sequence_values_are_unique_and_nonzero_across_threads() {
        const THREADS: usize = 8;
        const VALUES_PER_THREAD: usize = 1_000;

        let sequence = Arc::new(TraceSequence::default());
        let handles = (0..THREADS)
            .map(|_| {
                let sequence = Arc::clone(&sequence);
                thread::spawn(move || {
                    (0..VALUES_PER_THREAD)
                        .map(|_| sequence.next())
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();

        let values = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("allocator thread panicked"))
            .collect::<Vec<_>>();

        assert!(values.iter().all(|value| *value != 0));
        assert_eq!(
            values.iter().copied().collect::<HashSet<_>>().len(),
            values.len()
        );
    }
}
