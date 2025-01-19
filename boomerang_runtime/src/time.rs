use time::ext::InstantExt;

use crate::Duration;

/// A tag is a logical time point in the system.
///
/// Internally, a Tag is represented as an offset from the origin of logical time, and a superdense-timestep.
///
/// Given a delay `d` and a `Tag` `g=(t, n)`, for any value of `n`, `g + d = (t, 0)`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tag {
    /// Offset from origin of logical time
    offset: Duration,
    /// Superdense-timestep
    microstep: usize,
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self == &Tag::NEVER {
            write!(f, "[NEVER]")
        //} else if self == &Tag::ZERO {
        //    write!(f, "[ZERO]")
        } else if self == &Tag::FOREVER {
            write!(f, "[FOREVER]")
        } else {
            write!(f, "[{}+{}]", self.offset, self.microstep)
        }
    }
}

impl Tag {
    pub const NEVER: Self = Self {
        offset: Duration::MIN,
        microstep: 0,
    };

    pub const ZERO: Self = Self {
        offset: Duration::ZERO,
        microstep: 0,
    };

    pub const FOREVER: Self = Self {
        offset: Duration::MAX,
        microstep: usize::MAX,
    };

    /// Create a new Tag given an offset from the origin, and a microstep
    pub fn new(offset: impl Into<Duration>, microstep: usize) -> Tag {
        Self {
            offset: offset.into(),
            microstep,
        }
    }

    /// Create a new Tag given a physical time and the start time
    pub fn from_physical_time(origin: std::time::Instant, time: std::time::Instant) -> Self {
        Self {
            offset: time.signed_duration_since(origin),
            microstep: 0,
        }
    }

    /// Create a instant given the origin
    pub fn to_logical_time(&self, origin: std::time::Instant) -> std::time::Instant {
        origin + self.offset
    }

    /// Create a new Tag strictly in the future from the current.
    pub fn delay(&self, offset: Duration) -> Self {
        if offset.is_zero() {
            Self {
                offset: self.offset,
                microstep: self.microstep + 1,
            }
        } else {
            Self {
                offset: self.offset + offset,
                microstep: 0,
            }
        }
    }

    /// Create a new Tag offset strictly in the past from the current.
    pub fn pre(&self, offset: Duration) -> Self {
        if offset.is_zero() {
            return self.decrement();
        }

        Self {
            offset: self.offset - offset,
            microstep: usize::MAX,
        }
    }

    pub fn offset(&self) -> Duration {
        self.offset
    }

    pub fn microstep(&self) -> usize {
        self.microstep
    }

    /// Create a new Tag minimally smaller than the current.
    pub fn decrement(&self) -> Self {
        if self.microstep == 0 {
            Self {
                offset: self.offset - Duration::nanoseconds(1),
                microstep: usize::MAX,
            }
        } else {
            Self {
                offset: self.offset,
                microstep: self.microstep - 1,
            }
        }
    }
}
