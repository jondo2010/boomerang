//! Portable completion bounds and checked ordering.
/// Tag or time bound requested by an observer or established by a completion proof.
pub use crate::progress::ProgressBound as OutputBound;
#[cfg(any(feature = "hosted-output", test))]
use crate::Tag;
use crate::{
    clock::{ExecutionEpoch, PhysicalInstant},
    image::{EnclaveIndex, ReactorIndex},
};

/// A rejected bound, request, or terminated execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("output completion: {self:?}")]
pub enum OutputWaitError {
    /// The bound is negative, non-finite, or outside the physical clock range.
    InvalidBound,
    /// The reactor does not belong to the endpoint's execution owner.
    InvalidTarget,
    /// Execution identity has not yet been bound by setup.
    NotStarted,
    /// The backend cannot exclude further logical ingress through a proof bound.
    UnsupportedBackend,
    /// A request/proof bound is exhausted, the mailbox is full, or a reservation conflicts.
    Capacity,
    /// The host timeout expired; the request has been canceled.
    Timeout,
    /// Execution was aborted without a completion proof.
    Canceled,
    /// Normal shutdown occurred before the requested proof.
    Shutdown,
    /// Execution failed; effects may already have occurred.
    ExecutionFailed,
    /// The request belongs to another execution epoch.
    StaleExecution,
    /// The scheduler or its response channel was dropped.
    Disconnected,
}

impl OutputBound {
    /// Validates this value against the unsigned nanosecond execution timeline.
    pub fn validate(self) -> Result<(), OutputWaitError> {
        match self {
            Self::ThroughTag(tag) => {
                PhysicalInstant::from_tag(tag).map_err(|_| OutputWaitError::InvalidBound)?;
            }
            Self::ThroughTime(_) => {}
        }
        Ok(())
    }

    /// Projects only for existing grant arithmetic; never an executable event.
    #[cfg(feature = "hosted-output")]
    pub(crate) fn horizon(self) -> Tag {
        match self {
            Self::ThroughTag(tag) => tag,
            Self::ThroughTime(time) => Tag::new(
                crate::Duration::nanoseconds_i128(i128::from(time.0)),
                usize::MAX,
            ),
        }
    }

    /// The first executable tag strictly above this bound.
    #[cfg(any(feature = "hosted-output", test))]
    pub(crate) fn first_tag_after(self) -> Result<Tag, OutputWaitError> {
        self.validate()?;
        if let Self::ThroughTag(tag) = self {
            if let Some(microstep) = tag.microstep().checked_add(1) {
                return Ok(Tag::new(tag.offset(), microstep));
            }
        }
        let time = match self {
            Self::ThroughTag(tag) => {
                PhysicalInstant::from_tag(tag).map_err(|_| OutputWaitError::InvalidBound)?
            }
            Self::ThroughTime(time) => time,
        };
        let next = time.0.checked_add(1).ok_or(OutputWaitError::InvalidBound)?;
        Ok(Tag::new(
            crate::Duration::nanoseconds_i128(i128::from(next)),
            0,
        ))
    }

    /// Inverts one existing logical route's delay. `None` means before startup.
    #[cfg(any(feature = "hosted-output", test))]
    pub(crate) fn upstream_cut(
        self,
        delay: Option<crate::Duration>,
    ) -> Result<Option<Self>, OutputWaitError> {
        self.validate()?;
        let Some(delay) = delay else {
            return Ok(Some(self));
        };
        if delay.is_negative() {
            return Err(OutputWaitError::InvalidBound);
        }
        if delay.is_zero() {
            return Ok(match self {
                Self::ThroughTime(_) => Some(self),
                Self::ThroughTag(tag) if tag.microstep() > 0 => Some(Self::ThroughTag(Tag::new(
                    tag.offset(),
                    tag.microstep() - 1,
                ))),
                Self::ThroughTag(tag) => PhysicalInstant::from_tag(tag)
                    .map_err(|_| OutputWaitError::InvalidBound)?
                    .0
                    .checked_sub(1)
                    .map(|time| Self::ThroughTime(PhysicalInstant(time))),
            });
        }
        let time = self
            .coordinate()
            .0
            .checked_sub(delay.whole_nanoseconds())
            .ok_or(OutputWaitError::InvalidBound)?;
        if time < 0 {
            return Ok(None);
        }
        let time = u64::try_from(time).map_err(|_| OutputWaitError::InvalidBound)?;
        Ok(Some(Self::ThroughTime(PhysicalInstant(time))))
    }
}

// Construction requires the hosted observer; portable reports retain typed accessors.
#[cfg_attr(not(feature = "hosted-output"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputTarget {
    pub(crate) enclave: EnclaveIndex,
    pub(crate) reactor: ReactorIndex,
}

/// A proof issued by one execution for one directly owned reactor boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputCompletion {
    pub(crate) epoch: ExecutionEpoch,
    pub(crate) target: OutputTarget,
    pub(crate) bound: OutputBound,
}
impl OutputCompletion {
    /// Execution that established the proof.
    pub fn epoch(&self) -> ExecutionEpoch {
        self.epoch
    }
    /// Inclusive bound established by this report.
    pub fn bound(&self) -> OutputBound {
        self.bound
    }
    /// Exact compiled enclave and reactor that own the completed effects.
    pub fn target(&self) -> (EnclaveIndex, ReactorIndex) {
        (self.target.enclave, self.target.reactor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Duration;
    #[test]
    fn output_bound_orders_all_microsteps_before_the_time_cut_and_checks_range() {
        for microstep in [0, 1, usize::MAX] {
            assert!(
                OutputBound::ThroughTag(Tag::new(Duration::ZERO, microstep))
                    < OutputBound::ThroughTime(PhysicalInstant(0))
            );
        }
        assert!(
            OutputBound::ThroughTime(PhysicalInstant(0))
                < OutputBound::ThroughTag(Tag::new(Duration::nanoseconds(1), 0))
        );
        for tag in [
            Tag::NEVER,
            Tag::FOREVER,
            Tag::new(Duration::nanoseconds(-1), 0),
            Tag::new(Duration::MAX, 0),
        ] {
            assert_eq!(
                OutputBound::ThroughTag(tag).validate(),
                Err(OutputWaitError::InvalidBound)
            );
        }
        assert!(OutputBound::ThroughTime(PhysicalInstant(u64::MAX))
            .validate()
            .is_ok());
    }

    #[test]
    fn output_seal_inverts_logical_route_delay_without_truncating_microsteps() {
        use OutputBound::{ThroughTag, ThroughTime};
        let tag = ThroughTag(Tag::new(Duration::nanoseconds(5), 2));
        for (bound, delay, expected) in [
            (tag, None, Some(tag)),
            (
                tag,
                Some(Duration::ZERO),
                Some(ThroughTag(Tag::new(Duration::nanoseconds(5), 1))),
            ),
            (
                ThroughTag(Tag::new(Duration::nanoseconds(5), 0)),
                Some(Duration::ZERO),
                Some(ThroughTime(PhysicalInstant(4))),
            ),
            (
                tag,
                Some(Duration::nanoseconds(2)),
                Some(ThroughTime(PhysicalInstant(3))),
            ),
            (
                ThroughTime(PhysicalInstant(5)),
                Some(Duration::ZERO),
                Some(ThroughTime(PhysicalInstant(5))),
            ),
            (
                ThroughTime(PhysicalInstant(5)),
                Some(Duration::nanoseconds(2)),
                Some(ThroughTime(PhysicalInstant(3))),
            ),
            (
                ThroughTime(PhysicalInstant(5)),
                Some(Duration::nanoseconds(6)),
                None,
            ),
            (ThroughTag(Tag::ZERO), Some(Duration::ZERO), None),
        ] {
            assert_eq!(bound.upstream_cut(delay), Ok(expected));
        }
        assert_eq!(
            tag.upstream_cut(Some(Duration::nanoseconds(-1))),
            Err(OutputWaitError::InvalidBound)
        );
    }

    #[test]
    fn output_seal_floor_advances_physical_time_and_checks_overflow() {
        assert_eq!(
            OutputBound::ThroughTime(PhysicalInstant(5)).first_tag_after(),
            Ok(Tag::new(Duration::nanoseconds(6), 0)),
        );
        assert_eq!(
            OutputBound::ThroughTag(Tag::new(Duration::nanoseconds(5), 7)).first_tag_after(),
            Ok(Tag::new(Duration::nanoseconds(5), 8)),
        );
        assert_eq!(
            OutputBound::ThroughTag(Tag::new(Duration::nanoseconds(5), usize::MAX))
                .first_tag_after(),
            Ok(Tag::new(Duration::nanoseconds(6), 0)),
        );
        assert_eq!(
            OutputBound::ThroughTime(PhysicalInstant(u64::MAX)).first_tag_after(),
            Err(OutputWaitError::InvalidBound),
        );
    }
}
