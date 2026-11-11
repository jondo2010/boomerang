//! Ordered computational boundaries shared by coordination and output observation.

use crate::{clock::PhysicalInstant, Tag};
use core::cmp::Ordering;

/// An inclusive logical bound or an all-microstep physical-time bound.
/// A time bound sorts after every microstep at that time, including `usize::MAX`.
/// A value alone establishes no admission cutoff or completion evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressBound {
    /// Includes this logical tag and all earlier tags.
    ThroughTag(
        /// Last included logical tag.
        Tag,
    ),
    /// Includes all microsteps at this physical time and all earlier times.
    ThroughTime(
        /// Last included physical time.
        PhysicalInstant,
    ),
}

impl ProgressBound {
    /// Returns nanoseconds and the optional logical microstep used for ordering.
    pub(crate) fn coordinate(self) -> (i128, Option<usize>) {
        match self {
            Self::ThroughTag(tag) => (tag.offset().whole_nanoseconds(), Some(tag.microstep())),
            Self::ThroughTime(time) => (i128::from(time.0), None),
        }
    }
}

impl Ord for ProgressBound {
    fn cmp(&self, other: &Self) -> Ordering {
        let (left, lm) = self.coordinate();
        let (right, rm) = other.coordinate();
        left.cmp(&right).then_with(|| match (lm, rm) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
    }
}

impl PartialOrd for ProgressBound {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
