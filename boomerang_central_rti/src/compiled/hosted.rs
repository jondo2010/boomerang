//! Bounded local-process TCP transport for the compiled central RTI.
//!
//! An exact canonical handshake binds each socket to a compiler-owned member before typed
//! routes are admitted. Every stage reserves coordination capacity while preserving FIFO order.
//! Socket I/O is nonblocking; partial frames, admission and final flush have deadlines.

use super::{
    CentralRtiError, CompiledRti, RtiDelivery, RtiReply, RtiReplySource, RtiRequest, RtiRequestSink,
};
#[cfg(test)]
use crate::{compiled::CoordinationIdentity, WireTag};
use boomerang_federated::{
    channel::{Class, Queue},
    wire as canonical,
};
mod wire;
use boomerang_runtime::image::{FederateIndex, RtiRouteIndex};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tinymap::TinySecondaryMap;
pub use wire::WireContract;
use wire::*;

/// Maximum frames waiting in each transport direction.
pub use boomerang_federated::channel::QUEUE_CAPACITY;
pub use canonical::MAX_FRAME_BYTES;
use canonical::MAX_PAYLOAD_BYTES;
/// Temporary polling interval until the async hosted projection replaces this worker.
const POLL: Duration = Duration::from_millis(1);

/// Original terminal failure at the hosted channel boundary.
#[derive(Debug, thiserror::Error)]
pub enum HostedError {
    /// Socket or OS worker failure, retaining its original error.
    #[error("hosted I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Canonical framing or exact admission failure.
    #[error("hosted wire: {0}")]
    Wire(#[from] boomerang_federated::wire::WireError),
    /// Fixed queue or payload reservation exhausted.
    #[error("hosted queue: {0}")]
    Queue(#[from] boomerang_federated::channel::QueueError),
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

/// One complete bounded frame retained until its final byte is written.
struct PendingWrite {
    /// Complete framed bytes, bounded by `MAX_FRAME_BYTES`.
    bytes: Vec<u8>,
    /// Prefix already accepted by the socket.
    written: usize,
    /// Enqueue instant; progress never extends this frame's deadline.
    started: Instant,
}

/// Incremental nonblocking socket with bounded frame storage in both directions.
struct FramedSocket {
    /// TCP socket operated only by its owning event loop.
    stream: TcpStream,
    /// Partial header and body for at most one frame.
    input: Vec<u8>,
    /// Partial frame arrival instant, absent while no bytes are buffered.
    reading_since: Option<Instant>,
    /// Bounded ordered frames waiting to be flushed.
    output: Queue<PendingWrite>,
    /// Maximum duration of a partial read or queued write.
    timeout: Duration,
}
impl FramedSocket {
    /// Configures a connected socket for nonblocking polling.
    fn new(stream: TcpStream, timeout: Duration) -> Result<Self, CentralRtiError> {
        if timeout.is_zero() {
            return Err(failure("hosted timeout must be positive"));
        }
        stream.set_nonblocking(true).map_err(failure)?;
        stream.set_nodelay(true).map_err(failure)?;
        Ok(Self {
            stream,
            input: Vec::with_capacity(MAX_FRAME_BYTES),
            reading_since: None,
            output: Queue::default(),
            timeout,
        })
    }
    /// Enqueues a bounded encoded frame; saturation is a terminal transport failure.
    fn queue(&mut self, bytes: Vec<u8>, class: Class) -> Result<(), CentralRtiError> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(HostedError::Wire(canonical::FrameError::Oversize.into()).into());
        }
        self.output
            .push(
                PendingWrite {
                    bytes,
                    written: 0,
                    started: Instant::now(),
                },
                class,
            )
            .map_err(HostedError::from)?;
        Ok(())
    }
    /// Makes one bounded nonblocking write, preserving frame and request ordering.
    fn flush(&mut self) -> Result<(), CentralRtiError> {
        let Some(front) = self.output.front_mut() else {
            return Ok(());
        };
        if front.started.elapsed() >= self.timeout {
            return Err(failure("hosted frame write timed out"));
        }
        match self.stream.write(&front.bytes[front.written..]) {
            Ok(0) => return Err(failure("hosted socket disconnected during write")),
            Ok(count) => front.written += count,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::Interrupted =>
            {
                return Ok(())
            }
            Err(error) => return Err(failure(error)),
        }
        if front.written == front.bytes.len() {
            self.output.pop();
        }
        Ok(())
    }
    /// Receives at most one complete frame; malformed lengths fail before allocation.
    fn receive(&mut self) -> Result<Option<Vec<u8>>, CentralRtiError> {
        if self
            .reading_since
            .is_some_and(|start| start.elapsed() >= self.timeout)
        {
            return Err(failure("hosted frame read timed out"));
        }
        for _ in 0..3 {
            let total = canonical::frame_length(&self.input)
                .map_err(|error| HostedError::Wire(error.into()))?;
            if total == Some(self.input.len()) {
                self.reading_since = None;
                return Ok(Some(std::mem::replace(
                    &mut self.input,
                    Vec::with_capacity(MAX_FRAME_BYTES),
                )));
            }
            let wanted = total.unwrap_or(canonical::FRAME_PREFIX_BYTES);
            let mut bytes = [0; 8192];
            let count = (wanted - self.input.len()).min(bytes.len());
            match self.stream.read(&mut bytes[..count]) {
                Ok(0) => return Err(failure("hosted socket disconnected")),
                Ok(count) => {
                    self.reading_since.get_or_insert_with(Instant::now);
                    self.input.extend_from_slice(&bytes[..count]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    return Ok(None)
                }
                Err(error) => return Err(failure(error)),
            }
        }
        Ok(None)
    }
}

/// Shared bounded stages and the first terminal failure observed by any connection owner.
#[derive(Default)]
struct State {
    /// Requests retained only after checking owned field bounds.
    requests: Queue<RtiRequest>,
    /// Admitted replies, preserving payload-before-grant order.
    replies: Queue<RtiReply>,
    /// Closes producer admission and starts bounded draining.
    closing: bool,
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
#[derive(Default)]
struct Shared {
    /// Bounded mutable channel stages and terminal state.
    state: Mutex<State>,
    /// Wakes the synchronous reply owner on a reply or worker completion.
    wake: Condvar,
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
            state
                .requests
                .push(request, class)
                .map_err(HostedError::from)?;
            state.closing |= terminal;
            Ok(())
        })();
        result.map_err(|error| {
            let error = state.fail(error);
            self.0.wake.notify_all();
            error
        })
    }
}

/// Owns one socket worker and bounded synchronous scheduler queues.
/// Retain [`Self::sink`], then pass this owner to `CentralRtiClient::connect`.
/// Dropping the owner drains accepted requests within one deadline and joins the worker,
/// even when producers retain sink handles. Waiting occurs only on the synchronous owner.
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
        self.sink.0.state.lock().unwrap().closing = true;
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
        if let Some(reply) = state.replies.pop() {
            return Ok(Some(reply));
        }
        if state.done {
            return Err(failure("hosted connection is closed"));
        }
        Ok(None)
    }
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
    let stream = TcpStream::connect_timeout(&address, timeout).map_err(failure)?;
    let socket = FramedSocket::new(stream, timeout)?;
    let shared = Arc::new(Shared::default());
    let worker_shared = shared.clone();
    let worker = thread::Builder::new()
        .name("compiled-rti-socket".into())
        .spawn(move || {
            let result = client_loop(socket, member, &contract, &worker_shared);
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

/// Polls the temporary socket projection, admitting ordinary traffic only after the exact echo.
fn client_loop(
    mut socket: FramedSocket,
    member: FederateIndex,
    contract: &WireContract<'static>,
    shared: &Shared,
) -> Result<(), CentralRtiError> {
    let mut session = WireSession::new(contract, member).map_err(HostedError::from)?;
    let admission = Instant::now();
    let mut awaiting_echo = false;
    let mut admitted = false;
    let mut terminal_sent = false;
    let mut deadline = None;
    loop {
        if !admitted && admission.elapsed() >= socket.timeout {
            return Err(failure("hosted admission timed out"));
        }
        let mut state = shared.state.lock().unwrap();
        if state.closing {
            deadline.get_or_insert_with(Instant::now);
        }
        if deadline.is_some_and(|start: Instant| start.elapsed() >= socket.timeout) {
            return Err(failure("hosted shutdown request drain timed out"));
        }
        if !awaiting_echo
            && state
                .requests
                .front_class()
                .is_some_and(|class| socket.output.accepts(class))
        {
            let request = state.requests.pop().unwrap();
            drop(state);
            if let RtiRequest::Hello { identity } = request {
                if admitted {
                    return Err(failure("duplicate channel handshake"));
                }
                let mut hello = contract.handshake(member).map_err(HostedError::from)?;
                hello.coordination = identity;
                let mut bytes = vec![0; MAX_FRAME_BYTES];
                let count =
                    canonical::encode_handshake(&hello, &mut bytes).map_err(HostedError::from)?;
                bytes.truncate(count);
                socket.queue(bytes, Class::Coordination)?;
                awaiting_echo = true;
            } else {
                let message = canonical::Message::Request(request.borrowed());
                socket.queue(encode(&mut session, &message)?, class(&message))?;
                terminal_sent = matches!(
                    message,
                    canonical::Message::Request(canonical::Request::Abort { .. })
                );
            }
        } else {
            if state.closing && state.requests.is_empty() && socket.output.is_empty() {
                socket
                    .stream
                    .shutdown(std::net::Shutdown::Write)
                    .map_err(failure)?;
                return Ok(());
            }
            drop(state);
        }
        socket.flush()?;
        if let Some(bytes) = if terminal_sent {
            None
        } else {
            socket.receive()?
        } {
            if awaiting_echo {
                session
                    .accept_handshake(&bytes)
                    .map_err(HostedError::from)?;
                awaiting_echo = false;
                admitted = true;
            } else {
                let message = session.decode(&bytes).map_err(HostedError::from)?;
                let class = class(&message);
                let reply = reply_from(message)?;
                let terminal = matches!(reply, RtiReply::Stopped | RtiReply::Failed { .. });
                shared
                    .state
                    .lock()
                    .unwrap()
                    .replies
                    .push(reply, class)
                    .map_err(HostedError::from)?;
                shared.wake.notify_all();
                if terminal {
                    return Ok(());
                }
            }
        }
        thread::sleep(POLL);
    }
}

/// Admitted member socket and its original typed wire session.
struct Peer<'a, 'image> {
    /// FIFO socket stages.
    socket: FramedSocket,
    /// Exact admission and typed route ownership.
    session: WireSession<'a, 'image>,
    /// Successful stop makes later disconnect expected.
    stopped: bool,
}
impl Peer<'_, '_> {
    /// Encodes a core reply without moving it ahead of an accepted payload.
    fn queue(&mut self, reply: RtiReply) -> Result<(), CentralRtiError> {
        let message = canonical::Message::Reply(reply.borrowed());
        let bytes = encode(&mut self.session, &message)?;
        self.socket.queue(bytes, class(&message))?;
        self.stopped |= matches!(reply, RtiReply::Stopped);
        Ok(())
    }
}

/// Serves exactly the compiled membership until coordinated stop or terminal failure.
/// `timeout` bounds admission, partial frames, writes and terminal flushing. Healthy idle
/// peers may remain connected indefinitely. The caller owns listener readiness publication.
pub fn serve(
    listener: TcpListener,
    mut rti: CompiledRti<'_>,
    contract: WireContract<'_>,
    timeout: Duration,
) -> Result<(), CentralRtiError> {
    if timeout.is_zero() {
        return Err(failure("hosted timeout must be positive"));
    }
    listener.set_nonblocking(true).map_err(failure)?;
    let mut peers = TinySecondaryMap::with_capacity(rti.member_count());
    let mut pending = Vec::new();
    let result = server_loop(
        &listener,
        &mut rti,
        &contract,
        &mut peers,
        &mut pending,
        timeout,
    );
    if let Err(error) = &result {
        for delivery in rti.abort(error.to_string()) {
            if let Some(peer) = peers.get_mut(delivery.member) {
                let _ = peer.queue(delivery.reply);
            }
        }
    }
    let flush_result = flush_peers(&mut peers, timeout);
    result.and(flush_result)
}

/// Runs admission and core dispatch in a single fair nonblocking event loop.
fn server_loop<'a, 'image>(
    listener: &TcpListener,
    rti: &mut CompiledRti<'_>,
    contract: &'a WireContract<'image>,
    peers: &mut TinySecondaryMap<FederateIndex, Peer<'a, 'image>>,
    pending: &mut Vec<FramedSocket>,
    timeout: Duration,
) -> Result<(), CentralRtiError> {
    let admission = Instant::now();
    while !rti.is_finished() {
        if peers.len() < rti.member_count() && admission.elapsed() >= timeout {
            return Err(failure("hosted admission timed out"));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if peers.len() + pending.len() >= rti.member_count() {
                    return Err(failure("unexpected or duplicate hosted member connection"));
                }
                pending.push(FramedSocket::new(stream, timeout)?);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(failure(error)),
        }
        let mut deliveries = Vec::new();
        let mut index = 0;
        while index < pending.len() {
            if let Some(bytes) = pending[index].receive()? {
                let hello = canonical::decode_handshake(&bytes).map_err(HostedError::from)?;
                let member = rti.resolve_member(hello.member)?;
                if peers.contains_key(member) {
                    return Err(failure("duplicate hosted member binding"));
                }
                let mut session = WireSession::new(contract, member).map_err(HostedError::from)?;
                session
                    .accept_handshake(&bytes)
                    .map_err(HostedError::from)?;
                let mut socket = pending.swap_remove(index);
                socket.queue(bytes.clone(), Class::Coordination)?;
                peers.insert(
                    member,
                    Peer {
                        socket,
                        session,
                        stopped: false,
                    },
                );
                deliveries.extend(dispatch(
                    rti,
                    member,
                    RtiRequest::Hello {
                        identity: hello.coordination,
                    },
                )?);
            } else {
                index += 1;
            }
        }
        for (member, peer) in peers.iter_mut() {
            peer.socket.flush()?;
            if peer.stopped {
                continue;
            }
            if let Some(bytes) = peer.socket.receive()? {
                let request =
                    request_from(peer.session.decode(&bytes).map_err(HostedError::from)?)?;
                deliveries.extend(dispatch(rti, member, request)?);
            }
        }
        for delivery in deliveries {
            peers
                .get_mut(delivery.member)
                .ok_or_else(|| failure("RTI delivery has no bound hosted member"))?
                .queue(delivery.reply)?;
        }
        thread::sleep(POLL);
    }
    Ok(())
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

/// Flushes terminal replies within one shared deadline, then socket owners close on return.
fn flush_peers(
    peers: &mut TinySecondaryMap<FederateIndex, Peer<'_, '_>>,
    timeout: Duration,
) -> Result<(), CentralRtiError> {
    let start = Instant::now();
    let mut first_failure = None;
    while peers.values().any(|peer| !peer.socket.output.is_empty()) {
        if start.elapsed() >= timeout {
            return Err(first_failure.unwrap_or_else(|| failure("hosted shutdown flush timed out")));
        }
        for (_, peer) in peers.iter_mut() {
            if let Err(error) = peer.socket.flush() {
                first_failure.get_or_insert(error);
                peer.socket.output = Queue::default();
            }
        }
        thread::sleep(POLL);
    }
    first_failure.map_or(Ok(()), Err)
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
