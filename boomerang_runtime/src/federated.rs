use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use crate::{
    event::AsyncEvent, ActionCommon, AsyncActionRef, CommonContext, ReactorData, SendContext, Tag,
};

tinymap::key_type! {
    /// Dense process-local key for a lowered federated inbound endpoint.
    ///
    /// This key must never be serialized or compared between federates.
    pub FederatedEndpointKey
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum FederatedEndpointError {
    #[error("federated payload codec error: {0}")]
    Codec(String),

    #[error("federated outbound sink error: {0}")]
    Send(String),

    #[error("unknown federated endpoint: {0}")]
    UnknownEndpoint(FederatedEndpointKey),

    #[error("federated endpoints cannot target physical actions")]
    PhysicalAction,

    #[error("federated inbound endpoint {0} scheduler channel is closed")]
    SchedulerClosed(FederatedEndpointKey),
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

trait FederatedInboundEndpoint: Send + Sync {
    fn schedule(&self, tag: Tag, payload: &[u8]) -> Result<(), FederatedEndpointError>;
}

struct TypedFederatedInboundEndpoint<T: ReactorData> {
    endpoint: FederatedEndpointKey,
    context: SendContext,
    action_ref: AsyncActionRef<T>,
    decoder: Box<dyn FederatedPayloadDecoder<T>>,
}

impl<T: ReactorData> TypedFederatedInboundEndpoint<T> {
    fn new(
        endpoint: FederatedEndpointKey,
        context: SendContext,
        action_ref: AsyncActionRef<T>,
        decoder: Box<dyn FederatedPayloadDecoder<T>>,
    ) -> Self {
        Self {
            endpoint,
            context,
            action_ref,
            decoder,
        }
    }
}

impl<T: ReactorData> FederatedInboundEndpoint for TypedFederatedInboundEndpoint<T> {
    fn schedule(&self, tag: Tag, payload: &[u8]) -> Result<(), FederatedEndpointError> {
        let value = self.decoder.decode(payload)?;
        let scheduled = self.context.schedule_external(AsyncEvent::Logical {
            tag,
            key: self.action_ref.key(),
            value: Box::new(value),
        });

        if scheduled {
            Ok(())
        } else {
            Err(FederatedEndpointError::SchedulerClosed(self.endpoint))
        }
    }
}

#[derive(Default)]
pub struct FederatedInboundEndpointRegistry {
    endpoints: tinymap::TinyMap<FederatedEndpointKey, Arc<dyn FederatedInboundEndpoint>>,
}

impl Clone for FederatedInboundEndpointRegistry {
    fn clone(&self) -> Self {
        Self {
            endpoints: self.endpoints.values().cloned().collect(),
        }
    }
}

impl fmt::Debug for FederatedInboundEndpointRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FederatedInboundEndpointRegistry")
            .field("endpoints", &self.endpoints.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl FederatedInboundEndpointRegistry {
    pub fn register<T>(
        &mut self,
        context: SendContext,
        action_ref: AsyncActionRef<T>,
        decoder: Box<dyn FederatedPayloadDecoder<T>>,
    ) -> Result<FederatedEndpointKey, FederatedEndpointError>
    where
        T: ReactorData,
    {
        if !action_ref.is_logical() {
            return Err(FederatedEndpointError::PhysicalAction);
        }

        Ok(self.endpoints.insert_with_key(|endpoint| {
            Arc::new(TypedFederatedInboundEndpoint::new(
                endpoint, context, action_ref, decoder,
            ))
        }))
    }

    pub fn schedule(
        &self,
        endpoint: FederatedEndpointKey,
        tag: Tag,
        payload: &[u8],
    ) -> Result<(), FederatedEndpointError> {
        let endpoint_handler = self
            .endpoints
            .get(endpoint)
            .ok_or(FederatedEndpointError::UnknownEndpoint(endpoint))?;
        endpoint_handler.schedule(tag, payload)
    }

    pub fn contains(&self, endpoint: FederatedEndpointKey) -> bool {
        self.endpoints.get(endpoint).is_some()
    }

    pub fn len(&self) -> usize {
        self.endpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
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
