//! Contexts for reactions and external event delivery.
//!
//! Use [`Context`] inside a reaction to read logical time and schedule actions.
//! Use [`SendContext`] to send events from a worker or callback.
//! Both types implement [`CommonContext`] and use the execution's selected physical clock.
//!
//! Call [`SendContext::prepare_action`] once for each value.
//! Use [`PreparedAction::send`] to wait for mailbox capacity.
//! Use [`PreparedAction::try_send`] to send without blocking.
//! Retry the returned [`PreparedAction`] if the scheduler mailbox is full.
//! A retry keeps the same timestamp, payload, and destination.
//! Drop the prepared action to cancel delivery.
//!

mod common;
mod reaction;
mod send;

pub use common::CommonContext;
pub(crate) use reaction::{build_reaction_contexts, TriggerRes};
pub use reaction::{
    ActionScheduleError, CompiledModeEffectRef, Context, ModeEffectRef, ModeTransitionRequest,
};
pub use send::{PreparedAction, SendContext, SendError, TrySendError, TryShutdownError};

#[cfg(test)]
mod tests;
