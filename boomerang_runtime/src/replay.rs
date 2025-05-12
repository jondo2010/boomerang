//! The replay module is responsible for recording and replaying async events in the runtime.

use std::{collections::HashMap, path::Path};

use crate::{
    event::AsyncEvent, ActionKey, ActionRef, CommonContext, Enclave, EnclaveKey, ReactionFn,
    ReactorData, RuntimeError, SendContext, Tag,
};

use time::Duration;
use tinymap::TinyMap;
use tinymap::{DefaultKey, TinySecondaryMap};
/// Re-export the `foxglove` and `mcap` crates for use in this module.
pub use {foxglove, mcap};

#[derive(thiserror::Error, Debug)]
#[error("...")]
pub enum ReplayError {
    Io(#[from] std::io::Error),

    Mcap(#[from] mcap::McapError),

    #[error("MCAP Format error: {0}")]
    Format(String),
    SerializationError {
        error: String,
    },
}

const ENCLAVE: &str = "enclave";
const ACTION: &str = "action";

/// RecorderFn implements the [`ReactionFn`] trait for recording events.
/// It's used as the reaction fn, and is triggered on the action we want to record.
pub struct RecorderFn<T: serde::Serialize> {
    channel: foxglove::Channel<EncodeWrapper<T>>,
}

impl<T> RecorderFn<T>
where
    T: ReactorData + serde::Serialize,
{
    pub fn new(
        topic: &str,
        enclave_key: EnclaveKey,
        action_key: ActionKey,
    ) -> Result<Self, RuntimeError> {
        let channel = foxglove::ChannelBuilder::new(topic)
            .add_metadata(ENCLAVE, &enclave_key.to_string())
            .add_metadata(ACTION, &action_key.to_string())
            .build();
        Ok(Self { channel })
    }
}

impl<'store, T> ReactionFn<'store> for RecorderFn<T>
where
    T: ReactorData + serde::Serialize,
{
    fn trigger(
        &mut self,
        ctx: &mut crate::Context,
        _reactor: &mut dyn crate::BaseReactor,
        _ports: crate::Refs<dyn crate::BasePort>,
        _ports_mut: crate::RefsMut<dyn crate::BasePort>,
        actions: crate::RefsMut<dyn crate::BaseAction>,
    ) {
        let mut action: ActionRef<T> = actions.partition_mut().expect("Expected a typed action");
        let val = ctx
            .get_action_value::<T>(&mut action)
            .expect("Failed to get action value");
        let timestamp = ctx.get_elapsed_logical_time().whole_nanoseconds();
        self.channel.log_with_meta(
            EncodeWrapper::new(val),
            foxglove::PartialMetadata {
                log_time: Some(timestamp as _),
            },
        );
    }
}

#[derive(serde::Serialize)]
struct EncodeWrapper<T: serde::Serialize>(T);

impl<T: serde::Serialize> EncodeWrapper<T> {
    fn new(inner: &T) -> &EncodeWrapper<T> {
        // SAFETY: This is safe because we are not mutating the inner value.
        // The lifetime of the inner value is tied to the lifetime of the wrapper.
        unsafe { &*(inner as *const T as *const EncodeWrapper<T>) }
    }
}

impl<T: serde::Serialize> foxglove::Encode for EncodeWrapper<T> {
    type Error = crate::RuntimeError;

    fn get_schema() -> Option<foxglove::Schema> {
        None
    }

    fn get_message_encoding() -> String {
        "json".to_string()
    }

    fn encode(&self, buf: &mut impl bytes::BufMut) -> Result<(), Self::Error> {
        let json =
            serde_json::to_string(&self.0).map_err(|e| crate::RuntimeError::EncodeError {
                error: e.to_string(),
            })?;
        buf.put_slice(json.as_bytes());
        Ok(())
    }
}

/// ReplayFn trait defines the behavior for replaying events from recorded messages
pub trait ReplayFn: Send + 'static {
    /// Process a message and convert it to an AsyncEvent
    fn process(&self, msg: &mcap::Message<'_>) -> Result<AsyncEvent, ReplayError>;

    /// Get the action key this replayer is associated with
    fn action_key(&self) -> ActionKey;
}

/// TypedReplayer implements the ReplayFn trait for a specific type T
pub struct TypedReplayer<T>
where
    T: ReactorData + for<'de> serde::Deserialize<'de> + 'static,
{
    action_key: ActionKey,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> TypedReplayer<T>
where
    T: ReactorData + for<'de> serde::Deserialize<'de> + 'static,
{
    pub fn new(action_key: ActionKey) -> Self {
        Self {
            action_key,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T> ReplayFn for TypedReplayer<T>
where
    T: ReactorData + for<'de> serde::Deserialize<'de> + 'static,
{
    fn process(&self, msg: &mcap::Message<'_>) -> Result<AsyncEvent, ReplayError> {
        let tag = Tag::new(Duration::nanoseconds(msg.log_time as _), 0);
        let value: T =
            serde_json::from_slice(&msg.data).map_err(|e| ReplayError::SerializationError {
                error: e.to_string(),
            })?;
        Ok(AsyncEvent::Logical {
            tag,
            key: self.action_key,
            value: Box::new(value),
        })
    }

    fn action_key(&self) -> ActionKey {
        self.action_key
    }
}

pub type ReplayersMap =
    tinymap::TinySecondaryMap<EnclaveKey, tinymap::TinySecondaryMap<ActionKey, Box<dyn ReplayFn>>>;

/// Replay the recorded messages from an MCAP file (`path`) using the provided replayers.
pub fn create_replayer<P>(
    path: P,
    replayers: ReplayersMap,
    enclaves: &tinymap::TinyMap<EnclaveKey, Enclave>,
) -> Result<(), ReplayError>
where
    P: AsRef<Path>,
{
    let fd = std::fs::File::open(path.as_ref())?;
    let mapped = unsafe { memmap2::Mmap::map(&fd) }?;

    let summary = mcap::Summary::read(&mapped)?
        .ok_or_else(|| ReplayError::Format("Missing summary in MCAP file".to_string()))?;

    // Create a temporary mapping of (EnclaveKey, ActionKey) to mcap channel ID
    let channels: HashMap<(EnclaveKey, ActionKey), u16> = summary
        .channels
        .iter()
        .map(|(id, ch)| {
            let enclave_key = ch
                .metadata
                .get(ENCLAVE)
                .ok_or_else(|| {
                    ReplayError::Format(format!("Missing enclave metadata in channel: {id}"))
                })?
                .parse()
                .map_err(|_| {
                    ReplayError::Format(format!("Invalid enclave metadata in channel: {id}"))
                })?;
            let action_key = ch
                .metadata
                .get(ACTION)
                .ok_or_else(|| {
                    ReplayError::Format(format!("Missing action metadata in channel: {id}"))
                })?
                .parse()
                .map_err(|_| {
                    ReplayError::Format(format!("Invalid action metadata in channel: {id}"))
                })?;
            Ok(((enclave_key, action_key), *id))
        })
        .collect::<Result<_, ReplayError>>()?;

    if channels.is_empty() {
        return Err(ReplayError::Format(
            "No channels found in MCAP file".to_string(),
        ));
    } else {
        tracing::info!("Found {} replayable channels in MCAP file", channels.len());
    }

    struct ReplayContext {
        context: SendContext,
        replayer: Box<dyn ReplayFn>,
    }

    let replayers = replayers.into_iter().flat_map(|(enclave_key, replayers)| {
        replayers
            .into_iter()
            .map(move |(action_key, replayer)| (enclave_key, action_key, replayer))
    });

    let replayers = replayers.filter_map(|(enclave_key, action_key, replayer)| {
        if let Some(ch) = channels.get(&(enclave_key, action_key)) {
            let enclave = enclaves.get(enclave_key).expect("Enclave not found");
            tracing::info!("Replaying channel {}", summary.channels[ch].topic);
            Some((DefaultKey::from(*ch as usize), ReplayContext {
                context: enclave.create_send_context(enclave_key),
                replayer
            }))
        } else {
            tracing::warn!("No replay channel found for enclave_key: {enclave_key}, action_key: {action_key}");
            None
        }
    }).collect::<TinySecondaryMap<DefaultKey, ReplayContext>>();

    std::thread::spawn(move || {
        let message_stream = mcap::MessageStream::new(&mapped).unwrap();
        for msg in message_stream {
            let inner = msg.unwrap();

            let key = DefaultKey::from(inner.channel.id as usize);
            if let Some(replay_ctx) = replayers.get(key) {
                replay_ctx
                    .replayer
                    .process(&inner)
                    .map(|event| {
                        if !replay_ctx.context.schedule_external(event) {
                            tracing::error!("Failed to schedule event");
                        }
                    })
                    .unwrap_or_else(|e| {
                        tracing::error!("Failed to replay message: {}", e);
                    });
            }
        }
        tracing::info!("Replay finished");
    });

    Ok(())
}
