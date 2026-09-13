//! Bounded local-process TCP transport for the compiled central RTI.
//!
//! This versioned hosted framing is intentionally separate from the Phase 6 canonical protocol.
//! A stable member name binds each socket once; the core verifies the coordination fingerprint
//! through `Hello` before accepting dense route keys. Queues and frame storage are bounded.
//! Socket I/O is nonblocking; partial frames, admission and final flush have deadlines.

use super::{
    CentralRtiError, CompiledRti, CoordinationIdentity, RtiDelivery, RtiReply, RtiReplySource,
    RtiRequest, RtiRequestSink,
};
use crate::WireTag;
use boomerang_runtime::image::{FederateIndex, RtiRouteIndex};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tinymap::TinySecondaryMap;

/// Maximum encoded frame body, including control fields and payload bytes (one MiB).
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Maximum frames waiting in each transport direction.
pub const QUEUE_CAPACITY: usize = 16;
/// Poll interval bounds shutdown responsiveness without a background async runtime.
const POLL: Duration = Duration::from_millis(1);

/// Private hosted wire envelope, decoded only within this transport boundary.
#[derive(Debug)]
enum Frame {
    /// Versioned connection preflight containing the stable compiled member identity.
    Bind(String),
    /// Request in the fingerprint-verified image's typed route domain.
    Request(RtiRequest),
    /// Ordered server response.
    Reply(RtiReply),
}

/// Encodes a frame with explicit fixed-width fields and a bounded payload allocation.
fn encode(frame: Frame) -> Result<Vec<u8>, CentralRtiError> {
    let mut out = vec![0; 4];
    match frame {
        Frame::Bind(member) => {
            out.push(0);
            out.push(1);
            put_bytes(&mut out, member.as_bytes())?;
        }
        Frame::Request(request) => match request {
            RtiRequest::Hello { identity } => {
                out.push(1);
                out.extend(identity.bytes());
            }
            RtiRequest::Publish {
                revision,
                next_event,
            } => {
                out.push(2);
                out.extend(revision.to_be_bytes());
                out.push(u8::from(next_event.is_some()));
                if let Some(tag) = next_event {
                    put_tag(&mut out, tag);
                }
            }
            RtiRequest::Complete { tag } => {
                out.push(3);
                put_tag(&mut out, tag);
            }
            RtiRequest::Payload {
                route,
                tag,
                payload,
            } => {
                out.push(4);
                out.extend(route.as_u32().to_be_bytes());
                put_tag(&mut out, tag);
                put_bytes(&mut out, &payload)?;
            }
            RtiRequest::ConfirmIdle { revision } => {
                out.push(5);
                out.extend(revision.to_be_bytes());
            }
            RtiRequest::Stop => out.push(6),
            RtiRequest::Abort { message } => {
                out.push(7);
                put_bytes(&mut out, message.as_bytes())?;
            }
        },
        Frame::Reply(reply) => match reply {
            RtiReply::Started => out.push(8),
            RtiReply::Grant { revision, tag } => {
                out.push(9);
                out.extend(revision.to_be_bytes());
                put_tag(&mut out, tag);
            }
            RtiReply::Payload {
                route,
                tag,
                payload,
            } => {
                out.push(10);
                out.extend(route.as_u32().to_be_bytes());
                put_tag(&mut out, tag);
                put_bytes(&mut out, &payload)?;
            }
            RtiReply::Idle { revision } => {
                out.push(11);
                out.extend(revision.to_be_bytes());
            }
            RtiReply::Stopped => out.push(12),
            RtiReply::Failed { message } => {
                out.push(13);
                put_bytes(&mut out, message.as_bytes())?;
            }
        },
    }
    let len = u32::try_from(out.len() - 4).map_err(failure)?;
    out[..4].copy_from_slice(&len.to_be_bytes());
    Ok(out)
}

/// Appends a byte field only after checking the total encoded frame bound.
fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CentralRtiError> {
    if bytes.len() > MAX_FRAME_BYTES.saturating_sub(out.len()) {
        return Err(failure("hosted frame exceeds maximum size"));
    }
    out.extend(u32::try_from(bytes.len()).map_err(failure)?.to_be_bytes());
    out.extend(bytes);
    Ok(())
}

/// Encodes logical tags without process-local clocks or architecture-sized integers.
fn put_tag(out: &mut Vec<u8>, tag: WireTag) {
    match tag {
        WireTag::Never => out.push(0),
        WireTag::Finite {
            offset_ns,
            microstep,
        } => {
            out.push(1);
            out.extend(offset_ns.to_be_bytes());
            out.extend(microstep.to_be_bytes());
        }
        WireTag::Forever => out.push(2),
    }
}

/// Checked cursor over a complete, bounded frame body.
struct Decoder<'a> {
    /// Bytes not yet consumed; extra bytes are rejected at the envelope boundary.
    remaining: &'a [u8],
}
impl<'a> Decoder<'a> {
    /// Reads one fixed-width field without indexing unvalidated input.
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], CentralRtiError> {
        let (field, tail) = self
            .remaining
            .split_at_checked(N)
            .ok_or_else(|| failure("truncated hosted frame"))?;
        self.remaining = tail;
        Ok(field.try_into().expect("checked fixed field"))
    }
    /// Reads one discriminant or boolean byte.
    fn byte(&mut self) -> Result<u8, CentralRtiError> {
        Ok(self.fixed::<1>()?[0])
    }
    /// Reads a length-prefixed byte field within the already bounded frame.
    fn bytes(&mut self) -> Result<Vec<u8>, CentralRtiError> {
        let len = u32::from_be_bytes(self.fixed()?) as usize;
        let (field, tail) = self
            .remaining
            .split_at_checked(len)
            .ok_or_else(|| failure("truncated hosted frame bytes"))?;
        self.remaining = tail;
        Ok(field.to_vec())
    }
    /// Reads strict UTF-8 for member identities and terminal diagnostics.
    fn text(&mut self) -> Result<String, CentralRtiError> {
        String::from_utf8(self.bytes()?).map_err(failure)
    }
    /// Reads a wire tag with strict discriminant validation.
    fn tag(&mut self) -> Result<WireTag, CentralRtiError> {
        match self.byte()? {
            0 => Ok(WireTag::Never),
            1 => Ok(WireTag::finite(
                i128::from_be_bytes(self.fixed()?),
                u64::from_be_bytes(self.fixed()?),
            )),
            2 => Ok(WireTag::Forever),
            _ => Err(failure("invalid hosted frame tag")),
        }
    }
}

/// Decodes a complete hosted frame and rejects unknown variants or trailing bytes.
fn decode(bytes: &[u8]) -> Result<Frame, CentralRtiError> {
    let mut d = Decoder { remaining: bytes };
    let frame = match d.byte()? {
        0 => {
            if d.byte()? != 1 {
                return Err(failure("unsupported hosted frame version"));
            }
            Frame::Bind(d.text()?)
        }
        1 => Frame::Request(RtiRequest::Hello {
            identity: CoordinationIdentity::new(d.fixed()?),
        }),
        2 => {
            let revision = u64::from_be_bytes(d.fixed()?);
            let next_event = match d.byte()? {
                0 => None,
                1 => Some(d.tag()?),
                _ => return Err(failure("invalid hosted frame optional tag")),
            };
            Frame::Request(RtiRequest::Publish {
                revision,
                next_event,
            })
        }
        3 => Frame::Request(RtiRequest::Complete { tag: d.tag()? }),
        4 => Frame::Request(RtiRequest::Payload {
            route: RtiRouteIndex::new(u32::from_be_bytes(d.fixed()?)),
            tag: d.tag()?,
            payload: d.bytes()?,
        }),
        5 => Frame::Request(RtiRequest::ConfirmIdle {
            revision: u64::from_be_bytes(d.fixed()?),
        }),
        6 => Frame::Request(RtiRequest::Stop),
        7 => Frame::Request(RtiRequest::Abort { message: d.text()? }),
        8 => Frame::Reply(RtiReply::Started),
        9 => Frame::Reply(RtiReply::Grant {
            revision: u64::from_be_bytes(d.fixed()?),
            tag: d.tag()?,
        }),
        10 => Frame::Reply(RtiReply::Payload {
            route: RtiRouteIndex::new(u32::from_be_bytes(d.fixed()?)),
            tag: d.tag()?,
            payload: d.bytes()?,
        }),
        11 => Frame::Reply(RtiReply::Idle {
            revision: u64::from_be_bytes(d.fixed()?),
        }),
        12 => Frame::Reply(RtiReply::Stopped),
        13 => Frame::Reply(RtiReply::Failed { message: d.text()? }),
        _ => return Err(failure("unknown hosted frame type")),
    };
    if !d.remaining.is_empty() {
        return Err(failure("trailing hosted frame bytes"));
    }
    Ok(frame)
}

/// One partially flushed frame with its original fixed deadline.
struct PendingWrite {
    /// Complete framed bytes, bounded by `MAX_FRAME_BYTES + 4`.
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
    output: VecDeque<PendingWrite>,
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
            input: Vec::new(),
            reading_since: None,
            output: VecDeque::new(),
            timeout,
        })
    }
    /// Enqueues a bounded encoded frame; saturation is a terminal transport failure.
    fn queue(&mut self, bytes: Vec<u8>) -> Result<(), CentralRtiError> {
        if self.output.len() == QUEUE_CAPACITY {
            return Err(failure("hosted output queue is full"));
        }
        self.output.push_back(PendingWrite {
            bytes,
            written: 0,
            started: Instant::now(),
        });
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
            self.output.pop_front();
        }
        Ok(())
    }
    /// Receives at most one complete frame; malformed lengths fail before allocation.
    fn receive(&mut self) -> Result<Option<Frame>, CentralRtiError> {
        if self
            .reading_since
            .is_some_and(|start| start.elapsed() >= self.timeout)
        {
            return Err(failure("hosted frame read timed out"));
        }
        for _ in 0..2 {
            let wanted = if self.input.len() < 4 {
                4
            } else {
                let size = u32::from_be_bytes(self.input[..4].try_into().expect("complete header"))
                    as usize;
                if size == 0 || size > MAX_FRAME_BYTES {
                    return Err(failure("invalid hosted frame length"));
                }
                size + 4
            };
            if self.input.len() == wanted && wanted > 4 {
                let result = decode(&self.input[4..]);
                self.input.clear();
                self.reading_since = None;
                return result.map(Some);
            }
            let mut bytes = [0; 64 * 1024];
            let count = (wanted - self.input.len()).min(bytes.len());
            match self.stream.read(&mut bytes[..count]) {
                Ok(0) => return Err(failure("hosted socket disconnected")),
                Ok(count) => {
                    self.reading_since.get_or_insert_with(Instant::now);
                    self.input.extend_from_slice(&bytes[..count]);
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::Interrupted =>
                {
                    return Ok(None)
                }
                Err(error) => return Err(failure(error)),
            }
        }
        if self.input.len() >= 4 {
            let len =
                u32::from_be_bytes(self.input[..4].try_into().expect("complete header")) as usize;
            if len == 0 || len > MAX_FRAME_BYTES {
                return Err(failure("invalid hosted frame length"));
            }
            if self.input.len() == len + 4 {
                let result = decode(&self.input[4..]);
                self.input.clear();
                self.reading_since = None;
                return result.map(Some);
            }
        }
        Ok(None)
    }
}

/// Nonblocking producer for a single connection's reliable ordered request queue.
struct HostedSink {
    /// Bounded pre-encoded outgoing frames consumed by the socket worker.
    requests: mpsc::SyncSender<Vec<u8>>,
    /// Terminal lifecycle flag shared with the connection owner and worker.
    closed: Arc<AtomicBool>,
}
impl RtiRequestSink for HostedSink {
    fn send(&self, request: RtiRequest) -> Result<(), CentralRtiError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(failure("hosted connection is closed"));
        }
        let frame = encode(Frame::Request(request))?;
        self.requests.try_send(frame).map_err(|error| {
            self.closed.store(true, Ordering::Release);
            failure(format!("hosted request queue: {error}"))
        })
    }
}

/// Owns one socket worker and the bounded reply queue used by a compiled Federate.
///
/// Retain the sink using [`Self::sink`], then pass this owner to `CentralRtiClient::connect`.
/// Dropping the owner signals shutdown and joins its worker, even if producers still retain sinks.
pub struct HostedConnection {
    /// Shared nonblocking producer used by control and payload submissions.
    sink: Arc<HostedSink>,
    /// Bounded ordered replies decoded by the worker.
    replies: mpsc::Receiver<RtiReply>,
    /// Worker joined on explicit shutdown or owner drop.
    worker: Option<JoinHandle<Result<(), CentralRtiError>>>,
}
impl HostedConnection {
    /// Returns another producer in this connection's single ordered request domain.
    pub fn sink(&self) -> Arc<dyn RtiRequestSink> {
        self.sink.clone()
    }
    /// Closes admission, drains accepted requests within one operation timeout, and joins the worker.
    /// The drain preserves a submitted terminal Abort before the socket is closed.
    pub fn shutdown(&mut self) -> Result<(), CentralRtiError> {
        self.sink.closed.store(true, Ordering::Release);
        self.worker.take().map_or(Ok(()), |worker| {
            worker
                .join()
                .map_err(|_| failure("hosted socket worker panicked"))?
        })
    }
}
impl Drop for HostedConnection {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
impl RtiReplySource for HostedConnection {
    fn receive(&mut self, timeout: Duration) -> Result<Option<RtiReply>, CentralRtiError> {
        match self.replies.recv_timeout(timeout) {
            Ok(reply) => Ok(Some(reply)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.shutdown()?;
                Err(failure("hosted connection is closed"))
            }
        }
    }
}

/// Connects and binds a stable member name; fingerprint admission remains in `CentralRtiClient`.
pub fn connect(
    address: SocketAddr,
    member: &str,
    timeout: Duration,
) -> Result<HostedConnection, CentralRtiError> {
    if timeout.is_zero() {
        return Err(failure("hosted timeout must be positive"));
    }
    let bind = encode(Frame::Bind(member.to_owned()))?;
    let stream = TcpStream::connect_timeout(&address, timeout).map_err(failure)?;
    let mut socket = FramedSocket::new(stream, timeout)?;
    socket.queue(bind)?;
    let (requests, request_rx) = mpsc::sync_channel(QUEUE_CAPACITY);
    let (reply_tx, replies) = mpsc::sync_channel(QUEUE_CAPACITY);
    let closed = Arc::new(AtomicBool::new(false));
    let worker_closed = closed.clone();
    let worker = thread::Builder::new()
        .name("compiled-rti-socket".into())
        .spawn(move || {
            let result = client_loop(socket, request_rx, reply_tx, &worker_closed);
            worker_closed.store(true, Ordering::Release);
            result
        })
        .map_err(failure)?;
    Ok(HostedConnection {
        sink: Arc::new(HostedSink { requests, closed }),
        replies,
        worker: Some(worker),
    })
}

/// Pumps bounded queues in one socket worker without blocking request producers.
fn client_loop(
    mut socket: FramedSocket,
    requests: mpsc::Receiver<Vec<u8>>,
    replies: mpsc::SyncSender<RtiReply>,
    closed: &AtomicBool,
) -> Result<(), CentralRtiError> {
    while !closed.load(Ordering::Acquire) {
        if socket.output.len() < QUEUE_CAPACITY {
            match requests.try_recv() {
                Ok(bytes) => socket.queue(bytes)?,
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return drain_requests(&mut socket, &requests)
                }
            }
        }
        socket.flush()?;
        if let Some(frame) = socket.receive()? {
            let Frame::Reply(reply) = frame else {
                return Err(failure("unexpected hosted server frame"));
            };
            let terminal = matches!(reply, RtiReply::Stopped | RtiReply::Failed { .. });
            replies
                .try_send(reply)
                .map_err(|error| failure(format!("hosted reply queue: {error}")))?;
            if terminal {
                return Ok(());
            }
        }
        thread::sleep(POLL);
    }
    drain_requests(&mut socket, &requests)
}

/// Flushes accepted requests after producer admission closes, using one shared shutdown deadline.
fn drain_requests(
    socket: &mut FramedSocket,
    requests: &mpsc::Receiver<Vec<u8>>,
) -> Result<(), CentralRtiError> {
    let started = Instant::now();
    loop {
        if started.elapsed() >= socket.timeout {
            return Err(failure("hosted shutdown request drain timed out"));
        }
        let mut requests_empty = false;
        while socket.output.len() < QUEUE_CAPACITY {
            match requests.try_recv() {
                Ok(bytes) => socket.queue(bytes)?,
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {
                    requests_empty = true;
                    break;
                }
            }
        }
        socket.flush()?;
        if requests_empty && socket.output.is_empty() {
            // A FIN follows the complete ordered request stream, including its terminal Abort.
            socket
                .stream
                .shutdown(std::net::Shutdown::Write)
                .map_err(failure)?;
            return Ok(());
        }
        thread::sleep(POLL);
    }
}

/// One bound server peer, kept in the compiled member's typed sparse domain.
struct Peer {
    /// Socket carrying ordered control and payload frames.
    socket: FramedSocket,
    /// Whether the core accepted this peer's fingerprint-bearing Hello.
    admitted: bool,
    /// Whether the core has acknowledged stop, so disconnect is expected.
    stopped: bool,
}

/// Serves exactly the compiled membership until coordinated stop or terminal failure.
///
/// `timeout` bounds admission, partial frames, and terminal flushing. Healthy idle peers may
/// remain connected indefinitely. The caller owns listener readiness publication; this function starts no threads and borrows the compiled image.
pub fn serve(
    listener: TcpListener,
    mut rti: CompiledRti<'_>,
    timeout: Duration,
) -> Result<(), CentralRtiError> {
    if timeout.is_zero() {
        return Err(failure("hosted timeout must be positive"));
    }
    listener.set_nonblocking(true).map_err(failure)?;
    let mut peers = TinySecondaryMap::with_capacity(rti.member_count());
    let mut pending = Vec::new();
    let result = server_loop(&listener, &mut rti, &mut peers, &mut pending, timeout);
    if let Err(error) = &result {
        // Best effort terminal diagnostics; socket closure still releases saturated or broken peers.
        for delivery in rti.abort(error.to_string()) {
            if let Some(peer) = peers.get_mut(delivery.member) {
                if let Ok(frame) = encode(Frame::Reply(delivery.reply)) {
                    let _ = peer.socket.queue(frame);
                }
            }
        }
    }
    let flush_result = flush_peers(&mut peers, timeout);
    result.and(flush_result)
}

/// Runs admission and core dispatch in a single fair nonblocking event loop.
fn server_loop(
    listener: &TcpListener,
    rti: &mut CompiledRti<'_>,
    peers: &mut TinySecondaryMap<FederateIndex, Peer>,
    pending: &mut Vec<FramedSocket>,
    timeout: Duration,
) -> Result<(), CentralRtiError> {
    let admission = Instant::now();
    while !rti.is_finished() {
        if peers.values().filter(|peer| peer.admitted).count() < rti.member_count()
            && admission.elapsed() >= timeout
        {
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
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(failure(error)),
        }
        let mut index = 0;
        while index < pending.len() {
            if let Some(frame) = pending[index].receive()? {
                let Frame::Bind(identity) = frame else {
                    return Err(failure("hosted connection requires member binding"));
                };
                let member = rti.resolve_member(&identity)?;
                if peers.contains_key(member) {
                    return Err(failure("duplicate hosted member binding"));
                }
                peers.insert(
                    member,
                    Peer {
                        socket: pending.swap_remove(index),
                        admitted: false,
                        stopped: false,
                    },
                );
            } else {
                index += 1;
            }
        }
        let mut deliveries = Vec::new();
        for (member, peer) in peers.iter_mut() {
            peer.socket.flush()?;
            if peer.stopped {
                continue;
            }
            if let Some(frame) = peer.socket.receive()? {
                let Frame::Request(request) = frame else {
                    return Err(failure("unexpected hosted client frame"));
                };
                if !peer.admitted && !matches!(request, RtiRequest::Hello { .. }) {
                    return Err(failure("hosted request before fingerprint admission"));
                }
                let replies = rti.handle(member, request);
                if let Some(RtiDelivery {
                    reply: RtiReply::Failed { message },
                    ..
                }) = replies
                    .iter()
                    .find(|delivery| matches!(delivery.reply, RtiReply::Failed { .. }))
                {
                    return Err(failure(message));
                }
                peer.admitted = true;
                deliveries.extend(replies);
            }
        }
        for delivery in deliveries {
            let peer = peers
                .get_mut(delivery.member)
                .ok_or_else(|| failure("RTI delivery has no bound hosted member"))?;
            if matches!(delivery.reply, RtiReply::Stopped) {
                peer.stopped = true;
            }
            peer.socket.queue(encode(Frame::Reply(delivery.reply))?)?;
        }
        thread::sleep(POLL);
    }
    Ok(())
}

/// Flushes terminal replies within one shared deadline, then socket owners close on return.
fn flush_peers(
    peers: &mut TinySecondaryMap<FederateIndex, Peer>,
    timeout: Duration,
) -> Result<(), CentralRtiError> {
    let start = Instant::now();
    while peers.values().any(|peer| !peer.socket.output.is_empty()) {
        if start.elapsed() >= timeout {
            return Err(failure("hosted shutdown flush timed out"));
        }
        for (_, peer) in peers.iter_mut() {
            peer.socket.flush()?;
        }
        thread::sleep(POLL);
    }
    Ok(())
}

/// Adds transport context to diagnostics at the hosted boundary.
fn failure(error: impl std::fmt::Display) -> CentralRtiError {
    CentralRtiError::new(error.to_string())
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

/// Maximum application bytes after reserving opcode, route, finite tag and payload length.
const MAX_PAYLOAD_BYTES: usize = MAX_FRAME_BYTES - 34;

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
