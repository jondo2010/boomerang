use std::{cell::RefCell, rc::Rc};

use crate::event::EventValue;
use crate::reaction::Reaction;
use crate::{Duration, Index, Instant};

/// Enumeration of different policies for handling events that succeed one another more rapidly than
/// is allowed by a physical action's min. inter-arrival time.
#[derive(Eq, PartialEq, Debug)]
pub enum QueuingPolicy {
    /// For logical actions, the policy should always be `NONE`.
    NONE,
    /// For physical actions, the default policy is `DEFER`, which is to increase the offsets of
    /// newly-scheduled events so that the min. inter-arrival time is satisfied. This means that no
    /// events will be ignored, but they will occur later. This policy has the drawback that it may
    /// cause the event queue to grow indefinitely.
    DEFER,
    /// The `DROP` policy ignores events that are scheduled too close to one another.
    DROP,
    /// The `UPDATE` policy does the following. If the time that a newly-scheduled event is in too
    /// close proximity or is still on the event queue, the value carried by that event will be
    /// updated with the value of the newly-scheduled event. If this is not possible because the
    /// original event has already been popped off the queue, the `DEFER` policy applies.
    UPDATE,
}

/// Reaction activation record to push onto the reaction queue.
#[derive(Eq, PartialEq, Debug)]
pub struct Trigger<T: EventValue> {
    /// Reactions sensitive to this trigger.
    pub reactions: Vec<Rc<Reaction<T>>>,
    /// For a logical action, this will be a minimum delay. For physical, it is the minimum
    /// interarrival time.
    pub offset: Duration,
    /// For an action, this is not used.
    pub period: Option<Duration>,
    /// Pointer to malloc'd value (or None)
    pub value: Rc<RefCell<Option<T>>>,
    /// Indicator that this denotes a physical action (i.e., to be scheduled relative to physical
    /// time).
    pub is_physical: bool,
    /// Tag of the last event that was scheduled for this action.
    pub scheduled: Option<Instant>,
    /// Indicates the policy for handling events that succeed one another more rapidly than
    /// allowable by the specified min. interarrival time. Only applies to physical actions.
    pub policy: QueuingPolicy,
}
