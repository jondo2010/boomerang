use std::{
    future::Future,
    pin::Pin,
    sync::mpsc::{self, Receiver, Sender},
};

pub type TransportFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("transport peer is closed")]
    Closed,
}

/// Async sink for ordered protocol frames.
pub trait FrameSink<M>: Send {
    fn send<'a>(&'a mut self, frame: M) -> TransportFuture<'a, Result<(), TransportError>>;
}

/// Async stream for ordered protocol frames.
pub trait FrameStream<M>: Send {
    fn recv<'a>(&'a mut self) -> TransportFuture<'a, Result<Option<M>, TransportError>>;
}

/// In-memory ordered transport for deterministic protocol tests.
#[derive(Debug)]
pub struct InMemoryTransport<Outgoing, Incoming> {
    sender: Sender<Outgoing>,
    receiver: Receiver<Incoming>,
}

impl<Outgoing, Incoming> InMemoryTransport<Outgoing, Incoming> {
    fn new(sender: Sender<Outgoing>, receiver: Receiver<Incoming>) -> Self {
        Self { sender, receiver }
    }
}

pub fn in_memory_transport_pair<A, B>() -> (InMemoryTransport<A, B>, InMemoryTransport<B, A>) {
    let (a_sender, a_receiver) = mpsc::channel();
    let (b_sender, b_receiver) = mpsc::channel();

    (
        InMemoryTransport::new(a_sender, b_receiver),
        InMemoryTransport::new(b_sender, a_receiver),
    )
}

impl<Outgoing, Incoming> FrameSink<Outgoing> for InMemoryTransport<Outgoing, Incoming>
where
    Outgoing: Send + 'static,
    Incoming: Send + 'static,
{
    fn send<'a>(&'a mut self, frame: Outgoing) -> TransportFuture<'a, Result<(), TransportError>> {
        Box::pin(async move { self.sender.send(frame).map_err(|_| TransportError::Closed) })
    }
}

impl<Outgoing, Incoming> FrameStream<Incoming> for InMemoryTransport<Outgoing, Incoming>
where
    Outgoing: Send + 'static,
    Incoming: Send + 'static,
{
    fn recv<'a>(&'a mut self) -> TransportFuture<'a, Result<Option<Incoming>, TransportError>> {
        Box::pin(async move {
            match self.receiver.recv() {
                Ok(frame) => Ok(Some(frame)),
                Err(_) => Ok(None),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
    };

    use super::*;
    use crate::{
        protocol::{FederateId, FederateToRti, RtiToFederate, WireTag},
        ProtocolFrame,
    };

    struct NoopWaker;

    impl Wake for NoopWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWaker));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    #[test]
    fn memory_transport_delivers_frames_in_order() {
        let (mut federate, mut rti) = in_memory_transport_pair::<FederateToRti, RtiToFederate>();

        block_on(federate.send(FederateToRti::Net {
            federate_id: FederateId::new("fed-a"),
            tag: WireTag::ZERO,
        }))
        .unwrap();
        block_on(federate.send(FederateToRti::Ltc {
            federate_id: FederateId::new("fed-a"),
            tag: WireTag::ZERO,
        }))
        .unwrap();

        assert_eq!(
            block_on(rti.recv()).unwrap(),
            Some(FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            })
        );
        assert_eq!(
            block_on(rti.recv()).unwrap(),
            Some(FederateToRti::Ltc {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            })
        );
    }

    #[test]
    fn memory_transport_is_bidirectional() {
        let (mut federate, mut rti) = in_memory_transport_pair::<ProtocolFrame, ProtocolFrame>();

        block_on(
            federate.send(ProtocolFrame::FederateToRti(FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            })),
        )
        .unwrap();
        block_on(rti.send(ProtocolFrame::RtiToFederate(RtiToFederate::Tag {
            tag: WireTag::ZERO,
        })))
        .unwrap();

        assert_eq!(
            block_on(rti.recv()).unwrap(),
            Some(ProtocolFrame::FederateToRti(FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            }))
        );
        assert_eq!(
            block_on(federate.recv()).unwrap(),
            Some(ProtocolFrame::RtiToFederate(RtiToFederate::Tag {
                tag: WireTag::ZERO,
            }))
        );
    }

    #[test]
    fn memory_transport_reports_end_of_stream_when_peer_drops() {
        let (federate, mut rti) = in_memory_transport_pair::<FederateToRti, RtiToFederate>();
        drop(federate);

        assert_eq!(block_on(rti.recv()).unwrap(), None);
    }
}
