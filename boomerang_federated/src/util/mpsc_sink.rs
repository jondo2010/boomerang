//! Copied from https://github.com/herblet/sender-sink/blob/main/src/wrappers/mpsc_sink.rs
use std::task::{Context, Poll};

use futures::Sink;
use thiserror::Error;
use tokio::sync::mpsc;

/// Wraps an UnboundedSender in a Sink
#[derive(Clone)]
pub struct UnboundedSenderSink<T> {
    sender: Option<mpsc::UnboundedSender<T>>,
}

#[derive(Debug, Error)]
pub enum SinkError {
    #[error("Channel closed")]
    ChannelClosed,
    #[error("Send failed")]
    SendFailed,
}

impl<T> UnboundedSenderSink<T> {
    fn sender_if_open(&mut self) -> Option<&mpsc::UnboundedSender<T>> {
        match &self.sender {
            None => None,
            Some(sender) => {
                if sender.is_closed() {
                    // drop the actual sender, leaving an empty option
                    &self.sender.take();
                    None
                } else {
                    self.sender.as_ref()
                }
            }
        }
    }

    fn ok_unless_closed(&mut self) -> Poll<Result<(), SinkError>> {
        Poll::Ready(
            self.sender_if_open()
                .map(|_| ())
                .ok_or_else(|| SinkError::ChannelClosed),
        )
    }
}

impl<T> Unpin for UnboundedSenderSink<T> {}

impl<T> From<mpsc::UnboundedSender<T>> for UnboundedSenderSink<T> {
    fn from(sender: mpsc::UnboundedSender<T>) -> Self {
        UnboundedSenderSink {
            sender: Some(sender),
        }
    }
}

impl<T> Sink<T> for UnboundedSenderSink<T> {
    type Error = SinkError;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.ok_unless_closed()
    }

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        self.sender_if_open()
            .map(|sender| sender.send(item).map_err(|_| SinkError::SendFailed))
            .unwrap_or_else(|| Err(SinkError::ChannelClosed))
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.ok_unless_closed()
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        //drop the sender
        self.sender.take();
        Poll::Ready(Ok(()))
    }
}
