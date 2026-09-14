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

/// Serves the compiled membership until coordinated stop or terminal failure.
/// Admission, incomplete frames, queued writes and shutdown have bounded deadlines;
/// a fully admitted connection may remain idle indefinitely.
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
    runtime()?.block_on(async {
        let listener = AsyncListener::from_std(listener).map_err(failure)?;
        let mut peers = TinySecondaryMap::with_capacity(rti.member_count());
        let mut inputs = Inputs::new();
        let mut writers = JoinSet::new();
        let cancel = CancellationToken::new();
        let result = run(
            &listener,
            &mut rti,
            &contract,
            &mut peers,
            &mut inputs,
            &mut writers,
            &cancel,
            timeout,
        )
        .await;
        let deadline = Deadline::now() + timeout;
        cancel.cancel();
        let mut drains = JoinSet::new();
        while let Some(input) = inputs.next().await {
            drains.spawn(async move {
                let _ = input.reader.drain(deadline).await;
            });
        }
        let mut terminal = FuturesUnordered::new();
        if let Err(error) = &result {
            for delivery in rti.abort(error.to_string()) {
                if let Some(peer) = peers.get_mut(delivery.member) {
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
        drop(peers);
        let flushed = finish_writers(&mut writers, deadline).await;
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
    })
}

/// Coordinates admission, receive, dispatch and bounded writer transfers.
#[allow(clippy::too_many_arguments)]
async fn run<'a, 'image>(
    listener: &AsyncListener,
    rti: &mut CompiledRti<'_>,
    contract: &'a WireContract<'image>,
    peers: &mut TinySecondaryMap<FederateIndex, Peer<'a, 'image>>,
    inputs: &mut Inputs,
    writers: &mut JoinSet<Result<(), CentralRtiError>>,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<(), CentralRtiError> {
    let admission = Deadline::now() + timeout;
    let mut pending = 0;
    while !rti.is_finished() {
        let deliveries = tokio::select! {
            _ = tokio::time::sleep_until(admission), if peers.len() < rti.member_count() => {
                return Err(failure("hosted admission timed out"));
            }
            ended = writers.join_next(), if !writers.is_empty() => {
                return Err(writer_stopped(ended.expect("nonempty writer set")));
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(failure)?;
                if peers.len() + pending >= rti.member_count() {
                    return Err(failure("unexpected or duplicate hosted member connection"));
                }
                let (reader, writer) = io::split(stream, timeout);
                inputs.push(receive(None, reader, Some(writer), cancel.clone()));
                pending += 1;
                Vec::new()
            }
            input = inputs.next(), if !inputs.is_empty() => {
                let input = input.expect("nonempty reader set");
                let Some(frame) = input.frame else { return Err(failure("hosted server canceled")); };
                if let Some(member) = input.member {
                    let peer = peers.get_mut(member).expect("reader has admitted member");
                    if peer.stopped { continue; }
                    let frame = frame?;
                    let request = request_from(peer.session.decode(&frame).map_err(HostedError::from)?)?;
                    let deliveries = dispatch(rti, member, request)?;
                    inputs.push(receive(Some(member), input.reader, None, cancel.clone()));
                    deliveries
                } else {
                    let frame = frame?;
                    let hello = canonical::decode_handshake(&frame).map_err(HostedError::from)?;
                    let member = rti.resolve_member(hello.member)?;
                    if peers.contains_key(member) {
                        return Err(failure("duplicate hosted member binding"));
                    }
                    let mut session = WireSession::new(contract, member).map_err(HostedError::from)?;
                    session.accept_handshake(&frame).map_err(HostedError::from)?;
                    let (output, receiver) = channel::bounded();
                    // Queue the exact echo before any coordinator reply for this member.
                    output.send(frame.to_vec(), Class::Coordination, admission)?;
                    writers.spawn(writer_loop(input.writer.expect("pending writer"), receiver, timeout));
                    peers.insert(member, Peer { output, session, stopped: false });
                    pending -= 1;
                    inputs.push(receive(Some(member), input.reader, None, cancel.clone()));
                    dispatch(rti, member, RtiRequest::Hello { identity: hello.coordination })?
                }
            }
        };
        for delivery in deliveries {
            let peer = peers
                .get_mut(delivery.member)
                .ok_or_else(|| failure("RTI delivery has no bound hosted member"))?;
            let (bytes, class) = peer.encode(&delivery.reply)?;
            transfer(
                &peer.output,
                bytes,
                class,
                Deadline::now() + timeout,
                writers,
            )
            .await?;
        }
    }
    Ok(())
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
    use super::super::io::tests::sockets;
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A complete canonical frame used to verify exact output bytes.
    fn frame() -> Vec<u8> {
        let mut frame = vec![0; 256];
        let contract = super::super::tests::test_contract();
        let length = canonical::encode_handshake(
            &contract.handshake(FederateIndex::new(0)).unwrap(),
            &mut frame,
        )
        .unwrap();
        frame.truncate(length);
        frame
    }

    #[tokio::test]
    async fn terminal_flush_services_healthy_peers_after_another_write_fails() {
        let (mut broken, _remote) = sockets().await;
        broken.shutdown().await.unwrap();
        let (healthy, mut remote) = sockets().await;
        let frame = frame();
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
        let frame = frame();
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
