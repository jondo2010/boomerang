//! Bounded canonical frames over independently driven asynchronous socket halves.
//!
//! Frame lengths are checked before the standard codec reserves body storage. Buffers may
//! read ahead and retain allocator capacity; the frame limit bounds records, not exact heap
//! allocation. The writer flushes each admitted frame before accepting another.

use super::HostedError;
use boomerang_federated::wire::{self, FRAME_PREFIX_BYTES, MAX_FRAME_BYTES};
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, Stream};
use std::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
    time::Duration,
};
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    time::{sleep, timeout_at, Instant, Sleep},
};
use tokio_util::codec::{BytesCodec, Decoder, FramedRead, FramedWrite, LengthDelimitedCodec};

/// Assembles owned canonical frames; message decoding and admission belong to `WireSession`.
struct CanonicalFrameDecoder {
    /// Standard frame assembly, retaining the canonical length prefix in each returned record.
    inner: LengthDelimitedCodec,
    /// Maximum elapsed time from observing an incomplete frame until completing it.
    timeout: Duration,
    /// Absolute expiry for the incomplete frame, preserved across cancelled receive futures.
    partial_deadline: Option<Instant>,
}
impl Decoder for CanonicalFrameDecoder {
    type Item = BytesMut;
    type Error = HostedError;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<BytesMut>, HostedError> {
        let length = wire::frame_length(src).map_err(|error| HostedError::Wire(error.into()))?;
        if !src.is_empty() && length.is_none_or(|length| src.len() < length) {
            self.partial_deadline
                .get_or_insert_with(|| Instant::now() + self.timeout);
        }
        let frame = self.inner.decode(src)?;
        if frame.is_some() {
            self.partial_deadline = match wire::frame_length(src) {
                Ok(length) if !src.is_empty() && length.is_none_or(|length| src.len() < length) => {
                    Some(Instant::now() + self.timeout)
                }
                _ => None,
            };
        }
        Ok(frame)
    }
}

/// Streams bounded frames with a cancellation-safe partial-frame deadline; EOF or error ends it.
pub(super) struct Reader {
    /// Socket read ownership, buffered bytes, and canonical frame assembly state.
    framed: FramedRead<OwnedReadHalf, CanonicalFrameDecoder>,
    /// Wakeup polled for the decoder's partial deadline, retained across receive cancellation.
    timer: Pin<Box<Sleep>>,
    /// Prevents further reads after EOF or the first terminal error.
    done: bool,
}
/// Owns ordered writes; bytes already contain their authoritative canonical prefix.
pub(super) struct Writer {
    /// Socket write ownership and buffered canonical bytes awaiting an ordered flush.
    framed: FramedWrite<OwnedWriteHalf, BytesCodec>,
}
/// Separates read and write progress so a stalled write cannot prevent peer reads.
pub(super) fn split(stream: TcpStream, timeout: Duration) -> (Reader, Writer) {
    let (read, write) = stream.into_split();
    let decoder = CanonicalFrameDecoder {
        inner: LengthDelimitedCodec::builder()
            .big_endian()
            .length_field_length(FRAME_PREFIX_BYTES)
            .length_adjustment(FRAME_PREFIX_BYTES as isize)
            .num_skip(0)
            .max_frame_length(MAX_FRAME_BYTES - FRAME_PREFIX_BYTES)
            .new_codec(),
        timeout,
        partial_deadline: None,
    };
    (
        Reader {
            framed: FramedRead::new(read, decoder),
            timer: Box::pin(sleep(timeout)),
            done: false,
        },
        Writer {
            framed: FramedWrite::new(write, BytesCodec::new()),
        },
    )
}
impl Stream for Reader {
    type Item = Result<BytesMut, HostedError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        let deadline = this.framed.decoder().partial_deadline;
        // Buffered bytes must not complete a frame after its existing deadline has expired.
        let frame = if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            Poll::Pending
        } else {
            Pin::new(&mut this.framed).poll_next(cx)
        };
        let result = match frame {
            Poll::Pending => {
                let Some(deadline) = this.framed.decoder().partial_deadline else {
                    return Poll::Pending;
                };
                this.timer.as_mut().reset(deadline);
                ready!(this.timer.as_mut().poll(cx));
                Poll::Ready(Some(Err(HostedError::Lifecycle(
                    "hosted partial frame timed out",
                ))))
            }
            ready => ready,
        };
        this.done = matches!(result, Poll::Ready(None | Some(Err(_))));
        result
    }
}
impl Reader {
    /// Discards remaining bytes without reusing terminal protocol state, until EOF or deadline.
    pub(super) async fn drain(self, deadline: Instant) -> Result<(), HostedError> {
        if Instant::now() >= deadline {
            return Err(HostedError::Lifecycle("hosted receive drain timed out"));
        }
        timeout_at(
            deadline,
            tokio::io::copy(&mut self.framed.into_inner(), &mut tokio::io::sink()),
        )
        .await
        .map_err(|_| HostedError::Lifecycle("hosted receive drain timed out"))??;
        Ok(())
    }
}
impl Writer {
    /// Validates and flushes one complete frame within the caller's absolute budget.
    pub(super) async fn send(
        &mut self,
        frame: Vec<u8>,
        deadline: Instant,
    ) -> Result<(), HostedError> {
        let length = wire::frame_length(&frame).map_err(|error| HostedError::Wire(error.into()))?;
        if length != Some(frame.len()) {
            return Err(HostedError::Wire(wire::FrameError::Invalid.into()));
        }
        if Instant::now() >= deadline {
            return Err(HostedError::Lifecycle("hosted write timed out"));
        }
        timeout_at(deadline, self.framed.send(Bytes::from(frame)))
            .await
            .map_err(|_| HostedError::Lifecycle("hosted write timed out"))??;
        Ok(())
    }
    /// Flushes accepted bytes and half-closes TCP within the shared shutdown budget.
    pub(super) async fn close(&mut self, deadline: Instant) -> Result<(), HostedError> {
        if Instant::now() >= deadline {
            return Err(HostedError::Lifecycle("hosted close timed out"));
        }
        timeout_at(deadline, SinkExt::<Bytes>::close(&mut self.framed))
            .await
            .map_err(|_| HostedError::Lifecycle("hosted close timed out"))??;
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use futures_util::{StreamExt, TryStreamExt};
    use tokio::{io::AsyncWriteExt, net::TcpListener, time::timeout};

    /// Connected Tokio loopback sockets for framed I/O and supervisor tests.
    pub(in super::super) async fn sockets() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (client, server) = tokio::join!(
            TcpStream::connect(listener.local_addr().unwrap()),
            listener.accept()
        );
        (client.unwrap(), server.unwrap().0)
    }
    fn frame(body: &[u8]) -> Vec<u8> {
        let mut result = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        result.extend_from_slice(body);
        result
    }

    #[tokio::test]
    async fn canonical_frames_survive_fragmentation_and_read_ahead() {
        let (stream, mut peer) = sockets().await;
        let (mut reader, _) = split(stream, Duration::from_secs(1));
        let first = frame(b"first");
        peer.write_all(&first[..2]).await.unwrap();
        assert!(timeout(Duration::from_millis(10), reader.next())
            .await
            .is_err());
        let mut rest = first[2..].to_vec();
        let second = frame(b"second");
        rest.extend_from_slice(&second);
        peer.write_all(&rest).await.unwrap();
        assert_eq!(reader.next().await.unwrap().unwrap().as_ref(), first);
        assert_eq!(reader.next().await.unwrap().unwrap().as_ref(), second);
    }

    #[tokio::test]
    async fn idle_is_unlimited_but_cancelled_partial_read_keeps_deadline() {
        let (stream, mut peer) = sockets().await;
        let operation = Duration::from_millis(80);
        let (mut reader, _) = split(stream, operation);
        assert!(timeout(operation * 2, reader.next()).await.is_err());
        peer.write_all(&[0]).await.unwrap();
        assert!(timeout(operation / 2, reader.next()).await.is_err());
        tokio::time::sleep(operation).await;
        let error = timeout(operation / 2, reader.try_next())
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(
            error,
            HostedError::Lifecycle("hosted partial frame timed out")
        ));
        assert!(reader.next().await.is_none());
    }

    #[tokio::test]
    async fn oversized_prefix_fails_without_receiving_body() {
        let (stream, mut peer) = sockets().await;
        let (mut reader, _) = split(stream, Duration::from_secs(1));
        let capacity = reader.framed.read_buffer().capacity();
        peer.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        assert!(matches!(reader.try_next().await, Err(HostedError::Wire(_))));
        assert_eq!(reader.framed.read_buffer().capacity(), capacity);
    }

    #[tokio::test]
    async fn writer_preserves_one_prefix_and_half_close_allows_drain() {
        let (stream, peer) = sockets().await;
        let (reader, mut writer) = split(stream, Duration::from_secs(1));
        let (mut peer_reader, mut peer_writer) = split(peer, Duration::from_secs(1));
        let bytes = frame(&vec![7; MAX_FRAME_BYTES - FRAME_PREFIX_BYTES]);
        let deadline = Instant::now() + Duration::from_secs(1);
        writer.send(bytes.clone(), deadline).await.unwrap();
        assert_eq!(peer_reader.next().await.unwrap().unwrap().as_ref(), bytes);
        writer.close(deadline).await.unwrap();
        assert!(peer_reader.next().await.is_none());
        assert!(peer_reader.next().await.is_none());
        peer_writer
            .send(frame(b"remaining"), deadline)
            .await
            .unwrap();
        peer_writer.close(deadline).await.unwrap();
        reader.drain(deadline).await.unwrap();
    }

    #[tokio::test]
    async fn expired_shutdown_deadline_cannot_restart_budget() {
        let (stream, _peer) = sockets().await;
        let (reader, mut writer) = split(stream, Duration::from_secs(1));
        assert!(matches!(
            writer.close(Instant::now()).await,
            Err(HostedError::Lifecycle("hosted close timed out"))
        ));
        assert!(matches!(
            reader.drain(Instant::now()).await,
            Err(HostedError::Lifecycle("hosted receive drain timed out"))
        ));
    }

    #[tokio::test]
    async fn malformed_output_and_expired_write_deadlines_fail() {
        let (stream, _peer) = sockets().await;
        let (_, mut writer) = split(stream, Duration::from_secs(1));
        let deadline = Instant::now() + Duration::from_secs(1);
        for bytes in [
            vec![],
            vec![0; FRAME_PREFIX_BYTES],
            frame(b"x")[..FRAME_PREFIX_BYTES].to_vec(),
        ] {
            assert!(matches!(
                writer.send(bytes, deadline).await,
                Err(HostedError::Wire(_))
            ));
        }
        assert!(matches!(
            writer.send(frame(b"x"), Instant::now()).await,
            Err(HostedError::Lifecycle("hosted write timed out"))
        ));
    }
}
