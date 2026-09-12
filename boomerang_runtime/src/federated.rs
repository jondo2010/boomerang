use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use crate::{
    event::AsyncEvent, ActionCommon, AsyncActionRef, CommonContext, ReactorData, SendContext, Tag,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FederatedEndpointError {
    /// A compiled inbound event must carry a finite, nonnegative logical tag.
    #[error("invalid compiled inbound tag: {0:?}")]
    InvalidTag(Tag),
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
    /// Installs a decoded value directly into a validated compiled inbound route's port.
    pub(crate) fn for_port<T: ReactorData>(
        sender: crate::Sender<AsyncEvent>,
        port: crate::image::PortIndex,
        decoder: impl FederatedPayloadDecoder<T>,
    ) -> Self {
        Self {
            handler: Arc::new(move |tag, payload| {
                if tag < Tag::ZERO || tag >= Tag::FOREVER {
                    return Err(FederatedEndpointError::InvalidTag(tag));
                }
                let value = decoder.decode(payload)?;
                match sender.try_send(AsyncEvent::Logical {
                    tag,
                    target: crate::AsyncEventTarget::BoundaryPort(port),
                    value: Box::new(value),
                }) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(FederatedEndpointError::send(
                        "compiled inbound mailbox is full",
                    )),
                    Err(_) => Err(FederatedEndpointError::SchedulerClosed),
                }
            }),
        }
    }

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
                    target: crate::AsyncEventTarget::Action(action_ref.key()),
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

    /// Rejects non-event tags before a codec can run or an event can enter the mailbox.
    #[test]
    fn compiled_inbound_rejects_invalid_tags_before_decoding_or_admission() {
        let (tx, rx) = kanal::unbounded();
        let endpoint = FederatedInboundEndpoint::for_port(
            tx,
            crate::image::PortIndex::new(4),
            |_: &[u8]| -> Result<u32, FederatedEndpointError> {
                panic!("invalid tags must not invoke user codecs")
            },
        );
        for tag in [
            Tag::NEVER,
            Tag::FOREVER,
            Tag::new(crate::Duration::nanoseconds(-1), 0),
        ] {
            assert!(endpoint.schedule(tag, b"payload").is_err());
            assert!(rx.try_recv().unwrap().is_none());
        }
    }

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
