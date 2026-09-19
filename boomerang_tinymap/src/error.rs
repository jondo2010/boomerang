use thiserror::Error;

/// Errors returned when a TinyMap operation cannot satisfy its input contract.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TinyMapError {
    #[error("requested {requested} values exceeds limit {limit}")]
    Capacity { limit: usize, requested: usize },
    #[error("expected exactly {expected} values, got {actual}")]
    ExactLength { expected: usize, actual: usize },
    #[error("span starting at {start} with length {len} exceeds domain length {domain_len}")]
    InvalidSpan {
        start: usize,
        len: usize,
        domain_len: usize,
    },
    #[error("spans at {first} and {second} overlap")]
    OverlappingSpans { first: usize, second: usize },
    #[error("invalid layout: {invariant}")]
    InvalidLayout { invariant: &'static str },
}

impl TinyMapError {
    /// Returns the capacity limit when this error reports a capacity overrun.
    pub const fn limit(self) -> Option<usize> {
        match self {
            Self::Capacity { limit, .. } => Some(limit),
            _ => None,
        }
    }

    /// Returns the requested capacity when this error reports a capacity overrun.
    pub const fn requested(self) -> Option<usize> {
        match self {
            Self::Capacity { requested, .. } => Some(requested),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TinyMapError;
    use std::format;

    #[test]
    fn capacity_error_preserves_accessors_and_display() {
        let error = TinyMapError::Capacity {
            limit: 2,
            requested: 3,
        };
        assert_eq!(error.limit(), Some(2));
        assert_eq!(error.requested(), Some(3));
        assert_eq!(format!("{error}"), "requested 3 values exceeds limit 2");
    }

    #[test]
    fn non_capacity_errors_preserve_display_and_empty_capacity_accessors() {
        let errors = [
            (
                TinyMapError::ExactLength {
                    expected: 2,
                    actual: 3,
                },
                "expected exactly 2 values, got 3",
            ),
            (
                TinyMapError::InvalidSpan {
                    start: 2,
                    len: 3,
                    domain_len: 4,
                },
                "span starting at 2 with length 3 exceeds domain length 4",
            ),
            (
                TinyMapError::OverlappingSpans {
                    first: 2,
                    second: 3,
                },
                "spans at 2 and 3 overlap",
            ),
            (
                TinyMapError::InvalidLayout {
                    invariant: "entries are ordered",
                },
                "invalid layout: entries are ordered",
            ),
        ];

        for (error, expected_display) in errors {
            assert_eq!(error.limit(), None);
            assert_eq!(error.requested(), None);
            assert_eq!(format!("{error}"), expected_display);
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn errors_implement_std_error_when_std_is_enabled() {
        let error = TinyMapError::InvalidLayout { invariant: "valid" };
        let _: &dyn std::error::Error = &error;
    }
}
