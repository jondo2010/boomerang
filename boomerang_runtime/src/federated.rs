//! Legacy action-backed federation delivery; removed with the runtime bridge in phase 5.
use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use crate::{
    event::AsyncEvent, ActionCommon, AsyncActionRef, CommonContext, ReactorData, SendContext, Tag,
};

/// Failures specific to the legacy action-backed federation bridge.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LegacyFederatedError {
    /// The selected payload codec rejected a value.
    #[error("federated payload codec error: {0}")]
    Codec(#[from] crate::PayloadCodecError),

    /// The outbound transport rejected submission.
    #[error("federated outbound sink error: {0}")]
    Submission(#[from] crate::BoundarySubmissionError),

    /// Logical federation delivery cannot target a physical action.
    #[error("legacy inbound action adapters cannot target physical actions")]
    PhysicalAction,

    #[error("federated inbound endpoint scheduler channel is closed")]
    SchedulerClosed,
}

/// First-error latch for the legacy action-backed federation bridge.
#[derive(Debug, Clone, Default)]
pub struct FederatedFaultState {
    /// First terminal failure, shared with the legacy coordinator.
    first_error: Arc<OnceLock<LegacyFederatedError>>,
}

impl FederatedFaultState {
    /// Record `error` if no earlier legacy delivery failure has been published.
    pub fn record(&self, error: LegacyFederatedError) {
        let _ = self.first_error.set(error);
    }

    /// Return the first published legacy delivery failure without consuming it.
    pub fn get(&self) -> Option<LegacyFederatedError> {
        self.first_error.get().cloned()
    }
}

/// Erased decode-and-schedule operation for one legacy logical action.
type FederatedInboundHandler = dyn Fn(Tag, &[u8]) -> Result<(), LegacyFederatedError> + Send + Sync;

/// Legacy action-backed delivery adapter; compiled execution uses `InboundBoundaryAdapter`.
#[derive(Clone)]
pub struct LegacyInboundActionAdapter {
    /// Typed decode-and-schedule operation erased after lowering.
    handler: Arc<FederatedInboundHandler>,
}

impl fmt::Debug for LegacyInboundActionAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LegacyInboundActionAdapter").finish()
    }
}

impl LegacyInboundActionAdapter {
    /// Erase one typed logical-action decoder and scheduler target for storage in a route.
    pub fn new<T>(
        context: SendContext,
        action_ref: AsyncActionRef<T>,
        decoder: Box<dyn crate::PayloadDecoder<T, Error = crate::PayloadCodecError>>,
    ) -> Result<Self, LegacyFederatedError>
    where
        T: ReactorData,
    {
        if !action_ref.is_logical() {
            return Err(LegacyFederatedError::PhysicalAction);
        }

        Ok(Self {
            handler: Arc::new(move |tag, payload| {
                let value = decoder.decode(payload)?;
                let scheduled = context.schedule_external(AsyncEvent::Logical {
                    tag,
                    target: crate::AsyncEventTarget::Action(action_ref.key()),
                    value: Box::new(value),
                });
                if scheduled {
                    Ok(())
                } else {
                    Err(LegacyFederatedError::SchedulerClosed)
                }
            }),
        })
    }

    /// Decode and schedule one payload at its logical tag.
    pub fn schedule(&self, tag: Tag, payload: &[u8]) -> Result<(), LegacyFederatedError> {
        (self.handler)(tag, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn federated_fault_state_preserves_first_error() {
        let faults = FederatedFaultState::default();
        faults.record(crate::PayloadCodecError::new("first").into());
        faults.record(crate::BoundarySubmissionError::new("second").into());

        assert!(matches!(
            faults.get(),
            Some(LegacyFederatedError::Codec(message)) if message == crate::PayloadCodecError::new("first")
        ));
    }
}
