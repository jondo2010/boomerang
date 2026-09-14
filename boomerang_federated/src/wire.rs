//! Bounded canonical coordination frames, independent of transport I/O.
//!
//! A big-endian `u32` body length precedes a Serde-derived Postcard record. The
//! body contains a zero flags byte and either a handshake or ordinary traffic.
//! Record discriminants are handshake=0, traffic=1; traffic kinds are 0..=11 in
//! [`Message`](crate::wire::Message) declaration order, with 12 (PTAG) and 13 (port ABS) reserved/rejected.
//! Both records reserve `u64` epoch/incarnation fields, which must be zero.
//!
//! Postcard uses minimal unsigned varints, ZigZag signed integers, little-endian
//! IEEE floats, and varint sequence lengths. Tags use their enum discriminant:
//! never=0, finite=1 followed by `i128` nanoseconds and `u64` microstep, forever=2.
//! Decoding borrows caller bytes, checks bounds, and compares a canonical
//! re-serialization against those bytes without allocating a second frame.
//!
//! This is a closed-world format: every participant upgrades atomically and must
//! match the exact protocol, codec, and compiled identities. There is no version
//! negotiation, backward-compatible decoder, or unknown-field extension path.
//! Both endpoints validate the compiled channel member; the coordinator echoes
//! that member's identity without becoming a roster member. The transport adapter
//! enforces upstream/downstream message direction before dispatch.
use crate::WireTag;
use serde::{Deserialize, Serialize};
use tinymap::{Key, TinyMapView};

/// Exact supported coordination protocol revision.
pub const PROTOCOL_VERSION: u16 = 1;
/// Exact supported canonical framing revision.
pub const CODEC_VERSION: u16 = 2;
/// Largest application payload, independent of input or storage sizes.
pub const MAX_PAYLOAD_BYTES: usize = 65_535;
/// Largest UTF-8 diagnostic.
pub const MAX_DIAGNOSTIC_BYTES: usize = 1024;
/// Largest UTF-8 stable preflight member identity.
pub const MAX_MEMBER_BYTES: usize = 255;
/// Largest complete frame, including its four-byte length prefix.
// Prefix + flags + record + zero epoch/incarnation + kind + u32 route +
// finite tag (enum + i128 + u64 varints) + bounded payload length + payload.
pub const MAX_FRAME_BYTES: usize = 4 + 1 + 1 + 2 + 1 + 5 + 30 + 3 + MAX_PAYLOAD_BYTES;

/// Defines distinct digest claims with an identical fixed byte representation.
macro_rules! fingerprint {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $name(#[doc = "Canonical digest bytes in stable order."] [u8; 32]);
        impl $name {
            /// Wraps the canonical digest without changing its byte order.
            pub const fn new(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }
            /// Returns the digest bytes.
            pub const fn bytes(&self) -> [u8; 32] {
                self.0
            }
        }
    };
}
fingerprint!(
    CoordinationFingerprint,
    "Shared canonical coordination semantics, independent of artifact layout."
);
fingerprint!(
    FederateImageFingerprint,
    "One Federate's canonical local image, bindings, and resource bounds."
);
fingerprint!(
    ArtifactDigest,
    "Exact bytes of one built artifact; a provenance claim, not peer admission."
);

/// Compiled channel preflight identity; no stable string is present in ordinary messages.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake<'a> {
    /// Exact protocol revision.
    pub protocol: u16,
    /// Exact codec revision.
    pub codec: u16,
    /// Shared semantic coordination fingerprint.
    pub coordination: CoordinationFingerprint,
    /// Reserved federation epoch; baseline requires zero.
    pub epoch: u64,
    /// Reserved member incarnation; baseline requires zero.
    pub incarnation: u64,
    /// Digest of the canonical dense member and route mapping.
    pub mapping: [u8; 32],
    /// Compiled channel-member identity, echoed by the coordinator during preflight.
    pub member: &'a str,
}

/// A rejected frame, incompatible channel profile, or invalid session operation.
/// Errors from an existing session are terminal: discard that session after failure.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    /// The frame cannot be represented or decoded by the canonical codec.
    #[error("wire frame rejected: {0}")]
    Frame(#[from] FrameError),
    /// The frame or local image violates the compiled channel contract.
    #[error("wire admission rejected: {0}")]
    Admission(#[from] AdmissionError),
    /// Ordinary exchange preceded successful preflight.
    #[error("channel handshake has not completed")]
    NotAdmitted,
    /// The session previously failed and cannot be reused.
    #[error("channel session has failed")]
    SessionFailed,
}

/// Structural or representational failure in a complete canonical frame.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// Original Postcard failure, retained through the framing boundary.
    #[error("Postcard record failed: {0}")]
    Codec(#[from] postcard::Error),
    /// An alternate representation encoded the same value.
    #[error("record encoding is not canonical")]
    NonCanonical,

    /// Input or output exceeds a protocol bound.
    #[error("encoded value exceeds the protocol size limit")]
    Oversize,
    /// Input is incomplete or caller output storage is insufficient.
    #[error("frame bytes or caller storage are incomplete")]
    Truncated,
    /// Length, field representation, or complete consumption is noncanonical.
    #[error("invalid canonical frame encoding")]
    Invalid,
    /// A reserved message or flag has unsupported semantics.
    #[error("unsupported message kind or reserved flags")]
    Unsupported,
    /// A nonzero epoch or incarnation was supplied.
    #[error("epoch and incarnation must both be zero")]
    Epoch,
}

/// A mismatch against the compiler-owned channel profile or typed domains.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    /// The protocol revision differs.
    #[error("coordination protocol revision mismatch")]
    Protocol,
    /// The canonical codec revision differs.
    #[error("canonical codec revision mismatch")]
    Codec,
    /// The semantic coordination identity differs.
    #[error("coordination fingerprint mismatch")]
    Coordination,
    /// Dense mapping identity or local domain is invalid.
    #[error("dense mapping identity or local table is invalid")]
    Mapping,
    /// The channel member differs from the expected immutable roster entry.
    #[error("unexpected channel member")]
    Peer,
    /// A route is outside the domain or does not belong to the bound peer.
    #[error("route is outside the admitted member's domain")]
    Route,
}

/// Traffic on a preflight-bound member channel; payloads borrow caller storage.
/// Serde describes the record shape; only [`Session`] validates admission and routes.
/// Session APIs use typed `R: Key`; serialization uses the same record with raw `u32` routes.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message<'a, R> {
    /// NET publication, kind 0.
    Publish {
        /// Monotonic publication revision.
        revision: u64,
        /// Earliest event, or reversible local idle.
        next_event: Option<WireTag>,
    },
    /// LTC completion, kind 1.
    Complete {
        /// Greatest completed logical tag.
        tag: WireTag,
    },
    /// Member payload submission, kind 2.
    PayloadToRti {
        /// Validated deployment-wide route key.
        route: R,
        /// Final destination tag; delay is already applied.
        tag: WireTag,
        /// Opaque application codec bytes.
        payload: &'a [u8],
    },
    /// Terminal idle confirmation, kind 3.
    ConfirmIdle {
        /// Publication being confirmed.
        revision: u64,
    },
    /// Authorized member stop, kind 4.
    Stop,
    /// Terminal member failure, kind 5.
    Abort {
        /// Bounded original diagnostic.
        message: &'a str,
    },
    /// All expected members admitted, kind 6.
    Started,
    /// TAG authorization, kind 7.
    Grant {
        /// Publication being authorized.
        revision: u64,
        /// Authorized logical horizon.
        tag: WireTag,
    },
    /// Coordinator payload delivery, kind 8.
    PayloadToFederate {
        /// Validated deployment-wide route key.
        route: R,
        /// Final destination tag; receiver applies no further delay.
        tag: WireTag,
        /// Opaque application codec bytes.
        payload: &'a [u8],
    },
    /// Global idle confirmation, kind 9.
    Idle {
        /// Publication being confirmed.
        revision: u64,
    },
    /// Terminal stop acknowledgement, kind 10.
    Stopped,
    /// Terminal coordinator failure, kind 11.
    Failed {
        /// Bounded original diagnostic.
        message: &'a str,
    },
}

impl<'a, R: Copy> Message<'a, R> {
    /// Checks field ceilings independently of the complete-frame limit.
    fn validate_bounds(&self) -> Result<(), FrameError> {
        let oversized = match self {
            Self::PayloadToRti { payload, .. } | Self::PayloadToFederate { payload, .. } => {
                payload.len() > MAX_PAYLOAD_BYTES
            }
            Self::Abort { message } | Self::Failed { message } => {
                message.len() > MAX_DIAGNOSTIC_BYTES
            }
            _ => false,
        };
        if oversized {
            Err(FrameError::Oversize)
        } else {
            Ok(())
        }
    }
    /// Converts only route representations, preserving borrowed payloads and logical tags.
    fn map_routes<S>(
        &self,
        map: impl Fn(R, bool) -> Result<S, WireError>,
    ) -> Result<Message<'a, S>, WireError> {
        Ok(match *self {
            Self::Publish {
                revision,
                next_event,
            } => Message::Publish {
                revision,
                next_event,
            },
            Self::Complete { tag } => Message::Complete { tag },
            Self::PayloadToRti {
                route,
                tag,
                payload,
            } => Message::PayloadToRti {
                route: map(route, true)?,
                tag,
                payload,
            },
            Self::ConfirmIdle { revision } => Message::ConfirmIdle { revision },
            Self::Stop => Message::Stop,
            Self::Abort { message } => Message::Abort { message },
            Self::Started => Message::Started,
            Self::Grant { revision, tag } => Message::Grant { revision, tag },
            Self::PayloadToFederate {
                route,
                tag,
                payload,
            } => Message::PayloadToFederate {
                route: map(route, false)?,
                tag,
                payload,
            },
            Self::Idle { revision } => Message::Idle { revision },
            Self::Stopped => Message::Stopped,
            Self::Failed { message } => Message::Failed { message },
        })
    }
}

/// The immutable compiled deployment profile against which a channel admits traffic.
///
/// A contract borrows the complete member roster and route table; it owns no scheduler,
/// transport, queues, or mutable coordination state. Multiple [`Session`] values can
/// share it, each bound to one roster member. The coordinator uses that same member's
/// profile when accepting or sending traffic on its channel.
///
/// `M: Key` identifies members and `R: Key` identifies routes in separate compiler-owned
/// dense domains. `V` is the existing route-record type: it needs no trait bound because
/// the supplied `endpoints` function borrows a record and returns its source/destination
/// as `(M, M)`. No serialization bound or replacement route representation is required.
///
/// The compiler must derive `mapping` from these exact ordered tables and endpoint
/// assignments, and `coordination` from their shared protocol semantics. Construction
/// does not recompute either digest. [`Session::new`] checks roster ordering, size and
/// endpoint membership; [`Session::accept_handshake`] checks the remote identities
/// before any wire route number is converted into the original `R` domain.
pub struct Contract<'a, M: Key, R: Key, V> {
    /// Shared coordination semantics expected on this channel.
    coordination: CoordinationFingerprint,
    /// Digest of these exact canonical dense domains.
    mapping: [u8; 32],
    /// Stable roster identities in the validated member domain.
    members: TinyMapView<'a, M, &'a str>,
    /// Borrowed records in the validated route domain.
    routes: TinyMapView<'a, R, V>,
    /// Projects each route's source and destination without changing key domains.
    endpoints: fn(&V) -> (M, M),
}
impl<'a, M: Key, R: Key, V> Contract<'a, M, R, V> {
    /// Borrows the compiler's complete dense tables and records their precomputed identities.
    /// `members` must contain nonempty unique stable IDs in ascending order. Every pair
    /// returned by `endpoints` must index `members`; route ordering must match `mapping`.
    /// [`Session::new`] checks roster and endpoint validity; the compiler remains
    /// responsible for correspondence between the digests and these exact tables.
    pub const fn new(
        coordination: CoordinationFingerprint,
        mapping: [u8; 32],
        members: TinyMapView<'a, M, &'a str>,
        routes: TinyMapView<'a, R, V>,
        endpoints: fn(&V) -> (M, M),
    ) -> Self {
        Self {
            coordination,
            mapping,
            members,
            routes,
            endpoints,
        }
    }
}

/// One compiled member channel's fail-closed admission state, with no owned maps or queues.
pub struct Session<'a, 'image, M: Key, R: Key, V> {
    /// Immutable profile shared by both channel endpoints.
    contract: &'a Contract<'image, M, R, V>,
    /// Compiled member whose channel this session admits.
    peer: M,
    /// Whether exact preflight validation completed.
    admitted: bool,
    /// Sticky failure preventing further admission or exchange.
    failed: bool,
}
impl<'a, 'image, M: Key, R: Key, V> Session<'a, 'image, M, R, V> {
    /// Binds `expected_peer` to the compiled channel member before inspecting untrusted bytes.
    /// Both endpoints use that same roster entry, including the coordinator endpoint.
    pub fn new(
        contract: &'a Contract<'image, M, R, V>,
        expected_peer: M,
    ) -> Result<Self, WireError> {
        if contract.members.get(expected_peer).is_none() {
            return Err(AdmissionError::Peer.into());
        }
        if u32::try_from(contract.members.len()).is_err()
            || u32::try_from(contract.routes.len()).is_err()
        {
            return Err(AdmissionError::Mapping.into());
        }
        let mut previous = None;
        for id in contract.members.values() {
            if id.is_empty() || id.len() > MAX_MEMBER_BYTES || previous.is_some_and(|p| p >= id) {
                return Err(AdmissionError::Mapping.into());
            }
            previous = Some(id);
        }
        for route in contract.routes.values() {
            let (source, destination) = (contract.endpoints)(route);
            if contract.members.get(source).is_none() || contract.members.get(destination).is_none()
            {
                return Err(AdmissionError::Mapping.into());
            }
        }
        Ok(Self {
            contract,
            peer: expected_peer,
            admitted: false,
            failed: false,
        })
    }
    fn attempt<T>(
        &mut self,
        operation: impl FnOnce(&Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        if self.failed {
            return Err(WireError::SessionFailed);
        }
        let result = operation(self);
        self.failed |= result.is_err();
        result
    }
    /// Validates the channel profile and returns its member in the original typed domain.
    pub fn accept_handshake(&mut self, bytes: &[u8]) -> Result<M, WireError> {
        let peer = self.attempt(|s| {
            if s.admitted {
                return Err(FrameError::Invalid.into());
            }
            let Record::Handshake(hello) = decode_frame(bytes)? else {
                return Err(WireError::NotAdmitted);
            };
            if hello.protocol != PROTOCOL_VERSION {
                return Err(AdmissionError::Protocol.into());
            }
            if hello.codec != CODEC_VERSION {
                return Err(AdmissionError::Codec.into());
            }
            if hello.coordination != s.contract.coordination {
                return Err(AdmissionError::Coordination.into());
            }
            if hello.mapping != s.contract.mapping {
                return Err(AdmissionError::Mapping.into());
            }
            if hello.member != *s.contract.members.get(s.peer).ok_or(AdmissionError::Peer)? {
                return Err(AdmissionError::Peer.into());
            }
            Ok(s.peer)
        })?;
        self.admitted = true;
        Ok(peer)
    }
    fn ready(&self) -> Result<(), WireError> {
        if self.admitted {
            Ok(())
        } else {
            Err(WireError::NotAdmitted)
        }
    }
    fn check_route(&self, route: R, to_rti: bool) -> Result<(), WireError> {
        let value = self
            .contract
            .routes
            .get(route)
            .ok_or(AdmissionError::Route)?;
        let (source, destination) = (self.contract.endpoints)(value);
        if self.peer == if to_rti { source } else { destination } {
            Ok(())
        } else {
            Err(AdmissionError::Route.into())
        }
    }
    /// Range-checks a wire number before constructing a key in the actual route domain.
    fn route(&self, number: u32, to_rti: bool) -> Result<R, WireError> {
        let index = usize::try_from(number).map_err(|_| AdmissionError::Route)?;
        if index >= self.contract.routes.len() {
            return Err(AdmissionError::Route.into());
        }
        let key = R::from(index);
        self.check_route(key, to_rti)?;
        Ok(key)
    }
    /// Decodes one complete frame after preflight; payloads borrow `bytes` directly.
    pub fn decode<'b>(&mut self, bytes: &'b [u8]) -> Result<Message<'b, R>, WireError> {
        let message = self.attempt(|s| {
            s.ready()?;
            let Record::Message { message, .. } = decode_frame(bytes)? else {
                return Err(FrameError::Invalid.into());
            };
            message.map_routes(|number, to_rti| s.route(number, to_rti))
        })?;
        self.failed |= matches!(message, Message::Abort { .. } | Message::Failed { .. });
        Ok(message)
    }
    /// Encodes into caller storage after preflight; returns the complete frame length.
    pub fn encode(
        &mut self,
        message: &Message<'_, R>,
        output: &mut [u8],
    ) -> Result<usize, WireError> {
        let result = self.attempt(|s| {
            s.ready()?;
            message.validate_bounds()?;
            let message = message.map_routes(|route, to_rti| {
                s.check_route(route, to_rti)?;
                u32::try_from(route.index()).map_err(|_| AdmissionError::Route.into())
            })?;
            encode_frame(
                &Record::<Handshake<'_>, _>::Message {
                    epoch: 0,
                    incarnation: 0,
                    message,
                },
                output,
            )
        });
        self.failed |= matches!(message, Message::Abort { .. } | Message::Failed { .. });
        result
    }
}

/// Encodes a preflight record into caller storage, returning the complete frame length.
/// Nonzero reserved fields can be encoded for rejection probes; admission requires zero.
pub fn encode_handshake(handshake: &Handshake<'_>, output: &mut [u8]) -> Result<usize, WireError> {
    if handshake.member.len() > MAX_MEMBER_BYTES {
        return Err(FrameError::Oversize.into());
    }
    encode_frame(&Record::<_, Message<'_, u32>>::Handshake(handshake), output)
}

/// Derived envelope; `T` is borrowed for encoding and owned/borrowed-data for decoding.
#[derive(Serialize, Deserialize)]
struct Frame<T> {
    /// Reserved flags; no nonzero value is supported.
    flags: u8,
    /// Handshake or traffic record in declaration-defined Postcard order.
    record: T,
}
/// Distinguishes preflight from admitted traffic without serializing runtime key types.
#[derive(Serialize, Deserialize)]
enum Record<H, M> {
    /// Closed-world identity claim, including reserved epoch/incarnation fields.
    Handshake(H),
    /// Ordinary traffic carries reserved fields before its derived message record.
    Message {
        /// Reserved federation epoch; must be zero.
        epoch: u64,
        /// Reserved channel-member incarnation; must be zero.
        incarnation: u64,
        /// Decoded routes remain raw `u32` until the session validates their domain.
        message: M,
    },
}

/// Sizes a derived record before touching caller output; only the framing prefix is manual.
fn encode_frame(record: &impl Serialize, output: &mut [u8]) -> Result<usize, WireError> {
    let frame = Frame { flags: 0, record };
    let size = postcard::experimental::serialized_size(&frame).map_err(FrameError::Codec)?;
    if size > MAX_FRAME_BYTES - 4 {
        return Err(FrameError::Oversize.into());
    }
    let output = output.get_mut(..size + 4).ok_or(FrameError::Truncated)?;
    postcard::to_slice(&frame, &mut output[4..]).map_err(FrameError::Codec)?;
    output[..4].copy_from_slice(
        &u32::try_from(size)
            .map_err(|_| FrameError::Oversize)?
            .to_be_bytes(),
    );
    Ok(output.len())
}

/// Postcard serialization sink comparing canonical bytes directly with caller input.
struct Compare<'a>(&'a [u8]);
impl postcard::ser_flavors::Flavor for Compare<'_> {
    type Output = ();
    fn try_push(&mut self, byte: u8) -> postcard::Result<()> {
        self.try_extend(&[byte])
    }
    fn try_extend(&mut self, bytes: &[u8]) -> postcard::Result<()> {
        self.0 = self
            .0
            .strip_prefix(bytes)
            .ok_or(postcard::Error::SerializeBufferFull)?;
        Ok(())
    }
    fn finalize(self) -> postcard::Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(postcard::Error::SerializeBufferFull)
        }
    }
}

/// Decodes allocation-free records and rejects alternate encodings before session admission.
fn decode_frame(bytes: &[u8]) -> Result<Record<Handshake<'_>, Message<'_, u32>>, WireError> {
    let prefix: [u8; 4] = bytes
        .get(..4)
        .ok_or(FrameError::Truncated)?
        .try_into()
        .map_err(|_| FrameError::Truncated)?;
    let length = usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| FrameError::Oversize)?;
    if length > MAX_FRAME_BYTES - 4 || bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversize.into());
    }
    let body = &bytes[4..];
    if length != body.len() {
        return Err(if length > body.len() {
            FrameError::Truncated
        } else {
            FrameError::Invalid
        }
        .into());
    }
    let (frame, remaining): (Frame<Record<Handshake<'_>, Message<'_, u32>>>, _) =
        postcard::take_from_bytes(body).map_err(FrameError::Codec)?;
    if !remaining.is_empty() {
        return Err(FrameError::NonCanonical.into());
    }
    if frame.flags != 0 {
        return Err(FrameError::Unsupported.into());
    }
    let (epoch, incarnation) = match &frame.record {
        Record::Handshake(hello) => {
            if hello.member.len() > MAX_MEMBER_BYTES {
                return Err(FrameError::Oversize.into());
            }
            (hello.epoch, hello.incarnation)
        }
        Record::Message {
            epoch,
            incarnation,
            message,
        } => {
            message.validate_bounds()?;
            (*epoch, *incarnation)
        }
    };
    if epoch != 0 || incarnation != 0 {
        return Err(FrameError::Epoch.into());
    }
    postcard::serialize_with_flavor(&frame, Compare(body)).map_err(|_| FrameError::NonCanonical)?;
    Ok(frame.record)
}

#[cfg(test)]
mod tests;

mod payload;
pub use payload::{PayloadCodec, PayloadError, PortableValue, PostcardCodec};
