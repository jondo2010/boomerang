//! Tokio socket ownership around the borrowed compiled coordinator.

use super::{channel, io, *};
use bytes::BytesMut;
use futures_util::{future::LocalBoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use tinymap::TinySecondaryMap;
use tokio::{net::TcpListener as AsyncListener, task::JoinSet, time::Instant as Deadline};
use tokio_util::sync::CancellationToken;

/// Admitted member state owned alongside the borrowed coordinator.
struct Peer<'a, 'image> {
    /// FIFO transfer to the independently scheduled socket writer.
    output: channel::Sender<Vec<u8>>,
    /// Exact wire admission and typed route validation for this member.
    session: WireSession<'a, 'image>,
    /// A stopped member may close without failing the remaining membership.
    stopped: bool,
}
impl Peer<'_, '_> {
    /// Encodes a reply and records the terminal acknowledgement before transfer.
    fn encode(&mut self, reply: &RtiReply) -> Result<(Vec<u8>, Class), CentralRtiError> {
        let message = canonical::Message::Reply(reply.borrowed());
        let bytes = encode(&mut self.session, &message)?;
        self.stopped |= matches!(reply, RtiReply::Stopped);
        Ok((bytes, class(&message)))
    }
}

/// One completed or canceled receive, returning ownership for rescheduling or drain.
struct Input {
    /// Absent until the canonical handshake binds this socket.
    member: Option<FederateIndex>,
    /// Read half retained across receives and cancellation.
    reader: io::Reader,
    /// Pending connections transfer their writer only after successful admission.
    writer: Option<io::Writer>,
    /// Cancellation returns no frame and leaves socket ownership available.
    frame: Option<Result<BytesMut, HostedError>>,
}
/// At most one receive future per admitted or pending member.
type Inputs = FuturesUnordered<LocalBoxFuture<'static, Input>>;

/// Receives a frame while allowing shutdown to recover the read half.
fn receive(
    member: Option<FederateIndex>,
    mut reader: io::Reader,
    writer: Option<io::Writer>,
    cancel: CancellationToken,
) -> LocalBoxFuture<'static, Input> {
    async move {
        let frame = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            frame = reader.next() => Some(frame.unwrap_or(Err(HostedError::Lifecycle("hosted socket disconnected")))),
        };
        Input {
            member,
            reader,
            writer,
            frame,
        }
    }
    .boxed_local()
}

/// Owns an unstarted compiled TCP server and its borrowed image projections.
pub struct Server<'wire, 'image> {
    /// Standard listener retained until the serving runtime can register it.
    listener: TcpListener,
    /// Coordinator state borrowing its immutable compiled image.
    rti: CompiledRti<'image>,
    /// Owned admission profile borrowing the original member and route tables.
    contract: WireContract<'wire>,
    /// Bound on admission, partial frames, queued writes, and terminal draining.
    timeout: Duration,
}

impl<'wire, 'image> Server<'wire, 'image> {
    /// Takes ownership of a bound listener and admission profile without requiring a Tokio runtime.
    pub fn new(
        listener: TcpListener,
        rti: CompiledRti<'image>,
        contract: WireContract<'wire>,
        timeout: Duration,
    ) -> Result<Self, CentralRtiError> {
        if timeout.is_zero() {
            return Err(failure("hosted timeout must be positive"));
        }
        listener.set_nonblocking(true).map_err(failure)?;
        Ok(Self {
            listener,
            rti,
            contract,
            timeout,
        })
    }

    /// Serves until coordinated stop or failure, then joins all I/O within one shutdown deadline.
    /// Call from a synchronous owner; healthy admitted connections may remain idle indefinitely.
    pub fn serve(self) -> Result<(), CentralRtiError> {
        let Self {
            listener,
            rti,
            contract,
            timeout,
        } = self;
        runtime()?.block_on(async {
            RunningServer {
                listener: AsyncListener::from_std(listener).map_err(failure)?,
                peers: TinySecondaryMap::with_capacity(rti.member_count()),
                rti,
                contract: &contract,
                inputs: Inputs::new(),
                writers: JoinSet::new(),
                cancel: CancellationToken::new(),
                timeout,
            }
            .serve()
            .await
        })
    }
}

/// Active I/O state borrowing the contract held by the synchronous server owner.
struct RunningServer<'contract, 'wire, 'image> {
    /// Listener accepting only the compiled roster's bounded number of connections.
    listener: AsyncListener,
    /// Coordinator state borrowing its immutable compiled image.
    rti: CompiledRti<'image>,
    /// External immutable contract also borrowed by admitted peer sessions.
    contract: &'contract WireContract<'wire>,
    /// Admitted sessions indexed by their original compiled member keys.
    peers: TinySecondaryMap<FederateIndex, Peer<'contract, 'wire>>,
    /// At most one owned reader future per pending or admitted connection.
    inputs: Inputs,
    /// Independently driven socket writers, joined before server completion.
    writers: JoinSet<Result<(), CentralRtiError>>,
    /// Recovers read halves for draining when dispatch ends.
    cancel: CancellationToken,
    /// Bound on admission, partial frames, queued writes, and terminal draining.
    timeout: Duration,
}

impl RunningServer<'_, '_, '_> {
    /// Runs dispatch, then consumes every socket owner within one shared shutdown deadline.
    async fn serve(mut self) -> Result<(), CentralRtiError> {
        let result = self.run().await;
        let deadline = Deadline::now() + self.timeout;
        self.cancel.cancel();
        let mut drains = JoinSet::new();
        while let Some(input) = self.inputs.next().await {
            drains.spawn(async move {
                let _ = input.reader.drain(deadline).await;
            });
        }
        let mut terminal = FuturesUnordered::new();
        if let Err(error) = &result {
            for delivery in self.rti.abort(error.to_string()) {
                if let Some(peer) = self.peers.get_mut(delivery.member) {
                    if let Ok((bytes, class)) = peer.encode(&delivery.reply) {
                        let output = peer.output.clone();
                        terminal.push(async move {
                            if let Ok(reservation) = output.reserve(class, deadline).await {
                                reservation.send(bytes, deadline);
                            }
                        });
                    }
                }
            }
        }
        while terminal.next().await.is_some() {}
        drop(self.peers);
        let flushed = finish_writers(&mut self.writers, deadline).await;
        // Drain unread input until peer EOF without extending the shared shutdown budget.
        while !drains.is_empty() {
            if tokio::time::timeout_at(deadline, drains.join_next())
                .await
                .is_err()
            {
                break;
            }
        }
        drains.abort_all();
        while drains.join_next().await.is_some() {}
        result.and(flushed)
    }

    /// Coordinates admission, receive, dispatch and bounded writer transfers.
    async fn run(&mut self) -> Result<(), CentralRtiError> {
        let admission = Deadline::now() + self.timeout;
        let mut pending = 0;
        while !self.rti.is_finished() {
            let deliveries = tokio::select! {
                _ = tokio::time::sleep_until(admission), if self.peers.len() < self.rti.member_count() => {
                    return Err(failure("hosted admission timed out"));
                }
                ended = self.writers.join_next(), if !self.writers.is_empty() => {
                    return Err(writer_stopped(ended.expect("nonempty writer set")));
                }
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted.map_err(failure)?;
                    if self.peers.len() + pending >= self.rti.member_count() {
                        return Err(failure("unexpected or duplicate hosted member connection"));
                    }
                    let (reader, writer) = io::split(stream, self.timeout);
                    self.inputs.push(receive(None, reader, Some(writer), self.cancel.clone()));
                    pending += 1;
                    Vec::new()
                }
                input = self.inputs.next(), if !self.inputs.is_empty() => {
                    let input = input.expect("nonempty reader set");
                    let Some(frame) = input.frame else { return Err(failure("hosted server canceled")); };
                    if let Some(member) = input.member {
                        let peer = self.peers.get_mut(member).expect("reader has admitted member");
                        if peer.stopped { continue; }
                        let frame = frame?;
                        let request = request_from(peer.session.decode(&frame).map_err(HostedError::from)?)?;
                        let deliveries = dispatch(&mut self.rti, member, request)?;
                        self.inputs.push(receive(Some(member), input.reader, None, self.cancel.clone()));
                        deliveries
                    } else {
                        let frame = frame?;
                        let hello = canonical::decode_handshake(&frame).map_err(HostedError::from)?;
                        let member = self.rti.resolve_member(hello.member)?;
                        if self.peers.contains_key(member) {
                            return Err(failure("duplicate hosted member binding"));
                        }
                        let mut session = WireSession::new(self.contract, member).map_err(HostedError::from)?;
                        session.accept_handshake(&frame).map_err(HostedError::from)?;
                        let (output, receiver) = channel::bounded();
                        // Queue the exact echo before any coordinator reply for this member.
                        output.send(frame.to_vec(), Class::Coordination, admission)?;
                        self.writers.spawn(writer_loop(input.writer.expect("pending writer"), receiver, self.timeout));
                        self.peers.insert(member, Peer { output, session, stopped: false });
                        pending -= 1;
                        self.inputs.push(receive(Some(member), input.reader, None, self.cancel.clone()));
                        dispatch(&mut self.rti, member, RtiRequest::Hello { identity: hello.coordination })?
                    }
                }
            };
            for delivery in deliveries {
                let peer = self
                    .peers
                    .get_mut(delivery.member)
                    .ok_or_else(|| failure("RTI delivery has no bound hosted member"))?;
                let (bytes, class) = peer.encode(&delivery.reply)?;
                transfer(
                    &peer.output,
                    bytes,
                    class,
                    Deadline::now() + self.timeout,
                    &mut self.writers,
                )
                .await?;
            }
        }
        Ok(())
    }
}

/// Transfers one frame without treating a temporarily busy async consumer as failure.
async fn transfer(
    output: &channel::Sender<Vec<u8>>,
    bytes: Vec<u8>,
    class: Class,
    deadline: Deadline,
    writers: &mut JoinSet<Result<(), CentralRtiError>>,
) -> Result<(), CentralRtiError> {
    tokio::select! {
        biased;
        ended = writers.join_next(), if !writers.is_empty() => {
            Err(writer_stopped(ended.expect("nonempty writer set")))
        }
        reservation = output.reserve(class, deadline) => {
            reservation?.send(bytes, deadline);
            Ok(())
        }
    }
}

/// Retains the first writer failure, including a task panic or cancellation source.
fn writer_stopped(
    result: Result<Result<(), CentralRtiError>, tokio::task::JoinError>,
) -> CentralRtiError {
    match result {
        Ok(Err(error)) => error,
        Ok(Ok(())) => failure("hosted writer stopped unexpectedly"),
        Err(error) => failure(error),
    }
}

/// Every healthy writer receives its turn even if another writer has already failed.
async fn finish_writers(
    writers: &mut JoinSet<Result<(), CentralRtiError>>,
    deadline: Deadline,
) -> Result<(), CentralRtiError> {
    let mut failure_seen = None;
    while !writers.is_empty() {
        match tokio::time::timeout_at(deadline, writers.join_next()).await {
            Ok(Some(Ok(Ok(())))) => {}
            Ok(Some(Ok(Err(error)))) => {
                failure_seen.get_or_insert(error);
            }
            Ok(Some(Err(error))) => {
                failure_seen.get_or_insert_with(|| failure(error));
            }
            Ok(None) => break,
            Err(_) => {
                failure_seen.get_or_insert_with(|| failure("hosted shutdown flush timed out"));
                writers.abort_all();
                while writers.join_next().await.is_some() {}
            }
        }
    }
    failure_seen.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::super::io::tests::{frame, sockets};
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn terminal_flush_services_healthy_peers_after_another_write_fails() {
        let (mut broken, _remote) = sockets().await;
        broken.shutdown().await.unwrap();
        let (healthy, mut remote) = sockets().await;
        let frame = frame(b"coordination");
        let timeout = Duration::from_secs(1);
        let deadline = Deadline::now() + timeout;
        let mut writers = JoinSet::new();
        for stream in [broken, healthy] {
            let (_, writer) = io::split(stream, timeout);
            let (sender, receiver) = channel::bounded();
            sender
                .send(frame.clone(), Class::Coordination, deadline)
                .unwrap();
            drop(sender);
            writers.spawn(writer_loop(writer, receiver, timeout));
        }
        let mut received = vec![0; frame.len()];
        let (finished, delivered) = tokio::join!(
            finish_writers(&mut writers, deadline),
            tokio::time::timeout_at(deadline, remote.read_exact(&mut received)),
        );
        assert!(finished.is_err());
        delivered.unwrap().unwrap();
        assert_eq!(received, frame);
        assert!(writers.is_empty());
    }

    #[tokio::test]
    async fn async_server_transfer_waits_for_a_busy_writer() {
        let (socket, mut remote) = sockets().await;
        let frame = frame(b"coordination");
        let timeout = Duration::from_secs(1);
        let deadline = Deadline::now() + timeout;
        let (_, writer) = io::split(socket, timeout);
        let (sender, receiver) = channel::bounded();
        let mut writers = JoinSet::new();
        writers.spawn(writer_loop(writer, receiver, timeout));
        let frames = QUEUE_CAPACITY * 3;
        let expected = frame.repeat(frames);
        let consumer = tokio::spawn(async move {
            let mut received = vec![0; expected.len()];
            tokio::time::timeout_at(deadline, remote.read_exact(&mut received))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received, expected);
        });
        for _ in 0..frames {
            transfer(
                &sender,
                frame.clone(),
                Class::Coordination,
                deadline,
                &mut writers,
            )
            .await
            .unwrap();
        }
        drop(sender);
        finish_writers(&mut writers, deadline).await.unwrap();
        consumer.await.unwrap();
    }
}
