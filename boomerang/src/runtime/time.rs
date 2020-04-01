use derive_more::Display;

use super::{Duration, Instant};

#[derive(Debug, Display, PartialEq, Eq, PartialOrd, Ord, Clone)]
#[display(fmt = "[{:?}, {}]", time_point, micro_step)]
pub struct Tag {
    time_point: Instant,
    micro_step: usize,
}

impl Tag {
    pub fn new(time_point: Instant, micro_step: usize) -> Tag {
        Self {
            time_point,
            micro_step,
        }
    }

    pub fn delay(self, offset: Option<Duration>) -> Self {
        if let Some(offset) = offset {
            Self {
                time_point: self.time_point + offset,
                micro_step: 0,
            }
        } else {
            Self {
                time_point: self.time_point,
                micro_step: self.micro_step + 1,
            }
        }
    }
}

impl From<&Instant> for Tag {
    fn from(time_point: &Instant) -> Self {
        Self::new(time_point.clone(), 0)
    }
}

impl From<&LogicalTime> for Tag {
    fn from(logical_time: &LogicalTime) -> Self {
        Self::new(logical_time.time_point, logical_time.micro_step)
    }
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

    pub fn get_time_point(&self) -> &Instant {
        &self.time_point
    }

    pub fn get_micro_step(&self) -> &usize {
        &self.micro_step
    }

    pub fn advance_to(&mut self, tag: &Tag) {
        // assert!((self as &Self) < &tag.0);
        self.time_point = tag.time_point;
        self.micro_step = tag.micro_step;
    }
}
