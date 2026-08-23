//! Shared tracing vocabulary and helpers for runtime instrumentation.

/// Stable tracing target used by runtime instrumentation.
pub const TRACE_TARGET: &str = "boomerang::trace";

pub(crate) fn enabled() -> bool {
    tracing::enabled!(target: TRACE_TARGET, tracing::Level::TRACE)
}

#[inline]
pub(crate) fn collect_if_enabled<T>(collect: impl FnOnce() -> T) -> Option<T> {
    enabled().then(collect)
}

#[doc(hidden)]
pub fn logical_ns(tag: crate::Tag) -> u64 {
    let nanoseconds = tag.offset().whole_nanoseconds();
    if nanoseconds.is_negative() {
        0
    } else {
        u64::try_from(nanoseconds).unwrap_or(u64::MAX)
    }
}

#[doc(hidden)]
pub fn microstep(tag: crate::Tag) -> u64 {
    u64::try_from(tag.microstep()).expect("tag microstep does not fit in u64")
}

/// Stable event names emitted by runtime instrumentation.
pub mod event {
    pub const SCHEDULER_THREAD: &str = "scheduler_thread";
    pub const ASYNC_INGRESS: &str = "async_ingress";
    pub const TAG_PROCESS: &str = "tag_process";
    pub const REACTION_EXECUTE: &str = "reaction_execute";
    pub const ACTION_SCHEDULE: &str = "action_schedule";
    /// A reaction mutably accessed an output port through `OutputRef`.
    ///
    /// Rust's `DerefMut` boundary observes mutable access, not whether the caller subsequently
    /// assigned a different value.
    pub const PORT_WRITE: &str = "port_write";
    pub const PROPAGATION_SEND: &str = "propagation_send";
    pub const FRONTIER_PUBLISH: &str = "frontier_publish";
    pub const COORDINATION_WAIT: &str = "coordination_wait";
    pub const COORDINATION_GRANT: &str = "coordination_grant";
    pub const TAG_RELEASE: &str = "tag_release";
    pub const TAG_COMPLETE: &str = "tag_complete";
    pub const SHUTDOWN: &str = "shutdown";
    pub const DIAGNOSTIC: &str = "diagnostic";
}
