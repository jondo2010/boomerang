use std::cmp::Ordering;

use crate::event::EventValue;
use crate::trigger::Trigger;
use crate::{Duration, Index};

pub trait Reactor: std::fmt::Debug {}

/// Reaction activation record to push onto the reaction queue.
#[derive(Debug)]
pub struct Reaction<T: EventValue> {
    /// Reaction object
    pub reactor: Box<dyn Reactor>,
    /// Inverse priority determined by dependency analysis.
    pub index: Index,
    /// Binary encoding of the branches that this reaction has upstream in the dependency graph.
    pub chain_id: u64,
    /// Number of outputs that may possibly be produced by this function.
    pub num_outputs: usize,
    /// Array of pointers to booleans indicating whether outputs were produced.
    // output_produced: bool** ,
    /// Pointer to array of ints with number of triggers per output.
    // triggered_sizes: int* ,
    /// Array of pointers to arrays of pointers to triggers triggered by each output.
    /// Each output has a list of associated triggers
    pub triggers: Vec<Vec<Box<Trigger<T>>>>,
    /// Indicator that this reaction has already started executing.
    pub running: bool,
    /// Local deadline relative to the time stamp for invocation of the reaction.
    pub local_deadline: Option<Duration>,
    /* Local deadline violation handler.
     * deadline_violation_handler: reaction_function_t , */
}

impl<T: EventValue> PartialEq for Reaction<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T: EventValue> Eq for Reaction<T> {}

impl<T: EventValue> PartialOrd for Reaction<T> {
    fn partial_cmp(&self, other: &Reaction<T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: EventValue> Ord for Reaction<T> {
    fn cmp(&self, other: &Reaction<T>) -> Ordering {
        other.index.cmp(&self.index)
    }
}
