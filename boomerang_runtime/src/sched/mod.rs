//! The Scheduler is the core of the runtime. It is responsible for executing the
//! reactions in the system, and for managing the asynchronous events that may
//! occur during the execution of the system.

use crate::{ReactionSet, Tag};

mod common;
mod context;
#[cfg(feature = "federated")]
mod fed;
#[cfg(not(feature = "federated"))]
mod nonfed;

#[cfg(feature = "federated")]
pub use fed::{Config, Receiver, Scheduler, Sender};
#[cfg(not(feature = "federated"))]
pub use nonfed::{Config, Receiver, Scheduler, Sender};

pub use common::*;
pub use context::*;

#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    pub(crate) tag: Tag,
    pub(crate) reactions: ReactionSet,
    pub(crate) terminal: bool,
}

impl std::fmt::Display for ScheduledEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[tag={},({}R),terminal={}]",
            self.tag,
            self.reactions.num_levels(),
            self.terminal
        )
    }
}

impl Eq for ScheduledEvent {}

impl PartialEq for ScheduledEvent {
    fn eq(&self, other: &Self) -> bool {
        self.tag.eq(&other.tag)
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

impl Config {
    pub fn with_fast_forward(mut self, fast_forward: bool) -> Self {
        self.fast_forward = fast_forward;
        self
    }

    pub fn with_keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }
}

/// Scheduler errors
#[derive(Debug, thiserror::Error)]
pub enum SchedError {
    #[cfg(feature = "federated")]
    #[error(transparent)]
    ClientError(#[from] boomerang_federated::client::ClientError),
}
