use super::{Duration, Instant};
use derive_more::Display;

#[derive(Debug, Display, PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Hash)]
#[display(fmt = "[{:?}+{}]", offset, micro_step)]
pub struct Tag {
    /// Offset from origin t0 of logical time
    offset: Duration,
    /// Superdense-timestep
    micro_step: usize,
}

impl Tag {
    /// Create a new Tag given an offset from the origin, and a microstep
    pub fn new(offset: Duration, micro_step: usize) -> Tag {
        Self { offset, micro_step }
    }

    pub fn absolute(t0: Instant, instant: Instant) -> Self {
        Self {
            offset: instant - t0,
            micro_step: 0,
        }
    }

    pub fn now(t0: Instant) -> Self {
        Self {
            offset: Instant::now() - t0,
            micro_step: 0,
        }
    }

    /// Create a instant given the origin
    pub fn to_logical_time(&self, origin: Instant) -> Instant {
        origin + self.offset
    }

    /// Create a new Tag offset from the current.
    pub fn delay(&self, offset: Option<Duration>) -> Self {
        if let Some(offset) = offset {
            Self {
                offset: self.offset + offset,
                micro_step: 0,
            }
        } else {
            Self {
                offset: self.offset,
                micro_step: self.micro_step + 1,
            }
        }
    }

    // pub fn difference(&self, other: &Tag) -> (Duration, usize) {
    //    (
    //        self.offset.saturating_duration_since(other.offset),
    //        self.micro_step - other.micro_step,
    //    )
    //}
}

#[derive(Debug, Display, PartialEq, Eq, PartialOrd, Ord, Clone)]
#[display(fmt = "[{:?}, {}]", "time_point.elapsed()", micro_step)]
pub struct LogicalTime {
    time_point: Instant,
    micro_step: usize,
}

impl LogicalTime {
    pub fn new() -> Self {
        Self {
            time_point: Instant::now(),
            micro_step: 0,
        }
    }

    pub fn get_time_point(&self) -> Instant {
        self.time_point
    }

    pub fn get_micro_step(&self) -> usize {
        self.micro_step
    }

    pub fn advance_to(&mut self, tag: &Tag) {
        // assert!((self as &Self) < &tag.0);
        // self.time_point = tag.offset;
        self.micro_step = tag.micro_step;
    }
}
