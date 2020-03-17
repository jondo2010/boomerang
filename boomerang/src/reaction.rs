use std::{cell::RefCell, cmp::Ordering, rc::Rc};

use super::{Duration, EventValue, Index, Sched, Trigger};

use derive_more::Display;

#[derive(Debug)]
pub struct Port<T>(T, bool);

impl<T> Port<T> {
    pub fn new(t: T) -> Self {
        Port(t, false)
    }
    pub fn get(&self) -> &T {
        &self.0
    }
    pub fn set(&mut self, t: T) {
        self.0 = t;
        self.1 = true;
    }
}

pub trait IsPresent {
    fn is_present(&self) -> bool;
    fn reset(&mut self);
}

impl<T> IsPresent for Port<T> {
    fn is_present(&self) -> bool {
        self.1
    }
    fn reset(&mut self) {
        self.1 = false;
    }
}

type OutputTriggers<V, S> = (Rc<RefCell<dyn IsPresent>>, Vec<Rc<RefCell<Trigger<V, S>>>>);

/// Reaction activation record to push onto the reaction queue.
#[derive(Display)]
#[display(fmt = "{:p} {} {}", "reactor", "index", "chain_id")]
pub struct Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    pub name: &'static str,
    /// Reaction closure
    pub reactor: Box<RefCell<dyn FnMut(&mut S) -> ()>>,
    /// Inverse priority determined by dependency analysis.
    pub index: Index,
    /// Binary encoding of the branches that this reaction has upstream in the dependency graph.
    pub chain_id: u64,
    /// Vector of tuples per Output that are sensitive to this Reaction.
    /// Each output has a list of associated triggers
    pub triggers: Vec<OutputTriggers<V, S>>,
    /// Indicator that this reaction has already started executing.
    pub running: bool,
    /// Local deadline relative to the time stamp for invocation of the reaction.
    /// Local deadline violation handler.
    pub local_deadline: Option<(Duration, Box<RefCell<dyn FnMut(&mut S) -> bool>>)>,
}

impl<V, S> Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    pub fn new(
        name: &'static str,
        reactor: Box<RefCell<dyn FnMut(&mut S) -> ()>>,
        index: Index,
        chain_id: u64,
        triggers: Vec<OutputTriggers<V, S>>,
    ) -> Self {
        Self {
            name: name,
            reactor: reactor,
            index: index,
            chain_id: chain_id,
            triggers: triggers,
            running: false,
            local_deadline: None,
        }
    }
}

impl<V, S> core::fmt::Debug for Reaction<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match *self {
            Reaction {
                name: ref __name,
                reactor: ref __reactor,
                index: ref __index,
                chain_id: ref __chain_id,
                triggers: ref __triggers,
                running: ref __running,
                local_deadline: ref __local_deadline,
                // phantom: ref __phantom,
            } => {
                let mut debug_trait_builder = f.debug_struct("Reaction");
                debug_trait_builder.field("name", &&(*__name));
                // debug_trait_builder.field("reactor", format!("{:p}", &&*__reactor));
                debug_trait_builder.field("index", &&(*__index));
                debug_trait_builder.field("chain_id", &&(*__chain_id));
                // debug_trait_builder.field("num_outputs", &&(*__num_outputs));
                // debug_trait_builder.field("triggers", &&(*__triggers));
                debug_trait_builder.field("running", &&(*__running));
                //debug_trait_builder.field("local_deadline", &&(__local_deadline.map(|deadline| deadline.0)));
                debug_trait_builder.finish()
            }
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
