//! Integer identities and epoch-relative time, usable without a hosted runtime.

/// Stable configured identity of a physical clock domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysicalClockDomainId(pub u64);

/// Random identity of one execution, distinct across independently started hosted runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionEpoch(pub u128);

/// Unsigned nanoseconds since the selected execution epoch began.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysicalTimeNanos(pub u64);

/// Checked physical-time and clock protocol failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("physical clock: {self:?}")]
pub enum PhysicalClockError {
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

impl PhysicalTimeNanos {
    /// Converts an epoch offset to a platform-independent duration.
    pub const fn to_duration(self) -> core::time::Duration {
        core::time::Duration::from_nanos(self.0)
    }

    /// Converts a duration, rejecting offsets beyond the integer clock range.
    pub fn from_duration(value: core::time::Duration) -> Result<Self, PhysicalClockError> {
        u64::try_from(value.as_nanos())
            .map(Self)
            .map_err(|_| PhysicalClockError::Overflow)
    }

    /// Adds a nonnegative delay without wrapping the epoch offset.
    pub fn checked_add(self, delay: Self) -> Result<Self, PhysicalClockError> {
        self.0
            .checked_add(delay.0)
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
    /// Checked epoch-relative physical time.
    fn now(&self) -> Result<PhysicalTimeNanos, PhysicalClockError>;
}
