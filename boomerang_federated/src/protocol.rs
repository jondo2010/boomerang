use std::fmt;

/// A protocol tag independent of process-local clocks and architecture-sized integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WireTag {
    Never,
    Finite { offset_ns: i128, microstep: u64 },
    Forever,
}

impl WireTag {
    pub const NEVER: Self = Self::Never;
    pub const ZERO: Self = Self::Finite {
        offset_ns: 0,
        microstep: 0,
    };
    pub const FOREVER: Self = Self::Forever;

    pub const fn finite(offset_ns: i128, microstep: u64) -> Self {
        Self::Finite {
            offset_ns,
            microstep,
        }
    }

    pub fn is_finite(self) -> bool {
        matches!(self, Self::Finite { .. })
    }

    pub fn offset_ns(self) -> Option<i128> {
        match self {
            Self::Finite { offset_ns, .. } => Some(offset_ns),
            Self::Never | Self::Forever => None,
        }
    }

    pub fn microstep(self) -> Option<u64> {
        match self {
            Self::Finite { microstep, .. } => Some(microstep),
            Self::Never | Self::Forever => None,
        }
    }

    /// Apply a logical connection delay using Boomerang's delayed-action tag rule.
    ///
    /// A zero delay preserves the source tag. A positive delay advances the offset and resets the
    /// microstep to zero. Sentinel tags remain sentinels.
    pub fn checked_delay(self, delay: WireDelay) -> Option<Self> {
        match self {
            Self::Never => Some(Self::Never),
            Self::Forever => Some(Self::Forever),
            Self::Finite {
                offset_ns,
                microstep,
            } if delay.is_zero() => Some(Self::Finite {
                offset_ns,
                microstep,
            }),
            Self::Finite { offset_ns, .. } => offset_ns
                .checked_add(i128::from(delay.as_nanos()))
                .map(|offset_ns| Self::Finite {
                    offset_ns,
                    microstep: 0,
                }),
        }
    }
}

impl fmt::Display for WireTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireTag::Never => f.write_str("[NEVER]"),
            WireTag::Forever => f.write_str("[FOREVER]"),
            WireTag::Finite {
                offset_ns,
                microstep,
            } => write!(f, "[{offset_ns}ns+{microstep}]"),
        }
    }
}

/// A nonnegative logical delay on a cross-federate edge, represented in nanoseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WireDelay {
    nanos: u64,
}

impl WireDelay {
    pub const ZERO: Self = Self { nanos: 0 };

    pub const fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    pub const fn as_nanos(self) -> u64 {
        self.nanos
    }

    pub const fn is_zero(self) -> bool {
        self.nanos == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_tags_order_sentinels_and_finite_tags() {
        assert!(WireTag::Never < WireTag::ZERO);
        assert!(WireTag::ZERO < WireTag::finite(0, 1));
        assert!(WireTag::finite(10, 0) < WireTag::Forever);
    }

    #[test]
    fn wire_tag_delay_preserves_zero_delay_and_resets_positive_delay_microstep() {
        let tag = WireTag::finite(5, 3);

        assert_eq!(tag.checked_delay(WireDelay::ZERO), Some(tag));
        assert_eq!(
            tag.checked_delay(WireDelay::from_nanos(10)),
            Some(WireTag::finite(15, 0))
        );
        assert_eq!(
            WireTag::Forever.checked_delay(WireDelay::from_nanos(10)),
            Some(WireTag::Forever)
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn wire_tags_round_trip_through_serde_json() {
        for tag in [
            WireTag::Never,
            WireTag::ZERO,
            WireTag::finite(42, 7),
            WireTag::Forever,
        ] {
            let encoded = serde_json::to_vec(&tag).unwrap();
            let decoded: WireTag = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(decoded, tag);
        }
    }
}
