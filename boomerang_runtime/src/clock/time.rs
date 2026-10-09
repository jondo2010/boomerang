//! Integer identities and epoch-relative time, usable without a hosted runtime.

/// Stable configured identity of a physical clock domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysicalClockDomainId(pub u64);

/// Random identity of one execution, distinct across independently started hosted runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionEpoch(pub u128);

/// Checked physical-time and clock protocol failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("physical clock: {self:?}")]
pub enum PhysicalClockError {
    /// The native execution origin has not been bound at scheduler startup.
    NotStarted,
    /// The driver moved time backwards.
    Regression,
    /// Host entropy was unavailable when creating a fresh execution identity.
    EntropyUnavailable,
    /// The driver closed the clock.
    Closed,
    /// The driver explicitly failed the clock.
    Failed,
    /// A time, delay, or fresh epoch was not representable.
    Overflow,
    /// The configured clock domain does not match.
    DomainMismatch,
    /// The observation belongs to another execution.
    EpochMismatch,
    /// This clock has already been assigned to an execution.
    AlreadyUsed,
    /// A selected scheduler mailbox cannot retain even one wake notification.
    WakeCapacity,
}

/// A point on a Boomerang execution's physical timeline.
///
/// This is Boomerang's clock-selected counterpart to [`std::time::Instant`]. A
/// [`PhysicalClock`] supplies the current instant; its source may be the host
/// monotonic clock or an externally advanced simulation clock. Reading an instant
/// does not advance that clock or authorize logical execution.
///
/// # Epoch and clock domain
///
/// The coordinate is unsigned nanoseconds since the execution's physical epoch.
/// Zero (also [`Default::default()`]) is the epoch origin, not a read of current
/// time. The range is `0..=u64::MAX` nanoseconds, approximately 584 years.
/// This is neither calendar time nor a Unix timestamp.
///
/// Compare instants only within the same [`PhysicalClockDomainId`] and
/// [`ExecutionEpoch`]. The value stores neither identity: equality and ordering
/// compare only the numeric coordinate and cannot detect unrelated clocks or
/// runs. Adapters must carry and validate domain and epoch separately when
/// observations cross an execution boundary. Clock implementations enforce
/// monotonic reads; constructing a value does not establish that guarantee.
///
/// # Durations and host time
///
/// An instant is a time point. Add a [`core::time::Duration`] using
/// [`Self::checked_add`]; conversion methods expose the duration from the epoch
/// for adapter interchange. No ambient `now()` is provided because the caller
/// must select a clock. Host watchdogs and transport timeouts continue to use
/// [`std::time::Instant`] independently of simulated time.
///
/// ```
/// use boomerang_runtime::clock::{PhysicalClockError, PhysicalInstant};
/// use core::time::Duration;
///
/// let acquired = PhysicalInstant::from_duration(Duration::from_secs(2))?;
/// let due = acquired.checked_add(Duration::from_millis(5))?;
/// assert_eq!(due.to_duration(), Duration::from_millis(2005));
/// # Ok::<(), PhysicalClockError>(())
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysicalInstant(
    /// Nanoseconds since the physical epoch of the associated execution.
    pub u64,
);

impl PhysicalInstant {
    /// Returns the elapsed duration from this instant's execution epoch.
    /// This does not read a clock or measure time since this value was created.
    pub const fn to_duration(self) -> core::time::Duration {
        core::time::Duration::from_nanos(self.0)
    }

    /// Constructs an instant from a duration since the associated execution epoch.
    /// Returns [`PhysicalClockError::Overflow`] if the offset exceeds `u64::MAX` nanoseconds.
    /// The caller supplies the epoch meaning; this method does not read a clock.
    pub fn from_duration(value: core::time::Duration) -> Result<Self, PhysicalClockError> {
        u64::try_from(value.as_nanos())
            .map(Self)
            .map_err(|_| PhysicalClockError::Overflow)
    }

    /// Returns the instant a nonnegative duration after this one in the same epoch.
    /// Returns [`PhysicalClockError::Overflow`] if the duration or sum exceeds
    /// the representable nanosecond range. The clock itself is not advanced.
    pub fn checked_add(self, delay: core::time::Duration) -> Result<Self, PhysicalClockError> {
        let delay = u64::try_from(delay.as_nanos()).map_err(|_| PhysicalClockError::Overflow)?;
        self.0
            .checked_add(delay)
            .map(Self)
            .ok_or(PhysicalClockError::Overflow)
    }
}

/// Minimal read capability shared by hosted and future platform-specific clock implementations.
pub trait PhysicalClock {
    /// Stable configured acquisition-time domain.
    fn domain(&self) -> PhysicalClockDomainId;
    /// Identity of this execution.
    fn epoch(&self) -> ExecutionEpoch;
    /// Reads an instant in this clock's domain and execution epoch.
    /// Successful reads are monotonic; repeated instants are allowed.
    fn now(&self) -> Result<PhysicalInstant, PhysicalClockError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An instant advances by a duration and retains its epoch-relative coordinate.
    #[test]
    fn physical_instant_adds_a_duration_without_wrapping() {
        let instant = PhysicalInstant(20);
        assert_eq!(
            instant.checked_add(core::time::Duration::from_nanos(5)),
            Ok(PhysicalInstant(25))
        );
        assert_eq!(
            PhysicalInstant(u64::MAX).checked_add(core::time::Duration::from_nanos(1)),
            Err(PhysicalClockError::Overflow)
        );
        assert_eq!(
            instant.checked_add(core::time::Duration::MAX),
            Err(PhysicalClockError::Overflow)
        );
        assert_eq!(instant.to_duration(), core::time::Duration::from_nanos(20));
    }
}
