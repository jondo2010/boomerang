use std::{fmt::Display, time::Duration};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Timestamps are represented as the duration since the UNIX epoch.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Timestamp(Duration);

impl Timestamp {
    pub fn now() -> Self {
        Self(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("System time before UNIX epoch"),
        )
    }

    pub fn offset(&self, offset: Duration) -> Self {
        Self(self.0 + offset)
    }

    pub fn checked_duration_since(&self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0)
    }
}

impl From<Duration> for Timestamp {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl From<Timestamp> for Duration {
    fn from(timestamp: Timestamp) -> Self {
        timestamp.0
    }
}

impl std::ops::Sub for Timestamp {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        (self.0 - rhs.0).into()
    }
}

impl std::ops::Add<Timestamp> for Timestamp {
    type Output = Self;

    fn add(self, rhs: Timestamp) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag {
    /// Offset from origin of logical time
    pub offset: Timestamp,
    /// Superdense-timestep.
    pub microstep: usize,
}

impl Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}+{}]", self.offset, self.microstep)
    }
}

impl Tag {
    /// Create a new Tag given an offset from the origin, and a microstep
    pub fn new(offset: impl Into<Timestamp>, microstep: usize) -> Tag {
        Self {
            offset: offset.into(),
            microstep,
        }
    }

    pub fn absolute(t0: Timestamp, instant: Timestamp) -> Self {
        Self {
            offset: (instant - t0).into(),
            microstep: 0,
        }
    }

    pub fn now(t0: Timestamp) -> Self {
        Self {
            offset: (Timestamp::now() - t0).into(),
            microstep: 0,
        }
    }

    /// Create a instant given the origin
    pub fn to_logical_time(&self, origin: Timestamp) -> Timestamp {
        origin + self.offset
    }

    /// Create a new Tag offset from the current.
    pub fn delay(&self, offset: Option<impl Into<Duration>>) -> Self {
        if let Some(offset) = offset {
            Self {
                offset: self.offset + Timestamp::from(offset.into()),
                microstep: 0,
            }
        } else {
            Self {
                offset: self.offset,
                microstep: self.microstep + 1,
            }
        }
    }

    pub fn get_offset(&self) -> Duration {
        self.offset.into()
    }
}
