//! Checked conversions between Boomerang runtime metadata and wire protocol metadata.

use crate::{WireDelay, WireTag};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeBridgeError {
    #[error(
        "finite runtime tag {tag} has negative offset {offset_ns}ns; use Tag::NEVER for negative infinity"
    )]
    NegativeRuntimeTag {
        tag: boomerang_runtime::Tag,
        offset_ns: i128,
    },

    #[error("runtime tag {tag} microstep {microstep} does not fit wire u64")]
    RuntimeMicrostepOutOfRange {
        tag: boomerang_runtime::Tag,
        microstep: usize,
    },

    #[error(
        "finite wire tag {tag} has negative offset {offset_ns}ns; use WireTag::NEVER for negative infinity"
    )]
    NegativeWireTag { tag: WireTag, offset_ns: i128 },

    #[error("finite wire tag {tag} offset {offset_ns}ns does not fit runtime Duration")]
    WireTagOffsetOutOfRange { tag: WireTag, offset_ns: i128 },

    #[error("finite wire tag {tag} microstep {microstep} does not fit runtime usize")]
    WireMicrostepOutOfRange { tag: WireTag, microstep: u64 },

    #[error("finite wire tag {tag} collides with runtime Tag::FOREVER")]
    WireTagCollidesWithRuntimeForever { tag: WireTag },

    #[error("cross-federate delay {delay} is negative; wire delays must be nonnegative")]
    NegativeRuntimeDelay { delay: boomerang_runtime::Duration },

    #[error("cross-federate delay {delay} does not fit wire u64 nanoseconds")]
    RuntimeDelayOutOfRange { delay: boomerang_runtime::Duration },
}

impl TryFrom<boomerang_runtime::Tag> for WireTag {
    type Error = RuntimeBridgeError;

    fn try_from(tag: boomerang_runtime::Tag) -> Result<Self, Self::Error> {
        if tag == boomerang_runtime::Tag::NEVER {
            return Ok(Self::NEVER);
        }
        if tag == boomerang_runtime::Tag::FOREVER {
            return Ok(Self::FOREVER);
        }

        let offset_ns = tag.offset().whole_nanoseconds();
        if offset_ns < 0 {
            return Err(RuntimeBridgeError::NegativeRuntimeTag { tag, offset_ns });
        }

        let microstep = tag.microstep().try_into().map_err(|_| {
            RuntimeBridgeError::RuntimeMicrostepOutOfRange {
                tag,
                microstep: tag.microstep(),
            }
        })?;

        Ok(Self::finite(offset_ns, microstep))
    }
}

impl TryFrom<WireTag> for boomerang_runtime::Tag {
    type Error = RuntimeBridgeError;

    fn try_from(tag: WireTag) -> Result<Self, Self::Error> {
        match tag {
            WireTag::Never => Ok(Self::NEVER),
            WireTag::Forever => Ok(Self::FOREVER),
            WireTag::Finite {
                offset_ns,
                microstep,
            } => {
                if offset_ns < 0 {
                    return Err(RuntimeBridgeError::NegativeWireTag { tag, offset_ns });
                }

                let max_runtime_offset_ns = boomerang_runtime::Duration::MAX.whole_nanoseconds();
                if offset_ns > max_runtime_offset_ns {
                    return Err(RuntimeBridgeError::WireTagOffsetOutOfRange { tag, offset_ns });
                }

                let microstep = microstep
                    .try_into()
                    .map_err(|_| RuntimeBridgeError::WireMicrostepOutOfRange { tag, microstep })?;
                let runtime_tag = Self::new(
                    boomerang_runtime::Duration::nanoseconds_i128(offset_ns),
                    microstep,
                );
                if runtime_tag == Self::FOREVER {
                    return Err(RuntimeBridgeError::WireTagCollidesWithRuntimeForever { tag });
                }

                Ok(runtime_tag)
            }
        }
    }
}

impl TryFrom<boomerang_runtime::Duration> for WireDelay {
    type Error = RuntimeBridgeError;

    fn try_from(delay: boomerang_runtime::Duration) -> Result<Self, Self::Error> {
        let nanos = delay.whole_nanoseconds();
        if nanos < 0 {
            return Err(RuntimeBridgeError::NegativeRuntimeDelay { delay });
        }

        let nanos = u64::try_from(nanos)
            .map_err(|_| RuntimeBridgeError::RuntimeDelayOutOfRange { delay })?;

        Ok(Self::from_nanos(nanos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_runtime_bridge_error<T>(result: Result<T, RuntimeBridgeError>, expected: &str) {
        assert!(matches!(result, Err(error) if error.to_string().contains(expected)));
    }

    #[test]
    fn tag_bridge_round_trips_runtime_sentinels_and_finite_tags() {
        for tag in [
            boomerang_runtime::Tag::NEVER,
            boomerang_runtime::Tag::ZERO,
            boomerang_runtime::Tag::new(boomerang_runtime::Duration::nanoseconds(42), 7),
            boomerang_runtime::Tag::FOREVER,
        ] {
            let wire_tag = WireTag::try_from(tag).unwrap();
            assert_eq!(boomerang_runtime::Tag::try_from(wire_tag).unwrap(), tag);
        }
    }

    #[test]
    fn tag_bridge_rejects_negative_finite_tags() {
        assert_eq!(
            WireTag::try_from(boomerang_runtime::Tag::NEVER).unwrap(),
            WireTag::NEVER
        );
        assert_runtime_bridge_error(
            WireTag::try_from(boomerang_runtime::Tag::new(
                boomerang_runtime::Duration::nanoseconds(-1),
                0,
            )),
            "negative offset",
        );
        assert_runtime_bridge_error(
            boomerang_runtime::Tag::try_from(WireTag::finite(-1, 0)),
            "negative offset",
        );
    }

    #[test]
    fn tag_bridge_rejects_wire_values_outside_runtime_representation() {
        let too_large = boomerang_runtime::Duration::MAX.whole_nanoseconds() + 1;
        assert_runtime_bridge_error(
            boomerang_runtime::Tag::try_from(WireTag::finite(too_large, 0)),
            "does not fit runtime Duration",
        );

        #[cfg(target_pointer_width = "64")]
        assert_runtime_bridge_error(
            boomerang_runtime::Tag::try_from(WireTag::finite(
                boomerang_runtime::Duration::MAX.whole_nanoseconds(),
                u64::MAX,
            )),
            "collides with runtime Tag::FOREVER",
        );
    }

    #[test]
    fn delay_bridge_rejects_invalid_wire_delays() {
        assert_eq!(
            WireDelay::try_from(boomerang_runtime::Duration::ZERO).unwrap(),
            WireDelay::ZERO
        );
        assert_eq!(
            WireDelay::try_from(boomerang_runtime::Duration::nanoseconds(5))
                .unwrap()
                .as_nanos(),
            5
        );
        assert_runtime_bridge_error(
            WireDelay::try_from(boomerang_runtime::Duration::nanoseconds(-1)),
            "negative",
        );
        assert_runtime_bridge_error(
            WireDelay::try_from(boomerang_runtime::Duration::nanoseconds_i128(
                i128::from(u64::MAX) + 1,
            )),
            "does not fit wire u64",
        );
    }
}
