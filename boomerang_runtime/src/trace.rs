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

macro_rules! trace_vocabulary {
    ($(#[$enum_doc:meta])* $name:ident { $($(#[$variant_doc:meta])* $variant:ident = $code:literal => $text:literal),+ $(,)? }) => {
        $(#[$enum_doc])*
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
        #[repr(u64)]
        pub enum $name { $($(#[$variant_doc])* $variant = $code),+ }

        impl $name {
            /// Returns the stable text stored in Rerun recordings.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }

        impl TryFrom<u64> for $name {
            type Error = u64;
            fn try_from(value: u64) -> Result<Self, Self::Error> {
                match value { $($code => Ok(Self::$variant),)+ other => Err(other) }
            }
        }
    };
}

trace_vocabulary! {
    /// Closed set of Boomerang trace events. Numeric codes are stable tracing schema values.
    TraceEvent {
        /// Scheduler worker lifetime span.
        SchedulerThread = 1 => "scheduler_thread",
        /// Asynchronous runtime ingress.
        AsyncIngress = 2 => "async_ingress",
        /// Processing of one logical tag.
        TagProcess = 3 => "tag_process",
        /// Execution of one reaction.
        ReactionExecute = 4 => "reaction_execute",
        /// Scheduling or rebasing an action.
        ActionSchedule = 5 => "action_schedule",
        /// Mutable access to an output port through `OutputRef`.
        PortWrite = 6 => "port_write",
        /// Propagation from a reaction to another runtime endpoint.
        PropagationSend = 7 => "propagation_send",
        /// Publication of a logical-time frontier.
        FrontierPublish = 8 => "frontier_publish",
        /// Waiting for logical-time coordination.
        CoordinationWait = 9 => "coordination_wait",
        /// Result of logical-time coordination.
        CoordinationGrant = 10 => "coordination_grant",
        /// Release of a tag to a downstream enclave.
        TagRelease = 11 => "tag_release",
        /// Completion of a processed tag.
        TagComplete = 12 => "tag_complete",
        /// Scheduler shutdown.
        Shutdown = 13 => "shutdown",
        /// Runtime diagnostic.
        Diagnostic = 14 => "diagnostic",
    }
}

trace_vocabulary! {
    /// Closed kinds used to refine Boomerang trace events.
    TraceKind {
        /// A logical action or propagation.
        Logical = 1 => "logical",
        /// A physical action or propagation.
        Physical = 2 => "physical",
        /// A shutdown ingress.
        Shutdown = 3 => "shutdown",
        /// A provisional downstream tag release.
        ProvisionalRelease = 4 => "provisional_release",
    }
}

trace_vocabulary! {
    /// Closed lifecycle states used by Boomerang trace events.
    TraceState {
        /// A scheduler thread is running.
        Running = 1 => "running",
        /// A tag is being processed.
        Processing = 2 => "processing",
        /// A reaction has begun.
        Begin = 3 => "begin",
        /// Coordination is waiting.
        Waiting = 4 => "waiting",
        /// A candidate frontier.
        Candidate = 5 => "candidate",
        /// An idle frontier.
        Idle = 6 => "idle",
        /// A finished frontier.
        Finished = 7 => "finished",
        /// Shutdown is complete.
        Complete = 8 => "complete",
        /// A runtime error diagnostic.
        RuntimeError = 9 => "runtime_error",
    }
}

trace_vocabulary! {
    /// Closed outcomes produced by Boomerang trace events and spans.
    TraceOutcome {
        /// The operation was accepted.
        Accepted = 1 => "accepted",
        /// The operation failed.
        Failed = 2 => "failed",
        /// An event in the past was ignored.
        IgnoredPast = 3 => "ignored_past",
        /// An action was scheduled.
        Scheduled = 4 => "scheduled",
        /// A startup action was installed.
        Startup = 5 => "startup",
        /// A scheduled action was rebased.
        Rebased = 6 => "rebased",
        /// Mutable port access occurred.
        MutableAccess = 7 => "mutable_access",
        /// A frontier was published.
        Published = 8 => "published",
        /// Coordination granted the tag.
        Granted = 9 => "granted",
        /// Coordination was interrupted locally.
        InterruptedLocal = 10 => "interrupted_local",
        /// Coordination was interrupted externally.
        InterruptedExternal = 11 => "interrupted_external",
        /// Tag processing completed.
        Completed = 12 => "completed",
        /// Shutdown succeeded.
        Success = 13 => "success",
    }
}
