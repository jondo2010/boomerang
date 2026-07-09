use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
};

use crate::{
    event::AsyncEvent, ActionCommon, AsyncActionRef, CommonContext, ReactorData, SendContext, Tag,
};

/// Runtime-local endpoint identity for a serialized cross-federate connection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FederatedEndpointId(String);

impl FederatedEndpointId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for FederatedEndpointId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for FederatedEndpointId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for FederatedEndpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FederatedEndpointError {
    #[error("federated payload codec error: {0}")]
    Codec(String),

    #[error("federated outbound sink error: {0}")]
    Send(String),

    #[error("federated outbound receiver error: {0}")]
    Receive(String),

    #[error("duplicate federated endpoint: {0}")]
    DuplicateEndpoint(FederatedEndpointId),

    #[error("unknown federated endpoint: {0}")]
    UnknownEndpoint(FederatedEndpointId),

    #[error("federated endpoint {endpoint} targets a physical action")]
    PhysicalAction { endpoint: FederatedEndpointId },

    #[error("federated inbound endpoint {0} scheduler channel is closed")]
    SchedulerClosed(FederatedEndpointId),

    #[error("federated outbound buffer lock poisoned")]
    PoisonedOutboundBuffer,
}

impl FederatedEndpointError {
    pub fn codec(message: impl Into<String>) -> Self {
        Self::Codec(message.into())
    }

    pub fn send(message: impl Into<String>) -> Self {
        Self::Send(message.into())
    }

    pub fn receive(message: impl Into<String>) -> Self {
        Self::Receive(message.into())
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
    pub endpoint: FederatedEndpointId,
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

/// Live outbound command sink that wakes a waiting federate client.
#[derive(Debug, Clone)]
pub struct FederatedOutboundChannel {
    sender: kanal::Sender<FederatedOutboundCommand>,
}

/// Receiving half of a live outbound command channel.
#[derive(Debug)]
pub struct FederatedOutboundReceiver {
    receiver: kanal::Receiver<FederatedOutboundCommand>,
}

impl FederatedOutboundChannel {
    pub fn pair() -> (Self, FederatedOutboundReceiver) {
        let (sender, receiver) = kanal::unbounded();
        (Self { sender }, FederatedOutboundReceiver { receiver })
    }
}

impl FederatedOutboundReceiver {
    pub fn recv(&self) -> Result<FederatedOutboundCommand, FederatedEndpointError> {
        self.receiver
            .recv()
            .map_err(|error| FederatedEndpointError::receive(error.to_string()))
    }

    pub fn try_recv(&self) -> Result<Option<FederatedOutboundCommand>, FederatedEndpointError> {
        self.receiver
            .try_recv()
            .map_err(|error| FederatedEndpointError::receive(error.to_string()))
    }
}

impl FederatedOutboundSink for FederatedOutboundChannel {
    fn send(&self, command: FederatedOutboundCommand) -> Result<(), FederatedEndpointError> {
        self.sender
            .send(command)
            .map_err(|error| FederatedEndpointError::send(error.to_string()))
    }
}

/// In-memory outbound command buffer used by builder-lowered endpoint reactions.
///
/// The buffer is an integration boundary, not the wire format. A federated client can drain these
/// commands and convert runtime tags to the protocol crate's wire tags before serialization.
#[derive(Debug, Clone, Default)]
pub struct FederatedOutboundBuffer {
    commands: Arc<Mutex<Vec<FederatedOutboundCommand>>>,
}

impl FederatedOutboundBuffer {
    pub fn drain(&self) -> Result<Vec<FederatedOutboundCommand>, FederatedEndpointError> {
        let mut commands = self
            .commands
            .lock()
            .map_err(|_| FederatedEndpointError::PoisonedOutboundBuffer)?;
        Ok(commands.drain(..).collect())
    }

    pub fn len(&self) -> Result<usize, FederatedEndpointError> {
        let commands = self
            .commands
            .lock()
            .map_err(|_| FederatedEndpointError::PoisonedOutboundBuffer)?;
        Ok(commands.len())
    }

    pub fn is_empty(&self) -> Result<bool, FederatedEndpointError> {
        self.len().map(|len| len == 0)
    }
}

impl FederatedOutboundSink for FederatedOutboundBuffer {
    fn send(&self, command: FederatedOutboundCommand) -> Result<(), FederatedEndpointError> {
        let mut commands = self
            .commands
            .lock()
            .map_err(|_| FederatedEndpointError::PoisonedOutboundBuffer)?;
        commands.push(command);
        Ok(())
    }
}

/// Outbound sink that preserves test-drainable commands and can also forward live endpoint routes.
#[derive(Debug, Clone)]
pub struct BufferedFederatedOutboundSink {
    buffer: FederatedOutboundBuffer,
    live_routes: Arc<Mutex<BTreeMap<FederatedEndpointId, FederatedOutboundChannel>>>,
}

impl BufferedFederatedOutboundSink {
    pub fn new(buffer: FederatedOutboundBuffer) -> Self {
        Self {
            buffer,
            live_routes: Arc::default(),
        }
    }

    pub fn set_live_route(
        &self,
        endpoint: FederatedEndpointId,
        channel: FederatedOutboundChannel,
    ) -> Result<(), FederatedEndpointError> {
        self.live_routes
            .lock()
            .map_err(|_| {
                FederatedEndpointError::send("federated outbound route map lock poisoned")
            })?
            .insert(endpoint, channel);
        Ok(())
    }
}

impl Default for BufferedFederatedOutboundSink {
    fn default() -> Self {
        Self::new(FederatedOutboundBuffer::default())
    }
}

impl FederatedOutboundSink for BufferedFederatedOutboundSink {
    fn send(&self, command: FederatedOutboundCommand) -> Result<(), FederatedEndpointError> {
        FederatedOutboundSink::send(&self.buffer, command.clone())?;

        let FederatedOutboundCommand::Msg(message) = &command;
        let Some(channel) = self
            .live_routes
            .lock()
            .map_err(|_| {
                FederatedEndpointError::send("federated outbound route map lock poisoned")
            })?
            .get(&message.endpoint)
            .cloned()
        else {
            return Ok(());
        };

        FederatedOutboundSink::send(&channel, command)
    }
}

trait FederatedInboundEndpoint: Send + Sync {
    fn schedule(&self, tag: Tag, payload: &[u8]) -> Result<(), FederatedEndpointError>;
}

struct TypedFederatedInboundEndpoint<T: ReactorData> {
    endpoint: FederatedEndpointId,
    context: SendContext,
    action_ref: AsyncActionRef<T>,
    decoder: Box<dyn FederatedPayloadDecoder<T>>,
}

impl<T: ReactorData> TypedFederatedInboundEndpoint<T> {
    fn new(
        endpoint: FederatedEndpointId,
        context: SendContext,
        action_ref: AsyncActionRef<T>,
        decoder: Box<dyn FederatedPayloadDecoder<T>>,
    ) -> Result<Self, FederatedEndpointError> {
        if !action_ref.is_logical() {
            return Err(FederatedEndpointError::PhysicalAction { endpoint });
        }

        Ok(Self {
            endpoint,
            context,
            action_ref,
            decoder,
        })
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
            Err(FederatedEndpointError::SchedulerClosed(
                self.endpoint.clone(),
            ))
        }
    }
}

#[derive(Default, Clone)]
pub struct FederatedInboundEndpointRegistry {
    endpoints: BTreeMap<FederatedEndpointId, Arc<dyn FederatedInboundEndpoint>>,
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
        endpoint: FederatedEndpointId,
        context: SendContext,
        action_ref: AsyncActionRef<T>,
        decoder: Box<dyn FederatedPayloadDecoder<T>>,
    ) -> Result<(), FederatedEndpointError>
    where
        T: ReactorData,
    {
        if self.endpoints.contains_key(&endpoint) {
            return Err(FederatedEndpointError::DuplicateEndpoint(endpoint));
        }

        let endpoint_id = endpoint.clone();
        let endpoint = TypedFederatedInboundEndpoint::new(endpoint, context, action_ref, decoder)?;
        self.endpoints.insert(endpoint_id, Arc::new(endpoint));
        Ok(())
    }

    pub fn schedule(
        &self,
        endpoint: &FederatedEndpointId,
        tag: Tag,
        payload: &[u8],
    ) -> Result<(), FederatedEndpointError> {
        let endpoint_handler = self
            .endpoints
            .get(endpoint)
            .ok_or_else(|| FederatedEndpointError::UnknownEndpoint(endpoint.clone()))?;
        endpoint_handler.schedule(tag, payload)
    }

    pub fn contains(&self, endpoint: &FederatedEndpointId) -> bool {
        self.endpoints.contains_key(endpoint)
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
    use std::{sync::mpsc, time::Duration};

    use super::*;

    fn outbound_command() -> FederatedOutboundCommand {
        FederatedOutboundCommand::Msg(FederatedOutboundMessage {
            endpoint: FederatedEndpointId::new("source/out->sink/in"),
            tag: Tag::ZERO,
            payload: b"7".to_vec(),
        })
    }

    #[test]
    fn outbound_channel_try_recv_delivers_exact_command() {
        let (channel, receiver) = FederatedOutboundChannel::pair();
        let command = outbound_command();

        assert_eq!(receiver.try_recv().unwrap(), None);
        FederatedOutboundSink::send(&channel, command.clone()).unwrap();

        assert_eq!(receiver.try_recv().unwrap(), Some(command));
        assert_eq!(receiver.try_recv().unwrap(), None);
    }

    #[test]
    fn outbound_channel_send_wakes_blocking_receiver() {
        let (channel, receiver) = FederatedOutboundChannel::pair();
        let command = outbound_command();
        let expected = command.clone();
        let (observed_tx, observed_rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            observed_tx.send(receiver.recv()).unwrap();
        });

        FederatedOutboundSink::send(&channel, command).unwrap();

        let observed = observed_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking outbound receiver should wake")
            .unwrap();
        assert_eq!(observed, expected);
        handle.join().unwrap();
    }

    #[test]
    fn outbound_buffer_remains_drainable_test_helper() {
        let buffer = FederatedOutboundBuffer::default();
        let command = outbound_command();

        FederatedOutboundSink::send(&buffer, command.clone()).unwrap();

        assert_eq!(buffer.len().unwrap(), 1);
        assert_eq!(buffer.drain().unwrap(), vec![command]);
        assert!(buffer.is_empty().unwrap());
    }

    #[test]
    fn buffered_outbound_sink_buffers_and_forwards_live_route() {
        let buffer = FederatedOutboundBuffer::default();
        let sink = BufferedFederatedOutboundSink::new(buffer.clone());
        let (channel, receiver) = FederatedOutboundChannel::pair();
        let command = outbound_command();
        let FederatedOutboundCommand::Msg(message) = &command;

        sink.set_live_route(message.endpoint.clone(), channel)
            .unwrap();
        FederatedOutboundSink::send(&sink, command.clone()).unwrap();

        assert_eq!(buffer.drain().unwrap(), vec![command.clone()]);
        assert_eq!(receiver.try_recv().unwrap(), Some(command));
    }
}
