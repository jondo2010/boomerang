use core::fmt;

/// Errors returned when a TinyMap operation cannot satisfy its input contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TinyMapError {
    Capacity {
        limit: usize,
        requested: usize,
    },
    ExactLength {
        expected: usize,
        actual: usize,
    },
    InvalidSpan {
        start: usize,
        len: usize,
        domain_len: usize,
    },
    OverlappingSpans {
        first: usize,
        second: usize,
    },
    InvalidLayout {
        invariant: &'static str,
    },
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

impl fmt::Display for TinyMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capacity { limit, requested } => {
                write!(
                    formatter,
                    "requested {requested} values exceeds limit {limit}"
                )
            }
            Self::ExactLength { expected, actual } => {
                write!(
                    formatter,
                    "expected exactly {expected} values, got {actual}"
                )
            }
            Self::InvalidSpan {
                start,
                len,
                domain_len,
            } => write!(
                formatter,
                "span starting at {start} with length {len} exceeds domain length {domain_len}"
            ),
            Self::OverlappingSpans { first, second } => {
                write!(formatter, "spans at {first} and {second} overlap")
            }
            Self::InvalidLayout { invariant } => {
                write!(formatter, "invalid layout: {invariant}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TinyMapError {}

#[cfg(test)]
mod tests {
    use super::TinyMapError;
    use std::format;

    #[test]
    fn capacity_error_reports_limit_and_request_without_std() {
        let error = TinyMapError::Capacity {
            limit: 2,
            requested: 3,
        };
        assert_eq!(error.limit(), Some(2));
        assert_eq!(error.requested(), Some(3));
        assert_eq!(format!("{error}"), "requested 3 values exceeds limit 2");
    }
}
