use std::{cell::RefCell, cmp::Ordering, marker::PhantomData};

use super::{Duration, EventValue, Index, Sched, Trigger};

use derive_more::Display;

/// Reaction activation record to push onto the reaction queue.
#[derive(Display)]
#[display(fmt = "{:p} {} {}", "reactor", "index", "chain_id")]
pub struct Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    /// Reaction closure
    pub reactor: Box<RefCell<dyn FnMut(&mut S) -> ()>>,
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
    pub triggers: Vec<Vec<Box<Trigger<V, S>>>>,
    /// Indicator that this reaction has already started executing.
    pub running: bool,
    /// Local deadline relative to the time stamp for invocation of the reaction.
    pub local_deadline: Option<Duration>,
    // Local deadline violation handler.
    // deadline_violation_handler: reaction_function_t ,
    phantom: PhantomData<S>,
}

impl<V, S> core::fmt::Debug for Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match *self {
            Reaction {
                reactor: ref __reactor,
                index: ref __index,
                chain_id: ref __chain_id,
                num_outputs: ref __num_outputs,
                triggers: ref __triggers,
                running: ref __running,
                local_deadline: ref __local_deadline,
                phantom: ref __phantom,
            } => {
                let mut debug_trait_builder = f.debug_struct("Reaction");
                // debug_trait_builder.field("reactor", format!("{:p}", &&*__reactor));
                debug_trait_builder.field("index", &&(*__index));
                debug_trait_builder.field("chain_id", &&(*__chain_id));
                debug_trait_builder.field("num_outputs", &&(*__num_outputs));
                debug_trait_builder.field("triggers", &&(*__triggers));
                debug_trait_builder.field("running", &&(*__running));
                debug_trait_builder.field("local_deadline", &&(*__local_deadline));
                debug_trait_builder.finish()
            }
        }
    }
}

impl<V, S> Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    pub fn new(
        reactor: Box<RefCell<dyn FnMut(&mut S) -> ()>>,
        index: Index,
        chain_id: u64,
    ) -> Self {
        Self {
            reactor: reactor,
            index: index,
            chain_id: chain_id,
            num_outputs: 0,
            triggers: vec![],
            running: false,
            local_deadline: None,
            phantom: PhantomData,
        }
    }
}

impl<V, S> PartialEq for Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<V, S> Eq for Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
}

impl<V, S> PartialOrd for Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    fn partial_cmp(&self, other: &Reaction<V, S>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<V, S> Ord for Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    fn cmp(&self, other: &Reaction<V, S>) -> Ordering {
        other.index.cmp(&self.index)
    }
}
