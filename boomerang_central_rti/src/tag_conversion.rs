//! Checked conversions between runtime and portable wire tags.

use crate::WireTag;

#[derive(Debug, thiserror::Error)]
/// Failure converting between runtime and wire tags.
pub enum TagConversionError {
    /// A finite runtime tag used a negative time offset.
    #[error(
        "finite runtime tag {tag} has negative offset {offset_ns}ns; use Tag::NEVER for negative infinity"
    )]
    NegativeRuntimeTag {
        /// Runtime tag that cannot be sent as a finite wire tag.
        tag: boomerang_runtime::Tag,
        /// Negative nanosecond offset in the runtime tag.
        offset_ns: i128,
    },

    /// A runtime tag's microstep exceeds the wire representation.
    #[error("runtime tag {tag} microstep {microstep} does not fit wire u64")]
    RuntimeMicrostepOutOfRange {
        /// Runtime tag with the unrepresentable microstep.
        tag: boomerang_runtime::Tag,
        /// Microstep that cannot fit in `u64`.
        microstep: usize,
    },

    /// A finite wire tag used a negative time offset.
    #[error(
        "finite wire tag {tag} has negative offset {offset_ns}ns; use WireTag::NEVER for negative infinity"
    )]
    NegativeWireTag {
        /// Wire tag that cannot become a runtime tag.
        tag: WireTag,
        /// Negative nanosecond offset in the wire tag.
        offset_ns: i128,
    },

    /// A wire offset exceeds the runtime duration range.
    #[error("finite wire tag {tag} offset {offset_ns}ns does not fit runtime Duration")]
    WireTagOffsetOutOfRange {
        /// Wire tag with the unrepresentable offset.
        tag: WireTag,
        /// Nanosecond offset that cannot fit the runtime duration.
        offset_ns: i128,
    },

    /// A wire microstep exceeds the runtime index representation.
    #[error("finite wire tag {tag} microstep {microstep} does not fit runtime usize")]
    WireMicrostepOutOfRange {
        /// Wire tag with the unrepresentable microstep.
        tag: WireTag,
        /// Microstep that cannot fit in `usize`.
        microstep: u64,
    },

    /// A finite wire tag aliases the runtime's positive-infinity sentinel.
    #[error("finite wire tag {tag} collides with runtime Tag::FOREVER")]
    WireTagCollidesWithRuntimeForever {
        /// Finite wire tag that maps to `Tag::FOREVER`.
        tag: WireTag,
    },
}

/// Convert a runtime tag into its checked federated wire representation.
pub fn wire_tag_from_runtime(tag: boomerang_runtime::Tag) -> Result<WireTag, TagConversionError> {
    if tag == boomerang_runtime::Tag::NEVER {
        return Ok(WireTag::NEVER);
    }
    if tag == boomerang_runtime::Tag::FOREVER {
        return Ok(WireTag::FOREVER);
    }

    let offset_ns = tag.offset().whole_nanoseconds();
    if offset_ns < 0 {
        return Err(TagConversionError::NegativeRuntimeTag { tag, offset_ns });
    }

    let microstep =
        tag.microstep()
            .try_into()
            .map_err(|_| TagConversionError::RuntimeMicrostepOutOfRange {
                tag,
                microstep: tag.microstep(),
            })?;

    Ok(WireTag::finite(offset_ns, microstep))
}

/// Convert a federated wire tag into its checked runtime representation.
pub fn runtime_tag_from_wire(tag: WireTag) -> Result<boomerang_runtime::Tag, TagConversionError> {
    match tag {
        WireTag::Never => Ok(boomerang_runtime::Tag::NEVER),
        WireTag::Forever => Ok(boomerang_runtime::Tag::FOREVER),
        WireTag::Finite {
            offset_ns,
            microstep,
        } => {
            if offset_ns < 0 {
                return Err(TagConversionError::NegativeWireTag { tag, offset_ns });
            }

            let max_runtime_offset_ns = boomerang_runtime::Duration::MAX.whole_nanoseconds();
            if offset_ns > max_runtime_offset_ns {
                return Err(TagConversionError::WireTagOffsetOutOfRange { tag, offset_ns });
            }

            let microstep = microstep
                .try_into()
                .map_err(|_| TagConversionError::WireMicrostepOutOfRange { tag, microstep })?;
            let runtime_tag = boomerang_runtime::Tag::new(
                boomerang_runtime::Duration::nanoseconds_i128(offset_ns),
                microstep,
            );
            if runtime_tag == boomerang_runtime::Tag::FOREVER {
                return Err(TagConversionError::WireTagCollidesWithRuntimeForever { tag });
            }

            Ok(runtime_tag)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_tag_conversion_error<T>(result: Result<T, TagConversionError>, expected: &str) {
        assert!(matches!(result, Err(error) if error.to_string().contains(expected)));
    }

    #[test]
    fn tag_conversion_round_trips_runtime_sentinels_and_finite_tags() {
        for tag in [
            boomerang_runtime::Tag::NEVER,
            boomerang_runtime::Tag::ZERO,
            boomerang_runtime::Tag::new(boomerang_runtime::Duration::nanoseconds(42), 7),
            boomerang_runtime::Tag::FOREVER,
        ] {
            let wire_tag = wire_tag_from_runtime(tag).unwrap();
            assert_eq!(runtime_tag_from_wire(wire_tag).unwrap(), tag);
        }
    }

    #[test]
    fn tag_conversion_rejects_negative_finite_tags() {
        assert_eq!(
            wire_tag_from_runtime(boomerang_runtime::Tag::NEVER).unwrap(),
            WireTag::NEVER
        );
        assert_tag_conversion_error(
            wire_tag_from_runtime(boomerang_runtime::Tag::new(
                boomerang_runtime::Duration::nanoseconds(-1),
                0,
            )),
            "negative offset",
        );
        assert_tag_conversion_error(
            runtime_tag_from_wire(WireTag::finite(-1, 0)),
            "negative offset",
        );
    }

    #[test]
    fn tag_conversion_rejects_wire_values_outside_runtime_representation() {
        let too_large = boomerang_runtime::Duration::MAX.whole_nanoseconds() + 1;
        assert_tag_conversion_error(
            runtime_tag_from_wire(WireTag::finite(too_large, 0)),
            "does not fit runtime Duration",
        );

        #[cfg(target_pointer_width = "64")]
        assert_tag_conversion_error(
            runtime_tag_from_wire(WireTag::finite(
                boomerang_runtime::Duration::MAX.whole_nanoseconds(),
                u64::MAX,
            )),
            "collides with runtime Tag::FOREVER",
        );
    }
}
