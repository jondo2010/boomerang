#![allow(dead_code)]
#![feature(map_first_last)]

mod event;
mod reaction;
mod scheduler;
mod trigger;

#[cfg(test)]
mod tests;

// Re-exports
pub use event::{Event, EventValue};
pub use reaction::Reaction;
pub use scheduler::{Sched, Scheduler};
pub use trigger::{QueuingPolicy, Trigger};

pub use std::time::{Duration, Instant};

pub type Index = u64;
