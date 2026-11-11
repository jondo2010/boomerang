//! Direct output results returned by reaction invocations.
//!
//! A driver can perform an operation directly and return its result. It needs
//! no runtime command queue. Each binding must define its commitment point.
//! Queue acceptance and physical application can be different points.
//!
//! Declare external-effect reactions with `#[reactor(external_effects = [Apply])]`.
//! Each reactor instance has a distinct boundary. A declaration can name multiple
//! reactions. Parent instances do not inherit their children's declarations.
//! Declared effects require paced execution. Fast-forward is rejected before
//! invocation. These results do not yet provide completion observers, tag
//! completion, time closure, or controlled simulator advancement.

/// Failure reported by an external device operation, possibly after a partial effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFault {
    /// The bound device could not complete its operation.
    Device,
}

/// The external output result of one reaction invocation.
///
/// Unit-returning closures remain compatible: `()` converts to `NotReported`.
/// Declared external effects must return `Unchanged`, `Committed`, or `Failed`.
/// The runtime fails execution for a missing report, an undeclared external
/// result, or `Failed`. Failure does not undo a possible partial operation.
/// Other reactions in the same parallel level can already have performed effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputOutcome {
    /// The reaction returned without reporting an external output result.
    NotReported,
    /// The invocation required no external operation.
    Unchanged,
    /// The operation crossed the binding's declared acceptance boundary.
    /// This proves physical application only if the binding defines it that way.
    Committed,
    /// The operation failed and may have caused a partial external effect.
    Failed(OutputFault),
}

impl From<()> for OutputOutcome {
    fn from(_: ()) -> Self {
        Self::NotReported
    }
}

impl OutputOutcome {
    /// Checks one returned invocation against its external-effect declaration.
    pub(crate) fn is_valid(self, external_effect: bool) -> bool {
        matches!(
            (external_effect, self),
            (false, Self::NotReported) | (true, Self::Unchanged | Self::Committed)
        )
    }
}
