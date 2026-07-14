use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use crate::{
    event::AsyncEvent, ActionCommon, AsyncActionRef, CommonContext, ReactorData, SendContext, Tag,
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum FederatedEndpointError {
    #[error("federated payload codec error: {0}")]
    Codec(String),

    #[error("federated outbound sink error: {0}")]
    Send(String),

    #[error("federated endpoints cannot target physical actions")]
    PhysicalAction,

    #[error("federated inbound endpoint scheduler channel is closed")]
    SchedulerClosed,
}

/// Shared first-error latch for terminal federated runtime endpoint failures.
#[derive(Debug, Clone, Default)]
pub struct FederatedFaultState {
    first_error: Arc<OnceLock<FederatedEndpointError>>,
}

impl FederatedFaultState {
    /// Record `error` if no earlier endpoint failure has been published.
    pub fn record(&self, error: FederatedEndpointError) {
        let _ = self.first_error.set(error);
    }

    /// Return the first published endpoint failure without consuming it.
    pub fn get(&self) -> Option<FederatedEndpointError> {
        self.first_error.get().cloned()
    }
}

impl FederatedEndpointError {
    pub fn codec(message: impl Into<String>) -> Self {
        Self::Codec(message.into())
    }

    pub fn send(message: impl Into<String>) -> Self {
        Self::Send(message.into())
    }
}

/// Encodes typed payload values for a federated endpoint.
pub trait FederatedPayloadEncoder<T: ReactorData>: Send + Sync + 'static {
    fn encode(&self, value: &T) -> Result<Vec<u8>, FederatedEndpointError>;
}

impl<T, F> FederatedPayloadEncoder<T> for F
where
    T: ReactorData,
    F: Fn(&T) -> Result<Vec<u8>, FederatedEndpointError> + Send + Sync + 'static,
{
    fn encode(&self, value: &T) -> Result<Vec<u8>, FederatedEndpointError> {
        (self)(value)
    }
}

/// Decodes typed payload values for a federated endpoint.
pub trait FederatedPayloadDecoder<T: ReactorData>: Send + Sync + 'static {
    fn decode(&self, bytes: &[u8]) -> Result<T, FederatedEndpointError>;
}

impl<T, F> FederatedPayloadDecoder<T> for F
where
    T: ReactorData,
    F: Fn(&[u8]) -> Result<T, FederatedEndpointError> + Send + Sync + 'static,
{
    fn decode(&self, bytes: &[u8]) -> Result<T, FederatedEndpointError> {
        (self)(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedOutboundMessage {
    pub tag: Tag,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FederatedOutboundCommand {
    Msg(FederatedOutboundMessage),
}

pub trait FederatedOutboundSink: Send + Sync + 'static {
    fn send(&self, command: FederatedOutboundCommand) -> Result<(), FederatedEndpointError>;
}

type FederatedInboundHandler =
    dyn Fn(Tag, &[u8]) -> Result<(), FederatedEndpointError> + Send + Sync;

/// Type-erased runtime handler attached directly to one lowered federated route.
#[derive(Clone)]
pub struct FederatedInboundEndpoint {
    /// Typed decode-and-schedule operation erased after lowering.
    handler: Arc<FederatedInboundHandler>,
}

impl fmt::Debug for FederatedInboundEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FederatedInboundEndpoint").finish()
    }
}

impl FederatedInboundEndpoint {
    /// Erase one typed logical-action decoder and scheduler target for storage in a route.
    pub fn new<T>(
        context: SendContext,
        action_ref: AsyncActionRef<T>,
        decoder: Box<dyn FederatedPayloadDecoder<T>>,
    ) -> Result<Self, FederatedEndpointError>
    where
        T: ReactorData,
    {
        if !action_ref.is_logical() {
            return Err(FederatedEndpointError::PhysicalAction);
        }

        Ok(Self {
            handler: Arc::new(move |tag, payload| {
                let value = decoder.decode(payload)?;
                let scheduled = context.schedule_external(AsyncEvent::Logical {
                    tag,
                    key: action_ref.key(),
                    value: Box::new(value),
                });
                if scheduled {
                    Ok(())
                } else {
                    Err(FederatedEndpointError::SchedulerClosed)
                }
            }),
        })
    }

    /// Decode and schedule one payload at its logical tag.
    pub fn schedule(&self, tag: Tag, payload: &[u8]) -> Result<(), FederatedEndpointError> {
        (self.handler)(tag, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn federated_fault_state_preserves_first_error() {
        let faults = FederatedFaultState::default();
        faults.record(FederatedEndpointError::codec("first"));
        faults.record(FederatedEndpointError::send("second"));

        assert!(matches!(
            faults.get(),
            Some(FederatedEndpointError::Codec(message)) if message == "first"
        ));
    }
}
