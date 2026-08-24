//! Scheduler-facing adapters owned by the federated protocol crate.

use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use boomerang_runtime::{
    ActionCommon, AsyncActionRef, CommonContext, InterPartitionEventSink, InterPartitionEventTime,
    ReactorData, SendContext, Tag,
};

use crate::{PayloadDecoder, PayloadEncoder};

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

impl FederatedEndpointError {
    pub fn codec(message: impl Into<String>) -> Self {
        Self::Codec(message.into())
    }

    pub fn send(message: impl Into<String>) -> Self {
        Self::Send(message.into())
    }
}

#[derive(Debug, Clone, Default)]
pub struct FederatedFaultState {
    first_error: Arc<OnceLock<FederatedEndpointError>>,
}

impl FederatedFaultState {
    pub fn record(&self, error: FederatedEndpointError) {
        let _ = self.first_error.set(error);
    }

    pub fn get(&self) -> Option<FederatedEndpointError> {
        self.first_error.get().cloned()
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

#[derive(Clone)]
pub struct FederatedInboundEndpoint {
    handler: Arc<FederatedInboundHandler>,
}

impl fmt::Debug for FederatedInboundEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FederatedInboundEndpoint").finish()
    }
}

impl FederatedInboundEndpoint {
    pub fn new<T>(
        context: SendContext,
        action_ref: AsyncActionRef<T>,
        decoder: Box<dyn PayloadDecoder<T>>,
    ) -> Result<Self, FederatedEndpointError>
    where
        T: ReactorData,
    {
        if !action_ref.is_logical() {
            return Err(FederatedEndpointError::PhysicalAction);
        }

        Ok(Self {
            handler: Arc::new(move |tag, payload| {
                let value = decoder
                    .decode(payload)
                    .map_err(|error| FederatedEndpointError::codec(error.to_string()))?;
                let scheduled = context.schedule_external(boomerang_runtime::AsyncEvent::Logical {
                    tag,
                    key: action_ref.key(),
                    value: Box::new(value),
                });
                scheduled
                    .then_some(())
                    .ok_or(FederatedEndpointError::SchedulerClosed)
            }),
        })
    }

    pub fn schedule(&self, tag: Tag, payload: &[u8]) -> Result<(), FederatedEndpointError> {
        (self.handler)(tag, payload)
    }
}

/// Serialized cross-partition event sink backed by a payload codec and Federate mailbox.
pub struct SerializedInterPartitionEventSink<T: ReactorData> {
    encoder: Box<dyn PayloadEncoder<T>>,
    outbound: Box<dyn FederatedOutboundSink>,
    faults: FederatedFaultState,
}

impl<T: ReactorData> SerializedInterPartitionEventSink<T> {
    pub fn new(
        encoder: Box<dyn PayloadEncoder<T>>,
        outbound: Box<dyn FederatedOutboundSink>,
        faults: FederatedFaultState,
    ) -> Self {
        Self {
            encoder,
            outbound,
            faults,
        }
    }
}

impl<T: ReactorData> InterPartitionEventSink<T> for SerializedInterPartitionEventSink<T> {
    fn send(&self, time: InterPartitionEventTime, _target: &AsyncActionRef<T>, value: &T) {
        let InterPartitionEventTime::Logical(tag) = time else {
            tracing::error!("Serialized sender cannot target a physical action");
            return;
        };

        let payload = match self.encoder.encode(value) {
            Ok(payload) => payload,
            Err(error) => {
                let error = FederatedEndpointError::codec(error.to_string());
                tracing::error!(?error, "Failed to encode federated payload");
                self.faults.record(error);
                return;
            }
        };

        let command = FederatedOutboundCommand::Msg(FederatedOutboundMessage { tag, payload });
        if let Err(error) = self.outbound.send(command) {
            tracing::error!(?error, "Failed to emit federated command");
            self.faults.record(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fault_state_preserves_first_error() {
        let faults = FederatedFaultState::default();
        faults.record(FederatedEndpointError::codec("first"));
        faults.record(FederatedEndpointError::send("second"));
        assert!(matches!(
            faults.get(),
            Some(FederatedEndpointError::Codec(message)) if message == "first"
        ));
    }
}
