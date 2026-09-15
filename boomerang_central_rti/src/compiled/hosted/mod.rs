//! Bounded Tokio TCP transport for the compiled central RTI.
//!
//! Exact echoed admission precedes typed routes. Tokio channels preserve FIFO order and
//! reserve coordination capacity; asynchronous tasks own socket I/O, deadlines and draining.
//! Synchronous scheduler owners alone wait for replies and join their transport workers.
use super::{
    CentralRtiError, CompiledRti, RtiDelivery, RtiReply, RtiReplySource, RtiRequest, RtiRequestSink,
};
#[cfg(test)]
use crate::{compiled::CoordinationIdentity, WireTag};
use boomerang_federated::{channel::Class, wire as canonical};
use boomerang_runtime::image::{FederateIndex, RtiRouteIndex};
use channel::{Envelope, Sender};
use futures_util::StreamExt;
use std::{
    io::Write,
    net::{SocketAddr, TcpListener},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::{
    sync::mpsc,
    task::JoinSet,
    time::{timeout_at, Instant},
};
use tokio_util::sync::CancellationToken;
mod channel;
mod io;
mod server;
mod wire;
pub use boomerang_federated::channel::QUEUE_CAPACITY;
pub use canonical::MAX_FRAME_BYTES;
use canonical::MAX_PAYLOAD_BYTES;
pub use server::Server;
pub use wire::WireContract;
use wire::*;

/// Original terminal failure at the hosted channel boundary.
#[derive(Debug, thiserror::Error)]
pub enum HostedError {
    /// Supervised asynchronous task failure.
    #[error("hosted task: {0}")]
    Task(#[from] tokio::task::JoinError),
    /// Standard channel or payload semaphore admission failed.
    #[error("hosted channel: {0}")]
    Channel(#[from] channel::ChannelError),
    /// Socket or OS worker failure, retaining its original error.
    #[error("hosted I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Canonical framing or exact admission failure.
    #[error("hosted wire: {0}")]
    Wire(#[from] boomerang_federated::wire::WireError),
    /// A decoded message traveled in the wrong channel direction.
    #[error("message is invalid for this channel direction")]
    Direction,
    /// The hosted lifecycle cannot continue.
    #[error("{0}")]
    Lifecycle(&'static str),
}
impl From<&'static str> for HostedError {
    fn from(message: &'static str) -> Self {
        Self::Lifecycle(message)
    }
}

/// Synchronous reply bridge and the first terminal result.
struct State {
    /// Admitted replies, preserving payload-before-grant order.
    replies: mpsc::Receiver<Envelope<RtiReply>>,
    /// Rejects new submissions after terminal acceptance, shutdown, or failure.
    closing: bool,
    /// Shared absolute shutdown budget, set once when closing starts.
    deadline: Option<Instant>,
    /// Worker completion, distinct from a requested shutdown.
    done: bool,
    /// Original failure; later shutdown errors cannot overwrite it.
    failure: Option<CentralRtiError>,
}
impl State {
    /// Records the first cause and closes producer admission.
    fn fail(&mut self, error: CentralRtiError) -> CentralRtiError {
        self.closing = true;
        self.failure.get_or_insert(error).clone()
    }
}
/// Synchronizes short queue operations; socket I/O never holds this lock.
struct Shared {
    /// Standard bounded scheduler-to-worker FIFO with payload reservation.
    requests: Sender<RtiRequest>,
    /// Serializes synchronous admission and reply notifications.
    state: Mutex<State>,
    /// Wakes the synchronous reply owner on a reply or worker completion.
    wake: Condvar,
    /// Wakes the asynchronous worker when producer lifetime cannot signal shutdown.
    closing: CancellationToken,
    /// Duration of admission, stalled operations, and the total terminal drain.
    timeout: Duration,
}
/// Constructs the scheduler handoff and the asynchronous worker's channel endpoints.
fn shared(
    timeout: Duration,
) -> (
    Arc<Shared>,
    mpsc::Receiver<Envelope<RtiRequest>>,
    Sender<RtiReply>,
) {
    let (requests, rx) = channel::bounded();
    let (replies, reply_rx) = channel::bounded();
    (
        Arc::new(Shared {
            requests,
            timeout,
            wake: Condvar::new(),
            closing: CancellationToken::new(),
            state: Mutex::new(State {
                replies: reply_rx,
                closing: false,
                deadline: None,
                done: false,
                failure: None,
            }),
        }),
        rx,
        replies,
    )
}
impl Shared {
    /// Starts one shared shutdown budget, independent of surviving producer handles.
    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closing = true;
        state
            .deadline
            .get_or_insert_with(|| Instant::now() + self.timeout);
        self.closing.cancel();
        self.wake.notify_all();
    }
}
/// Nonblocking producer shared by scheduler control and payload submissions.
struct HostedSink(Arc<Shared>);
impl RtiRequestSink for HostedSink {
    fn send(&self, mut request: RtiRequest) -> Result<(), CentralRtiError> {
        let mut state = self.0.state.lock().unwrap();
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if state.closing {
            return Err(failure("hosted connection is closed"));
        }
        let result = (|| {
            let class = if matches!(request, RtiRequest::Hello { .. }) {
                Class::Coordination
            } else {
                let message = canonical::Message::Request(request.borrowed());
                message
                    .validate_bounds()
                    .map_err(|error| HostedError::Wire(error.into()))?;
                class(&message)
            };
            let terminal = matches!(request, RtiRequest::Abort { .. });
            match &mut request {
                RtiRequest::Payload { payload, .. } => {
                    *payload = std::mem::take(payload).into_boxed_slice().into_vec();
                }
                RtiRequest::Abort { message } => {
                    *message = std::mem::take(message).into_boxed_str().into_string();
                }
                _ => {}
            }
            self.0
                .requests
                .send(request, class, Instant::now() + self.0.timeout)?;
            if terminal {
                state.closing = true;
                state.deadline = Some(Instant::now() + self.0.timeout);
                self.0.closing.cancel();
            }
            Ok(())
        })();
        result.map_err(|error| {
            let error = state.fail(error);
            state
                .deadline
                .get_or_insert_with(|| Instant::now() + self.0.timeout);
            self.0.closing.cancel();
            self.0.wake.notify_all();
            error
        })
    }
}
/// Owns one socket worker and bounded synchronous scheduler queues.
/// Retain [`Self::sink`], then pass this owner to `CentralRtiClient::connect`.
/// Dropping the owner drains accepted requests within one deadline and joins the worker,
/// even when producers retain sink handles. Waiting occurs only on the synchronous owner.
/// Call these synchronous entrypoints outside an async executor.
pub struct HostedConnection {
    /// Shared nonblocking request producer and ordered reply state.
    sink: Arc<HostedSink>,
    /// Worker joined exactly once by shutdown or owner drop.
    worker: Option<JoinHandle<()>>,
}
impl HostedConnection {
    /// Returns another producer in this connection's single ordered request domain.
    pub fn sink(&self) -> Arc<dyn RtiRequestSink> {
        self.sink.clone()
    }
    /// Closes admission, drains accepted requests, and preserves the first failure across calls.
    pub fn shutdown(&mut self) -> Result<(), CentralRtiError> {
        self.sink.0.close();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                self.sink
                    .0
                    .state
                    .lock()
                    .unwrap()
                    .fail(failure("hosted socket worker panicked"));
            }
        }
        self.sink
            .0
            .state
            .lock()
            .unwrap()
            .failure
            .clone()
            .map_or(Ok(()), Err)
    }
}
impl Drop for HostedConnection {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
impl RtiReplySource for HostedConnection {
    fn receive(&mut self, timeout: Duration) -> Result<Option<RtiReply>, CentralRtiError> {
        let shared = &self.sink.0;
        let state = shared.state.lock().unwrap();
        let (mut state, _) = shared
            .wake
            .wait_timeout_while(state, timeout, |state| {
                state.replies.is_empty() && state.failure.is_none() && !state.done
            })
            .unwrap();
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        if let Ok(reply) = state.replies.try_recv() {
            return Ok(Some(reply.into_value()));
        }
        if state.done {
            return Err(failure("hosted connection is closed"));
        }
        Ok(None)
    }
}
/// Builds the hosted I/O executor; scheduler reactions remain on their synchronous owner.
fn runtime() -> Result<tokio::runtime::Runtime, CentralRtiError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)
}
/// Connects a compiler-owned member channel; the client's Hello starts exact canonical preflight.
/// The contract borrows static generated tables; no scheduler image is copied into the worker.
pub fn connect(
    address: SocketAddr,
    member: FederateIndex,
    contract: WireContract<'static>,
    timeout: Duration,
) -> Result<HostedConnection, CentralRtiError> {
    if timeout.is_zero() {
        return Err(failure("hosted timeout must be positive"));
    }
    let _ = WireSession::new(&contract, member).map_err(HostedError::from)?;
    let runtime = runtime()?;
    let stream = runtime.block_on(async {
        timeout_at(
            Instant::now() + timeout,
            tokio::net::TcpStream::connect(address),
        )
        .await
        .map_err(|_| failure("hosted connect timed out"))?
        .map_err(failure)
    })?;
    stream.set_nodelay(true).map_err(failure)?;
    let (shared, requests, replies) = shared(timeout);
    let worker_shared = shared.clone();
    let worker = thread::Builder::new()
        .name("compiled-rti-tokio".into())
        .spawn(move || {
            let result = runtime.block_on(client_loop(
                stream,
                member,
                &contract,
                &worker_shared,
                requests,
                replies,
            ));
            let mut state = worker_shared.state.lock().unwrap();
            if let Err(error) = result {
                state.fail(error);
            }
            state.closing = true;
            state.done = true;
            worker_shared.wake.notify_all();
        })
        .map_err(failure)?;
    Ok(HostedConnection {
        sink: Arc::new(HostedSink(shared)),
        worker: Some(worker),
    })
}
/// Writes one FIFO through a framed sink; queue residence counts against each write deadline.
async fn writer_loop(
    mut writer: io::Writer,
    mut requests: mpsc::Receiver<Envelope<Vec<u8>>>,
    timeout: Duration,
) -> Result<(), CentralRtiError> {
    while let Some(frame) = requests.recv().await {
        let deadline = frame.deadline;
        writer.send(frame.into_value(), deadline).await?;
    }
    writer.close(Instant::now() + timeout).await?;
    Ok(())
}
/// Drives admission and replies concurrently with its independently supervised writer.
async fn client_loop(
    stream: tokio::net::TcpStream,
    member: FederateIndex,
    contract: &WireContract<'static>,
    shared: &Shared,
    mut requests: mpsc::Receiver<Envelope<RtiRequest>>,
    replies: Sender<RtiReply>,
) -> Result<(), CentralRtiError> {
    let (mut reader, writer) = io::split(stream, shared.timeout);
    let (output, writes) = channel::bounded();
    let mut writers = JoinSet::new();
    writers.spawn(writer_loop(writer, writes, shared.timeout));
    let mut session = WireSession::new(contract, member).map_err(HostedError::from)?;
    let admission = Instant::now() + shared.timeout;
    let mut awaiting_echo = false;
    let mut admitted = false;
    let mut terminal_sent = false;
    let mut received_terminal = false;
    let mut pending: Option<(Vec<u8>, Class, Instant)> = None;
    let mut incoming: Option<(RtiReply, Class, Instant)> = None;
    let mut closing = false;
    let result = async {
        loop {
            let deadline = {
                let state = shared.state.lock().unwrap();
                if let Some(error) = &state.failure { return Err(error.clone()); }
                state.deadline
            };
            tokio::select! {
                _ = shared.closing.cancelled(), if !closing => {
                    closing = true;
                    requests.close();
                }
                _ = tokio::time::sleep_until(deadline.unwrap_or(admission)), if deadline.is_some() || !admitted => {
                    return Err(failure(if deadline.is_some() { "hosted shutdown request drain timed out" } else { "hosted admission timed out" }));
                }
                finished = writers.join_next(), if !received_terminal => {
                    finished.unwrap().map_err(HostedError::from)??;
                    return Err(failure("hosted writer stopped unexpectedly"));
                }
                ready = async {
                    let (_, class, deadline) = pending.as_ref().unwrap();
                    output.reserve(*class, *deadline).await
                }, if pending.is_some() && !received_terminal => {
                    let (bytes, _, deadline) = pending.take().unwrap();
                    ready?.send(bytes, deadline);
                }
                ready = async {
                    let (_, class, deadline) = incoming.as_ref().unwrap();
                    replies.reserve(*class, *deadline).await
                }, if incoming.is_some() => {
                    let reservation = ready?;
                    let (reply, _, deadline) = incoming.take().unwrap();
                    let state = shared.state.lock().unwrap();
                    if let Some(error) = &state.failure { return Err(error.clone()); }
                    reservation.send(reply, deadline);
                    shared.wake.notify_all();
                    if received_terminal { return Ok(false); }
                }
                request = requests.recv(), if !awaiting_echo && pending.is_none() && !received_terminal => {
                    let Some(request) = request else { return Ok(terminal_sent); };
                    let deadline = request.deadline;
                    let request = request.into_value();
                    if let RtiRequest::Hello { identity } = request {
                        if admitted { return Err(failure("duplicate channel handshake")); }
                        let mut hello = contract.handshake(member).map_err(HostedError::from)?;
                        hello.coordination = identity;
                        let mut bytes = vec![0; MAX_FRAME_BYTES];
                        let count = canonical::encode_handshake(&hello, &mut bytes).map_err(HostedError::from)?;
                        bytes.truncate(count);
                        pending = Some((bytes, Class::Coordination, deadline));
                        awaiting_echo = true;
                    } else {
                        let message = canonical::Message::Request(request.borrowed());
                        pending = Some((encode(&mut session, &message)?, class(&message), deadline));
                        terminal_sent = matches!(request, RtiRequest::Abort { .. });
                    }
                }
                bytes = reader.next(), if !terminal_sent && incoming.is_none() && !received_terminal => {
                    let bytes = bytes.ok_or_else(|| failure("hosted socket disconnected"))??;
                    if awaiting_echo {
                        session.accept_handshake(&bytes).map_err(HostedError::from)?;
                        awaiting_echo = false;
                        admitted = true;
                    } else {
                        let message = session.decode(&bytes).map_err(HostedError::from)?;
                        let class = class(&message);
                        let reply = reply_from(message)?;
                        let terminal = matches!(reply, RtiReply::Stopped | RtiReply::Failed { .. });
                        let mut state = shared.state.lock().unwrap();
                        if let Some(error) = &state.failure { return Err(error.clone()); }
                        incoming = Some((reply, class, Instant::now() + shared.timeout));
                        if terminal {
                            state.closing = true;
                            received_terminal = true;
                            writers.abort_all();
                        }
                    }
                }
            }
        }
    }.await;
    drop(output);
    if received_terminal {
        writers.abort_all();
        while writers.join_next().await.is_some() {}
        if let Some((RtiReply::Failed { message }, _, _)) = incoming {
            return Err(CentralRtiError::new(message));
        }
        return result.map(|_| ());
    }
    let deadline = shared
        .state
        .lock()
        .unwrap()
        .deadline
        .unwrap_or_else(|| Instant::now() + shared.timeout);
    let result = match result {
        Ok(drain) => {
            let finish = async {
                writers
                    .join_next()
                    .await
                    .unwrap()
                    .map_err(HostedError::from)?
            };
            let receive = async {
                if drain {
                    reader.drain(deadline).await?;
                }
                Ok::<_, CentralRtiError>(())
            };
            match timeout_at(deadline, async {
                let (write, read) = tokio::join!(finish, receive);
                write.and(read)
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(failure("hosted shutdown drain timed out")),
            }
        }
        Err(error) => Err(error),
    };
    writers.abort_all();
    while writers.join_next().await.is_some() {}
    result
}

/// Stops dispatch immediately at the first coordinator failure, before inspecting another peer.
fn dispatch(
    rti: &mut CompiledRti<'_>,
    member: FederateIndex,
    request: RtiRequest,
) -> Result<Vec<RtiDelivery>, CentralRtiError> {
    let deliveries = rti.handle(member, request);
    if let Some(RtiDelivery {
        reply: RtiReply::Failed { message },
        ..
    }) = deliveries
        .iter()
        .find(|delivery| matches!(delivery.reply, RtiReply::Failed { .. }))
    {
        return Err(CentralRtiError::new(message));
    }
    Ok(deliveries)
}

/// Adds transport context to diagnostics at the hosted boundary.
fn failure(error: impl Into<HostedError>) -> CentralRtiError {
    error.into().into()
}

/// Serializes an application value for the hosted serde-json boundary codec.
pub fn encode_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut output = BoundedJson(Vec::new());
    serde_json::to_writer(&mut output, value)?;
    Ok(output.0)
}
/// Decodes an application value for the hosted serde-json boundary codec.
pub fn decode_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(serde_json::Error::io(payload_limit()));
    }
    serde_json::from_slice(bytes)
}

/// Bounded serializer output that refuses growth before appending oversized data.
struct BoundedJson(
    /// Application JSON bytes, excluding the hosted envelope.
    Vec<u8>,
);
impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_PAYLOAD_BYTES.saturating_sub(self.0.len()) {
            return Err(payload_limit());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Constructs the same actionable error for oversized JSON on either boundary direction.
fn payload_limit() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "hosted JSON payload exceeds maximum frame size",
    )
}

#[cfg(test)]
mod tests;
