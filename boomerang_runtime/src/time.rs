use std::time::{Duration, Instant};

/// Timestamps are represented as the `Duration` since the UNIX epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Timestamp(Duration);

impl From<Duration> for Timestamp {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl Timestamp {
    pub const ZERO: Self = Self(Duration::ZERO);
    pub const MAX: Self = Self(Duration::MAX);

    pub fn now() -> Self {
        Self(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("System time before UNIX epoch"),
        )
    }

    pub fn offset(&self, offset: impl Into<Duration>) -> Self {
        Self(self.0 + offset.into())
    }

    pub fn checked_duration_since(&self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0)
    }
}

impl std::ops::Sub for Timestamp {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        self.0 - rhs.0
    }
}

impl std::ops::Add<Timestamp> for Timestamp {
    type Output = Self;

    fn add(self, rhs: Timestamp) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tag {
    /// Offset from origin of logical time
    offset: Timestamp,
    /// Superdense-timestep
    micro_step: usize,
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}+{}]", self.offset, self.micro_step)
    }
}

impl Tag {
    pub const NEVER: Self = Self {
        offset: Timestamp::ZERO,
        micro_step: 0,
    };

    pub const FOREVER: Self = Self {
        offset: Timestamp::MAX,
        micro_step: usize::MAX,
    };

    /// Create a new Tag given an offset from the origin, and a microstep
    pub fn new(offset: impl Into<Timestamp>, micro_step: usize) -> Tag {
        Self {
            offset: offset.into(),
            micro_step,
        }
    }

    pub fn absolute(t0: Timestamp, instant: Timestamp) -> Self {
        Self {
            offset: (instant - t0).into(),
            micro_step: 0,
        }
    }

    pub fn now(t0: Timestamp) -> Self {
        Self {
            offset: (Timestamp::now() - t0).into(),
            micro_step: 0,
        }
    }

    /// Calculate a `Tag` relative to the given origin `t0`.
    pub fn since(&self, t0: Timestamp) -> Self {
        Self {
            offset: self
                .offset
                .checked_duration_since(t0)
                .unwrap_or_default()
                .into(),
            micro_step: 0,
        }
    }

    /// Create a instant given the origin
    pub fn to_logical_time(&self, origin: Timestamp) -> Timestamp {
        origin + self.offset
    }

    /// Create a new Tag offset from the current.
    pub fn delay(&self, offset: impl Into<Timestamp>) -> Self {
        Self {
            offset: self.offset + offset.into(),
            micro_step: 0,
        }
    }

    pub fn get_offset(&self) -> Timestamp {
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
