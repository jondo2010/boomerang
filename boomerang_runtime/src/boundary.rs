//! Hosted runtime adapters for compiled boundary delivery, independent of protocol and topology.
use std::{fmt, sync::Arc};

use crate::{image::PortIndex, AsyncEvent, AsyncEventTarget, ReactorData, Sender, Tag};

/// A payload could not be encoded or decoded according to its selected codec.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("payload codec failed: {0}")]
pub struct PayloadCodecError(
    /// Diagnostic supplied by the codec implementation.
    String,
);

impl PayloadCodecError {
    /// Retains a codec diagnostic without importing transport or scheduler errors.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// A transport could not accept an encoded outbound payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("boundary submission failed: {0}")]
pub struct BoundarySubmissionError(
    /// Diagnostic supplied by the transport implementation.
    String,
);

impl BoundarySubmissionError {
    /// Retains a transport diagnostic without importing codec or scheduler errors.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Encodes one concrete payload type at a serialized boundary.
pub trait PayloadEncoder<T: ReactorData>: Send + Sync + 'static {
    /// Produces bytes in the boundary's selected encoding.
    fn encode(&self, value: &T) -> Result<Vec<u8>, PayloadCodecError>;
}

impl<T: ReactorData, F: Fn(&T) -> Result<Vec<u8>, PayloadCodecError> + Send + Sync + 'static>
    PayloadEncoder<T> for F
{
    fn encode(&self, value: &T) -> Result<Vec<u8>, PayloadCodecError> {
        (self)(value)
    }
}

/// Decodes one concrete payload type at a serialized boundary.
pub trait PayloadDecoder<T: ReactorData>: Send + Sync + 'static {
    /// Validates and decodes bytes in the boundary's selected encoding.
    fn decode(&self, bytes: &[u8]) -> Result<T, PayloadCodecError>;
}

impl<T: ReactorData, F: Fn(&[u8]) -> Result<T, PayloadCodecError> + Send + Sync + 'static>
    PayloadDecoder<T> for F
{
    fn decode(&self, bytes: &[u8]) -> Result<T, PayloadCodecError> {
        (self)(bytes)
    }
}

/// Encoded application data carrying its already delay-adjusted logical tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedPayload {
    /// Destination logical tag; receivers must not apply the route delay again.
    pub tag: Tag,
    /// Bytes produced by the selected payload codec.
    pub payload: Vec<u8>,
}

/// Nonblocking submission to one bound outbound route, with no protocol frame dependency.
pub trait OutboundBoundarySink: Send + Sync + 'static {
    /// Accepts a payload in order with the owning Federate's coordination publications.
    fn send(&self, message: TaggedPayload) -> Result<(), BoundarySubmissionError>;
}

/// Failure to decode and admit a logical event to a compiled scheduler mailbox.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BoundaryAdmissionError {
    /// A logical event needs a finite, nonnegative tag.
    #[error("invalid boundary event tag: {0:?}")]
    InvalidTag(Tag),
    /// The payload failed the selected decoder's contract.
    #[error(transparent)]
    Decode(#[from] PayloadCodecError),
    /// The mailbox has no remaining capacity; no event was admitted.
    #[error("boundary mailbox is full")]
    MailboxFull,
    /// The destination scheduler has closed its mailbox.
    #[error("boundary mailbox is closed")]
    MailboxClosed,
}

/// Erased typed decode-and-admit operation bound after compiled image validation.
type Admit = dyn Fn(Tag, &[u8]) -> Result<(), BoundaryAdmissionError> + Send + Sync;

/// Delivery capability for one compiled inbound route's local port, not a graph entity.
///
/// The owning route supplies boundary identity. This hosted adapter only decodes and enqueues;
/// the scheduler and coordination backend retain responsibility for authorizing execution.
#[derive(Clone)]
pub struct InboundBoundaryAdapter {
    /// Decoder and validated port/mailbox captured after binding preflight.
    admit: Arc<Admit>,
}

impl fmt::Debug for InboundBoundaryAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InboundBoundaryAdapter")
            .finish_non_exhaustive()
    }
}

impl InboundBoundaryAdapter {
    /// Binds a typed decoder to an already validated compiled inbound port.
    pub(crate) fn for_port<T: ReactorData>(
        sender: Sender<AsyncEvent>,
        port: PortIndex,
        decoder: impl PayloadDecoder<T>,
    ) -> Self {
        Self {
            admit: Arc::new(move |tag, payload| {
                if tag < Tag::ZERO || tag >= Tag::FOREVER {
                    return Err(BoundaryAdmissionError::InvalidTag(tag));
                }
                let value = decoder.decode(payload)?;
                match sender.try_send(AsyncEvent::Logical {
                    tag,
                    target: AsyncEventTarget::BoundaryPort(port),
                    value: Box::new(value),
                }) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(BoundaryAdmissionError::MailboxFull),
                    Err(_) => Err(BoundaryAdmissionError::MailboxClosed),
                }
            }),
        }
    }

    /// Decodes and admits bytes at their final logical tag without granting execution.
    pub fn admit(&self, tag: Tag, payload: &[u8]) -> Result<(), BoundaryAdmissionError> {
        (self.admit)(tag, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keeps decoder rejection, capacity exhaustion and shutdown independently observable.
    #[test]
    fn admission_distinguishes_decode_full_and_closed_without_overwriting_queued_work() {
        let (tx, rx) = kanal::bounded(1);
        let adapter = InboundBoundaryAdapter::for_port(tx, PortIndex::new(4), |bytes: &[u8]| {
            if bytes == b"valid" {
                Ok(42_u32)
            } else {
                Err(PayloadCodecError::new("malformed"))
            }
        });
        assert_eq!(
            adapter.admit(Tag::ZERO, b"bad"),
            Err(BoundaryAdmissionError::Decode(PayloadCodecError::new(
                "malformed"
            )))
        );
        assert!(rx.try_recv().unwrap().is_none());
        adapter.admit(Tag::ZERO, b"valid").unwrap();
        assert_eq!(
            adapter.admit(Tag::ZERO, b"valid"),
            Err(BoundaryAdmissionError::MailboxFull)
        );
        assert!(rx.try_recv().unwrap().is_some());
        rx.close().unwrap();
        assert_eq!(
            adapter.admit(Tag::ZERO, b"valid"),
            Err(BoundaryAdmissionError::MailboxClosed)
        );
    }
    /// Rejects non-event tags before a codec can run or an event can enter the mailbox.
    #[test]
    fn compiled_inbound_rejects_invalid_tags_before_decoding_or_admission() {
        let (tx, rx) = kanal::unbounded();
        let adapter = InboundBoundaryAdapter::for_port(
            tx,
            crate::image::PortIndex::new(4),
            |_: &[u8]| -> Result<u32, PayloadCodecError> {
                panic!("invalid tags must not invoke user codecs")
            },
        );
        for tag in [
            Tag::NEVER,
            Tag::FOREVER,
            Tag::new(crate::Duration::nanoseconds(-1), 0),
        ] {
            assert!(adapter.admit(tag, b"payload").is_err());
            assert!(rx.try_recv().unwrap().is_none());
        }
    }
}
