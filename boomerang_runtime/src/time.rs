use super::{Duration, Instant};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tag {
    /// Offset from origin of logical time
    offset: Duration,
    /// Superdense-timestep
    micro_step: usize,
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}+{}]", self.offset, self.micro_step)
    }
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

    pub fn get_offset(&self) -> Duration {
        self.offset
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct LogicalTime {
    time_point: Instant,
    micro_step: usize,
}

impl std::fmt::Display for LogicalTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}+{}]", self.time_point.elapsed(), self.micro_step)
    }
}

impl Default for LogicalTime {
    fn default() -> Self {
        Self {
            time_point: Instant::now(),
            micro_step: 0,
        }
    }
}

impl LogicalTime {
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
