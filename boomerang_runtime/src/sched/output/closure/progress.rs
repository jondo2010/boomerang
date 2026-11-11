//! Durable ingress rules and final computational frontiers, separate from execution.
use crate::{clock::PhysicalClockError, OutputBound, Tag};

/// An ordered cutoff issued by the existing admission authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IngressSeal {
    /// Monotonic authority generation; older markers cannot replace a newer floor.
    pub(crate) generation: u64,
    /// Fresh admission is excluded at and below this bound.
    pub(crate) cut: OutputBound,
}

/// One enclave's durable admission floor and certified computational progress.
#[derive(Debug, Default)]
pub(crate) struct EnclaveOutputProgress {
    /// Installed admission cutoff, including cuts whose caller stopped waiting.
    pub(crate) closing: Option<IngressSeal>,
    /// Final computational certificate; no executed-tag or source-retention meaning.
    pub(crate) sealed: Option<OutputBound>,
}
impl EnclaveOutputProgress {
    /// Applies an ordered marker without rolling back a newer or stronger cutoff.
    pub(crate) fn install(&mut self, seal: IngressSeal) {
        if self
            .closing
            .is_none_or(|old| seal.generation > old.generation && seal.cut >= old.cut)
        {
            self.closing = Some(seal);
        }
    }

    /// Preserves unrestricted physical ingress above the durable cutoff.
    pub(crate) fn retag(&self, tag: Tag) -> Result<Tag, PhysicalClockError> {
        match self.closing {
            Some(seal) if OutputBound::ThroughTag(tag) <= seal.cut => seal
                .cut
                .first_tag_after()
                .map_err(|_| PhysicalClockError::Overflow),
            _ => Ok(tag),
        }
    }

    /// Rejects fresh strict input without affecting previously admitted values.
    pub(crate) fn rejects(&self, tag: Tag, internal: bool) -> bool {
        let cut = if internal {
            self.sealed
        } else {
            self.closing.map(|seal| seal.cut)
        };
        cut.is_some_and(|cut| OutputBound::ThroughTag(tag) <= cut)
    }
}

/// Opaque final progress on an existing local dependency edge.
#[derive(Clone, Copy, Debug)]
pub struct ProgressRelease {
    /// Actual sender enclave in the existing local barrier key domain.
    pub(crate) enclave: crate::EnclaveKey,
    /// Certified all-microstep frontier, separate from actual executed tags.
    pub(crate) time: crate::clock::PhysicalInstant,
}
