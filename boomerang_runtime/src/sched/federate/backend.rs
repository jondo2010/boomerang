//! Backend-neutral compiled-Federate coordination contract.
//!
//! The compiled scheduler remains the authority for logical tag and revision
//! semantics. This module preserves those values as publication, acquisition,
//! and completion records while a selected backend carries them between
//! coordination participants. Backends consume this contract mechanically and
//! must not depend on scheduler event receivers, RTI types, Federate
//! identities, or transport-specific state.

use crate::Tag;

/// Monotonically advancing version of a Federate coordination exchange.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoordinationRevision(
    /// Transport-neutral integer value of this revision.
    u64,
);

impl CoordinationRevision {
    /// Creates a coordination revision from its transport-neutral value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the transport-neutral value of this revision.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Returns the next revision, wrapping deterministically at the integer boundary.
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// A Federate's current revision and optional next logical event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FederatePublication {
    /// Revision that identifies this publication exchange.
    revision: CoordinationRevision,
    /// Next finite logical event, if the Federate has one to publish.
    next_event: Option<Tag>,
}

impl FederatePublication {
    /// Creates a publication for a coordination revision and optional next event.
    pub const fn new(revision: CoordinationRevision, next_event: Option<Tag>) -> Self {
        Self {
            revision,
            next_event,
        }
    }

    /// Returns the revision supplied by this publication.
    pub const fn revision(self) -> CoordinationRevision {
        self.revision
    }

    /// Returns the next event supplied by this publication.
    pub const fn next_event(self) -> Option<Tag> {
        self.next_event
    }
}

/// A granted logical tag for a specific Federate coordination revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FederateAcquisition {
    /// Revision whose publication this grant answers.
    revision: CoordinationRevision,
    /// Logical tag that the backend granted for processing.
    granted: Tag,
}

impl FederateAcquisition {
    /// Creates an acquisition for a coordination revision and granted tag.
    pub const fn new(revision: CoordinationRevision, granted: Tag) -> Self {
        Self { revision, granted }
    }

    /// Returns the revision supplied by this acquisition.
    pub const fn revision(self) -> CoordinationRevision {
        self.revision
    }

    /// Returns the granted tag supplied by this acquisition.
    pub const fn granted(self) -> Tag {
        self.granted
    }
}

/// A completed logical tag reported by a Federate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FederateCompletion {
    /// Logical tag that the Federate finished processing.
    completed: Tag,
}

impl FederateCompletion {
    /// Creates a completion for a processed logical tag.
    pub const fn new(completed: Tag) -> Self {
        Self { completed }
    }

    /// Returns the completed logical tag.
    pub const fn completed(self) -> Tag {
        self.completed
    }
}

/// Protocol-free error returned by a Federate coordination backend.
#[derive(Debug, thiserror::Error)]
#[error("federate coordination failed: {source}")]
pub struct FederateCoordinationError {
    /// Concrete backend failure preserved without transport-specific conversion.
    #[source]
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
}

impl FederateCoordinationError {
    /// Preserves a concrete backend error as this transport-neutral error's source.
    pub fn from_error(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(error),
        }
    }
}

/// Transport-neutral coordination boundary for one compiled Federate.
pub trait FederateCoordinationBackend: Send {
    /// Publishes the Federate's current revision and optional next event.
    fn publish(
        &mut self,
        publication: FederatePublication,
    ) -> Result<(), FederateCoordinationError>;

    /// Polls for a grant without exposing a backend-specific transport.
    fn poll_acquisition(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<FederateAcquisition>, FederateCoordinationError>;

    /// Reports completion of a logical tag.
    fn complete(&mut self, completion: FederateCompletion)
        -> Result<(), FederateCoordinationError>;

    /// Stops coordination and releases any backend-owned pending state.
    fn stop(&mut self) -> Result<(), FederateCoordinationError>;
}

/// In-process backend used by local compiled Federate execution.
#[derive(Default)]
pub(crate) struct LocalFederateCoordinationBackend {
    /// Latest finite publication awaiting one local acquisition.
    pending_publication: Option<FederatePublication>,
}

impl FederateCoordinationBackend for LocalFederateCoordinationBackend {
    /// Retains the latest publication with a finite next event for local delivery.
    fn publish(
        &mut self,
        publication: FederatePublication,
    ) -> Result<(), FederateCoordinationError> {
        if publication
            .next_event
            .is_some_and(|tag| tag > Tag::NEVER && tag < Tag::FOREVER)
        {
            self.pending_publication = Some(publication);
        }
        Ok(())
    }

    /// Consumes the retained publication and derives its one matching acquisition.
    fn poll_acquisition(
        &mut self,
        _timeout: std::time::Duration,
    ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
        Ok(self.pending_publication.take().and_then(|publication| {
            publication
                .next_event
                .map(|granted| FederateAcquisition::new(publication.revision, granted))
        }))
    }

    /// Accepts completion because local coordination needs no additional acknowledgment.
    fn complete(
        &mut self,
        _completion: FederateCompletion,
    ) -> Result<(), FederateCoordinationError> {
        Ok(())
    }

    /// Discards pending local work when coordination stops.
    fn stop(&mut self) -> Result<(), FederateCoordinationError> {
        self.pending_publication = None;
        Ok(())
    }
}

#[cfg(test)]
/// Contract tests using an in-process recording backend without a transport.
mod tests {
    use super::*;

    #[derive(Default)]
    /// Test backend that records the contract calls it receives.
    struct RecordingBackend {
        /// Publications supplied to the test backend.
        publications: Vec<FederatePublication>,
        /// Completions supplied to the test backend.
        completions: Vec<FederateCompletion>,
        /// One acquisition returned by the next poll.
        acquisition: Option<FederateAcquisition>,
        /// Whether the test backend received a stop call.
        stopped: bool,
    }

    impl FederateCoordinationBackend for RecordingBackend {
        /// Records the publication so the contract test can inspect it.
        fn publish(
            &mut self,
            publication: FederatePublication,
        ) -> Result<(), FederateCoordinationError> {
            self.publications.push(publication);
            Ok(())
        }

        /// Returns the configured acquisition once without waiting.
        fn poll_acquisition(
            &mut self,
            _timeout: std::time::Duration,
        ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
            Ok(self.acquisition.take())
        }

        /// Records the completion so the contract test can inspect it.
        fn complete(
            &mut self,
            completion: FederateCompletion,
        ) -> Result<(), FederateCoordinationError> {
            self.completions.push(completion);
            Ok(())
        }

        /// Records that the contract requested shutdown.
        fn stop(&mut self) -> Result<(), FederateCoordinationError> {
            self.stopped = true;
            Ok(())
        }
    }

    #[test]
    /// Verifies the values and operations required by the transport-neutral contract.
    fn backend_contract_is_transport_neutral() {
        let revision = CoordinationRevision::new(u64::MAX);
        let next_revision = revision.next();
        let tag = crate::Tag::new(crate::Duration::milliseconds(1), 2);
        let publication = FederatePublication::new(revision, Some(tag));
        let acquisition = FederateAcquisition::new(next_revision, tag);
        let completion = FederateCompletion::new(tag);
        let mut backend = RecordingBackend {
            acquisition: Some(acquisition),
            ..Default::default()
        };

        backend.publish(publication).unwrap();
        let acquired = backend
            .poll_acquisition(std::time::Duration::ZERO)
            .unwrap()
            .unwrap();
        backend.complete(completion).unwrap();
        backend.stop().unwrap();

        assert_eq!(revision.value(), u64::MAX);
        assert_eq!(next_revision.value(), 0);
        assert_eq!(backend.publications[0].revision(), revision);
        assert_eq!(backend.publications[0].next_event(), Some(tag));
        assert_eq!(acquired.revision(), next_revision);
        assert_eq!(acquired.granted(), tag);
        assert_eq!(backend.completions[0].completed(), tag);
        assert!(backend.stopped);

        let latest_tag = crate::Tag::new(crate::Duration::milliseconds(2), 0);
        let latest = FederatePublication::new(next_revision, Some(latest_tag));
        let mut local = LocalFederateCoordinationBackend::default();
        local.publish(publication).unwrap();
        local.publish(latest).unwrap();
        local
            .publish(FederatePublication::new(next_revision.next(), None))
            .unwrap();
        let locally_acquired = local
            .poll_acquisition(std::time::Duration::ZERO)
            .unwrap()
            .unwrap();
        local.publish(latest).unwrap();
        local.stop().unwrap();

        assert_eq!(locally_acquired.revision(), next_revision);
        assert_eq!(locally_acquired.granted(), latest_tag);
        assert_eq!(
            local.poll_acquisition(std::time::Duration::ZERO).unwrap(),
            None
        );

        let mut terminal = LocalFederateCoordinationBackend::default();
        terminal
            .publish(FederatePublication::new(
                next_revision,
                Some(crate::Tag::FOREVER),
            ))
            .unwrap();
        assert_eq!(
            terminal
                .poll_acquisition(std::time::Duration::ZERO)
                .unwrap(),
            None
        );
    }
}
