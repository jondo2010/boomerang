//! The replay module is responsible for recording and replaying async events in the runtime.

use std::{collections::HashMap, path::Path};

use crate::{
    event::AsyncEvent, reaction::ReactionFn, ActionKey, ActionRef, BaseReactor, CommonContext,
    Context, Duration, EnclaveKey, ReactionRefs, ReactorData, RuntimeError, SendContext, Tag,
};

/// Re-export the `foxglove` and `mcap` crates for use in this module.
pub use {foxglove, mcap};

#[derive(thiserror::Error, Debug)]
pub enum ReplayError {
    #[error("replay I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("MCAP replay error: {0}")]
    Mcap(#[from] mcap::McapError),

    #[error("invalid replay format: {0}")]
    Format(String),

    #[error("replay serialization failed: {error}")]
    SerializationError { error: String },

    #[error("scheduler channel closed while replaying enclave {enclave_key}, action {action_key}")]
    SchedulerClosed {
        enclave_key: EnclaveKey,
        action_key: ActionKey,
    },

    #[error("replay worker panicked: {what}")]
    WorkerPanicked { what: String },
}

/// A running replay worker. Joining it reports all deferred replay failures.
#[must_use = "replay errors are reported when the handle is joined"]
#[derive(Debug)]
pub struct ReplayHandle {
    worker: std::thread::JoinHandle<Result<(), ReplayError>>,
}

impl ReplayHandle {
    pub fn join(self) -> Result<(), ReplayError> {
        self.worker
            .join()
            .map_err(|payload| ReplayError::WorkerPanicked {
                what: panic_payload_message(payload),
            })?
    }
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

impl<T> ReactionFn<'_> for RecorderFn<T>
where
    T: ReactorData + serde::Serialize,
{
    fn trigger(
        &mut self,
        ctx: &mut Context,
        _reactor: &mut dyn BaseReactor,
        refs: ReactionRefs<'_>,
    ) {
        let mut action: ActionRef<T> = refs
            .actions
            .partition_mut()
            .expect("Expected a typed action");
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
    enclaves: &tinymap::TinyMap<EnclaveKey, crate::Enclave>,
) -> Result<ReplayHandle, ReplayError>
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
        enclave_key: EnclaveKey,
        action_key: ActionKey,
        context: SendContext,
        replayer: Box<dyn ReplayFn>,
    }

    let replayers = replayers.into_iter().flat_map(|(enclave_key, replayers)| {
        replayers
            .into_iter()
            .map(move |(action_key, replayer)| (enclave_key, action_key, replayer))
    });

    let replayers = replayers
        .filter_map(|(enclave_key, action_key, replayer)| {
            let Some(ch) = channels.get(&(enclave_key, action_key)) else {
                tracing::warn!("No replay channel found for enclave_key: {enclave_key}, action_key: {action_key}");
                return None;
            };
            Some(
                enclaves
                    .get(enclave_key)
                    .ok_or_else(|| {
                        ReplayError::Format(format!(
                            "recorded enclave {enclave_key} is not present in the runtime"
                        ))
                    })
                    .map(|enclave| {
                        tracing::info!("Replaying channel {}", summary.channels[ch].topic);
                        (
                            tinymap::DefaultKey::from(*ch as usize),
                            ReplayContext {
                                enclave_key,
                                action_key,
                                context: enclave.create_send_context(enclave_key),
                                replayer,
                            },
                        )
                    }),
            )
        })
        .collect::<Result<
            tinymap::TinySecondaryMap<tinymap::DefaultKey, ReplayContext>,
            ReplayError,
        >>()?;

    let worker = std::thread::Builder::new()
        .name("boomerang-replay".to_owned())
        .spawn(move || {
            let message_stream = mcap::MessageStream::new(&mapped)?;
            for msg in message_stream {
                let inner = msg?;

                let key = tinymap::DefaultKey::from(inner.channel.id as usize);
                if let Some(replay_ctx) = replayers.get(key) {
                    let event = replay_ctx.replayer.process(&inner)?;
                    if !replay_ctx.context.schedule_external(event) {
                        return Err(ReplayError::SchedulerClosed {
                            enclave_key: replay_ctx.enclave_key,
                            action_key: replay_ctx.action_key,
                        });
                    }
                }
            }
            tracing::info!("Replay finished");
            Ok(())
        })?;

    Ok(ReplayHandle { worker })
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_owned(),
            Err(_) => "non-string panic payload".to_owned(),
        },
    }
}
