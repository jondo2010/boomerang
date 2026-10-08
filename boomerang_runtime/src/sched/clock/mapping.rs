//! Legacy action, event, and route mapping at the physical/logical time boundary.
//!
//! These helpers retain native compatibility while applying checked ordering for
//! manual execution. They are separate from the clock's read/wait responsibilities.

use super::RuntimeClock;
#[cfg(feature = "external-clock")]
use crate::{physical_clock::after_current_tag, Duration};
use crate::{physical_time::PhysicalClockError, Tag};

/// Maps a physical action delay while preserving native delay and manual microstep semantics.
#[cfg(feature = "external-clock")]
pub(crate) fn action_tag(
    clock: &RuntimeClock,
    origin: std::time::Instant,
    current: Tag,
    minimum: Duration,
    delay: Duration,
) -> Result<Tag, PhysicalClockError> {
    match clock {
        RuntimeClock::Native => {
            Ok(Tag::from_physical_time(origin, std::time::Instant::now()).delay(minimum + delay))
        }
        RuntimeClock::Manual { clock, .. } => {
            let delay = minimum
                .checked_add(delay)
                .ok_or(PhysicalClockError::Overflow)?;
            let tag = clock.now()?.to_tag(delay)?;
            after_current_tag(
                if delay.is_zero() {
                    Tag::new(tag.offset(), 1)
                } else {
                    tag
                },
                current,
            )
        }
    }
}

/// Rejects exhausted manual action cursors while retaining native saturation behavior.
#[cfg(feature = "external-clock")]
pub(crate) fn check_action_tag(clock: &RuntimeClock, tag: Tag) -> Result<Tag, PhysicalClockError> {
    if matches!(clock, RuntimeClock::Manual { .. }) && tag.microstep() == usize::MAX {
        Err(PhysicalClockError::Overflow)
    } else {
        Ok(tag)
    }
}

/// Normalizes and reserves manual event tags, retaining a terminal mapping failure.
/// Native events keep their original tag and do not invoke the reservation callback.
pub(crate) fn event_tag(
    clock: &RuntimeClock,
    tag: Tag,
    _current: Tag,
    _reserve: impl FnOnce(Tag) -> Result<Tag, PhysicalClockError>,
) -> Result<Tag, PhysicalClockError> {
    match clock {
        RuntimeClock::Native => Ok(tag),
        #[cfg(feature = "external-clock")]
        RuntimeClock::Manual { clock, .. } => {
            let result = after_current_tag(tag, _current).and_then(_reserve);
            if let Err(error) = result {
                clock.latch(error);
            }
            result
        }
    }
}

/// Converts a physical route delay while preserving the existing native error category.
#[cfg(feature = "external-clock")]
pub(crate) fn route_time(
    clock: &RuntimeClock,
    delay: std::time::Duration,
    host: impl FnOnce() -> Result<std::time::Instant, crate::OwnedStorageError>,
) -> Result<std::time::Instant, crate::OwnedStorageError> {
    match clock {
        RuntimeClock::Native => host(),
        RuntimeClock::Manual { .. } => clock.instant_after(delay).map_err(Into::into),
    }
}
