//! Bounded canonical Phase 6 wire frames; synchronous and independent of transport I/O.
//!
//! Every frame starts with a big-endian `u32` body length, then a one-byte kind,
//! one reserved flags byte, and zero-valued `u64` epoch and incarnation fields.
//! Tags occupy 25 bytes: kind (never=0, finite=1, forever=2), signed big-endian
//! `i128` nanoseconds, and big-endian `u64` microstep; sentinel padding is zero.
//! Dense routes use `u32`, revisions `u64`, payload lengths `u32`, text lengths
//! `u16`, all big-endian. Kind 0 is preflight; kinds 1..=12 are baseline messages;
//! 13 (PTAG) and 14 (port ABS) are reserved and rejected. No extension is skipped.
//!
//! Both channel endpoints validate the same compiled member profile: the coordinator
//! echoes that member's handshake identity without becoming a Federate roster member.
//! The #134 transport adapter must enforce upstream/downstream message direction
//! before dispatch; this codec validates canonical bytes and channel membership.
use crate::WireTag;
use tinymap::{Key, TinyMapView};

/// Exact supported coordination protocol revision.
pub const PROTOCOL_VERSION: u16 = 1;
/// Exact supported canonical framing revision.
pub const CODEC_VERSION: u16 = 1;
/// Largest application payload, independent of input or storage sizes.
pub const MAX_PAYLOAD_BYTES: usize = 65_535;
/// Largest UTF-8 diagnostic.
pub const MAX_DIAGNOSTIC_BYTES: usize = 1024;
/// Largest UTF-8 stable preflight member identity.
pub const MAX_MEMBER_BYTES: usize = 255;
/// Largest complete frame, including its four-byte length prefix.
pub const MAX_FRAME_BYTES: usize = 4 + 18 + 4 + 25 + 4 + MAX_PAYLOAD_BYTES;

/// Defines distinct digest claims with an identical fixed byte representation.
macro_rules! fingerprint {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, PartialEq, Eq)]
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
#[derive(Debug, PartialEq, Eq)]
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

/// Precise framing or admission failure. Every session error is terminal.
#[derive(Debug, PartialEq, Eq)]
pub enum WireError {
    /// Input or output exceeds a protocol bound.
    Oversize,
    /// A required byte or caller output byte is unavailable.
    Truncated,
    /// The complete frame length or canonical encoding is invalid.
    Invalid,
    /// A reserved message or flag has unsupported semantics.
    Unsupported,
    /// The protocol revision differs.
    Protocol,
    /// The canonical codec revision differs.
    Codec,
    /// The semantic coordination identity differs.
    Coordination,
    /// A nonzero epoch or incarnation was supplied.
    Epoch,
    /// Dense mapping identity or local domain is invalid.
    Mapping,
    /// The channel-member identity differs from the immutable expected roster entry.
    Peer,
    /// A route is outside the domain or does not belong to the bound peer.
    Route,
    /// Ordinary exchange preceded successful preflight.
    NotAdmitted,
    /// The session previously failed and cannot be reused.
    SessionFailed,
}
impl core::fmt::Display for WireError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "wire protocol: {self:?}")
    }
}
impl core::error::Error for WireError {}

/// Baseline traffic on a preflight-bound member channel; payloads borrow caller storage.
#[derive(Debug, PartialEq, Eq)]
pub enum Message<'a, R> {
    /// NET publication, kind 1.
    Publish {
        /// Monotonic publication revision.
        revision: u64,
        /// Earliest event, or reversible local idle.
        next_event: Option<WireTag>,
    },
    /// LTC completion, kind 2.
    Complete {
        /// Greatest completed logical tag.
        tag: WireTag,
    },
    /// Member payload submission, kind 3.
    PayloadToRti {
        /// Validated deployment-wide route key.
        route: R,
        /// Final destination tag; delay is already applied.
        tag: WireTag,
        /// Opaque application codec bytes.
        payload: &'a [u8],
    },
    /// Terminal idle confirmation, kind 4.
    ConfirmIdle {
        /// Publication being confirmed.
        revision: u64,
    },
    /// Authorized member stop, kind 5.
    Stop,
    /// Terminal member failure, kind 6.
    Abort {
        /// Bounded original diagnostic.
        message: &'a str,
    },
    /// All expected members admitted, kind 7.
    Started,
    /// TAG authorization, kind 8.
    Grant {
        /// Publication being authorized.
        revision: u64,
        /// Authorized logical horizon.
        tag: WireTag,
    },
    /// Coordinator payload delivery, kind 9.
    PayloadToFederate {
        /// Validated deployment-wide route key.
        route: R,
        /// Final destination tag; receiver applies no further delay.
        tag: WireTag,
        /// Opaque application codec bytes.
        payload: &'a [u8],
    },
    /// Global idle confirmation, kind 10.
    Idle {
        /// Publication being confirmed.
        revision: u64,
    },
    /// Terminal stop acknowledgement, kind 11.
    Stopped,
    /// Terminal coordinator failure, kind 12.
    Failed {
        /// Bounded original diagnostic.
        message: &'a str,
    },
}

/// Immutable compiler-owned mapping borrowed from validated member and route domains.
/// The owning compiler must compute `mapping` from these exact domains and semantics.
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
    /// Borrows actual dense tables; `endpoints` projects typed source and destination members.
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
            return Err(WireError::Peer);
        }
        if u32::try_from(contract.members.len()).is_err()
            || u32::try_from(contract.routes.len()).is_err()
        {
            return Err(WireError::Mapping);
        }
        let mut previous = None;
        for id in contract.members.values() {
            if id.is_empty() || id.len() > MAX_MEMBER_BYTES || previous.is_some_and(|p| p >= id) {
                return Err(WireError::Mapping);
            }
            previous = Some(id);
        }
        for route in contract.routes.values() {
            let (source, destination) = (contract.endpoints)(route);
            if contract.members.get(source).is_none() || contract.members.get(destination).is_none()
            {
                return Err(WireError::Mapping);
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
                return Err(WireError::Invalid);
            }
            let (kind, mut d) = Decoder::frame(bytes)?;
            if kind != 0 {
                return Err(WireError::NotAdmitted);
            }
            if u16::from_be_bytes(d.fixed()?) != PROTOCOL_VERSION {
                return Err(WireError::Protocol);
            }
            if u16::from_be_bytes(d.fixed()?) != CODEC_VERSION {
                return Err(WireError::Codec);
            }
            if d.fixed::<32>()? != s.contract.coordination.bytes() {
                return Err(WireError::Coordination);
            }
            if d.fixed::<32>()? != s.contract.mapping {
                return Err(WireError::Mapping);
            }
            if d.text(MAX_MEMBER_BYTES)?
                != *s.contract.members.get(s.peer).ok_or(WireError::Peer)?
            {
                return Err(WireError::Peer);
            }
            d.finish()?;
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
        let value = self.contract.routes.get(route).ok_or(WireError::Route)?;
        let (source, destination) = (self.contract.endpoints)(value);
        if self.peer == if to_rti { source } else { destination } {
            Ok(())
        } else {
            Err(WireError::Route)
        }
    }
    fn route(&self, d: &mut Decoder<'_>, to_rti: bool) -> Result<R, WireError> {
        // The sole wire-representation adapter: range-check in the actual domain before construction.
        let index =
            usize::try_from(u32::from_be_bytes(d.fixed()?)).map_err(|_| WireError::Route)?;
        if index >= self.contract.routes.len() {
            return Err(WireError::Route);
        }
        let key = R::from(index);
        self.check_route(key, to_rti)?;
        Ok(key)
    }
    /// Decodes one complete frame only after preflight; payloads borrow `bytes` directly.
    pub fn decode<'b>(&mut self, bytes: &'b [u8]) -> Result<Message<'b, R>, WireError> {
        let message = self.attempt(|s| {
            s.ready()?;
            let (kind, mut d) = Decoder::frame(bytes)?;
            let message = match kind {
                1 => Message::Publish {
                    revision: d.revision()?,
                    next_event: match d.byte()? {
                        0 => None,
                        1 => Some(d.tag()?),
                        _ => return Err(WireError::Invalid),
                    },
                },
                2 => Message::Complete { tag: d.tag()? },
                3 => Message::PayloadToRti {
                    route: s.route(&mut d, true)?,
                    tag: d.tag()?,
                    payload: d.payload()?,
                },
                4 => Message::ConfirmIdle {
                    revision: d.revision()?,
                },
                5 => Message::Stop,
                6 => Message::Abort {
                    message: d.text(MAX_DIAGNOSTIC_BYTES)?,
                },
                7 => Message::Started,
                8 => Message::Grant {
                    revision: d.revision()?,
                    tag: d.tag()?,
                },
                9 => Message::PayloadToFederate {
                    route: s.route(&mut d, false)?,
                    tag: d.tag()?,
                    payload: d.payload()?,
                },
                10 => Message::Idle {
                    revision: d.revision()?,
                },
                11 => Message::Stopped,
                12 => Message::Failed {
                    message: d.text(MAX_DIAGNOSTIC_BYTES)?,
                },
                _ => return Err(WireError::Unsupported),
            };
            d.finish()?;
            Ok(message)
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
            let kind = match message {
                Message::Publish { .. } => 1,
                Message::Complete { .. } => 2,
                Message::PayloadToRti { .. } => 3,
                Message::ConfirmIdle { .. } => 4,
                Message::Stop => 5,
                Message::Abort { .. } => 6,
                Message::Started => 7,
                Message::Grant { .. } => 8,
                Message::PayloadToFederate { .. } => 9,
                Message::Idle { .. } => 10,
                Message::Stopped => 11,
                Message::Failed { .. } => 12,
            };
            encode_frame(kind, 0, 0, output, |w| {
                match message {
                    Message::Publish {
                        revision,
                        next_event,
                    } => {
                        w.put(&revision.to_be_bytes())?;
                        w.put(&[u8::from(next_event.is_some())])?;
                        if let Some(tag) = next_event {
                            w.tag(tag)?;
                        }
                    }
                    Message::Complete { tag } => w.tag(tag)?,
                    Message::PayloadToRti {
                        route,
                        tag,
                        payload,
                    }
                    | Message::PayloadToFederate {
                        route,
                        tag,
                        payload,
                    } => {
                        if payload.len() > MAX_PAYLOAD_BYTES {
                            return Err(WireError::Oversize);
                        }
                        s.check_route(*route, kind == 3)?;
                        w.put(
                            &u32::try_from(route.index())
                                .map_err(|_| WireError::Route)?
                                .to_be_bytes(),
                        )?;
                        w.tag(tag)?;
                        w.put(
                            &u32::try_from(payload.len())
                                .map_err(|_| WireError::Oversize)?
                                .to_be_bytes(),
                        )?;
                        w.put(payload)?;
                    }
                    Message::ConfirmIdle { revision } | Message::Idle { revision } => {
                        w.put(&revision.to_be_bytes())?
                    }
                    Message::Grant { revision, tag } => {
                        w.put(&revision.to_be_bytes())?;
                        w.tag(tag)?;
                    }
                    Message::Abort { message } | Message::Failed { message } => {
                        w.text(message, MAX_DIAGNOSTIC_BYTES)?
                    }
                    Message::Stop | Message::Started | Message::Stopped => (),
                }
                Ok(())
            })
        });
        self.failed |= matches!(message, Message::Abort { .. } | Message::Failed { .. });
        result
    }
}

/// Encodes a stable preflight record into caller storage, returning the frame length.
/// Nonbaseline fields may be encoded for interoperability probes; admission rejects them.
pub fn encode_handshake(handshake: &Handshake<'_>, output: &mut [u8]) -> Result<usize, WireError> {
    encode_frame(0, handshake.epoch, handshake.incarnation, output, |w| {
        w.put(&handshake.protocol.to_be_bytes())?;
        w.put(&handshake.codec.to_be_bytes())?;
        w.put(&handshake.coordination.bytes())?;
        w.put(&handshake.mapping)?;
        w.text(handshake.member, MAX_MEMBER_BYTES)
    })
}

/// Bounds-checking sink used first for sizing and then for caller-buffer encoding.
struct Writer<'a> {
    /// Caller storage, absent during the sizing pass.
    output: Option<&'a mut [u8]>,
    /// Checked number of frame bytes counted or written.
    length: usize,
}
impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), WireError> {
        let end = self
            .length
            .checked_add(bytes.len())
            .ok_or(WireError::Oversize)?;
        if end > MAX_FRAME_BYTES {
            return Err(WireError::Oversize);
        }
        if let Some(output) = &mut self.output {
            output
                .get_mut(self.length..end)
                .ok_or(WireError::Truncated)?
                .copy_from_slice(bytes);
        }
        self.length = end;
        Ok(())
    }
    fn text(&mut self, text: &str, bound: usize) -> Result<(), WireError> {
        if text.len() > bound {
            return Err(WireError::Oversize);
        }
        self.put(
            &u16::try_from(text.len())
                .map_err(|_| WireError::Oversize)?
                .to_be_bytes(),
        )?;
        self.put(text.as_bytes())
    }
    fn tag(&mut self, tag: &WireTag) -> Result<(), WireError> {
        let (kind, offset, microstep) = match tag {
            WireTag::Never => (0, 0, 0),
            WireTag::Finite {
                offset_ns,
                microstep,
            } => (1, *offset_ns, *microstep),
            WireTag::Forever => (2, 0, 0),
        };
        self.put(&[kind])?;
        self.put(&offset.to_be_bytes())?;
        self.put(&microstep.to_be_bytes())
    }
}
/// Sizes and validates the body before emitting a complete frame into caller storage.
fn encode_frame(
    kind: u8,
    epoch: u64,
    incarnation: u64,
    output: &mut [u8],
    body: impl Fn(&mut Writer<'_>) -> Result<(), WireError>,
) -> Result<usize, WireError> {
    let mut size = Writer {
        output: None,
        length: 22,
    };
    body(&mut size)?;
    if output.len() < size.length {
        return Err(WireError::Truncated);
    }
    let mut w = Writer {
        output: Some(output),
        length: 0,
    };
    w.put(
        &u32::try_from(size.length - 4)
            .map_err(|_| WireError::Oversize)?
            .to_be_bytes(),
    )?;
    w.put(&[kind, 0])?;
    w.put(&epoch.to_be_bytes())?;
    w.put(&incarnation.to_be_bytes())?;
    body(&mut w)?;
    Ok(w.length)
}
/// Borrowed cursor over one exactly bounded canonical frame.
struct Decoder<'a> {
    /// Unconsumed bytes; exposed payloads remain borrowed from this same input.
    remaining: &'a [u8],
}
impl<'a> Decoder<'a> {
    fn frame(bytes: &'a [u8]) -> Result<(u8, Self), WireError> {
        let mut d = Self { remaining: bytes };
        let length =
            usize::try_from(u32::from_be_bytes(d.fixed()?)).map_err(|_| WireError::Oversize)?;
        if length > MAX_FRAME_BYTES - 4 || bytes.len() > MAX_FRAME_BYTES {
            return Err(WireError::Oversize);
        }
        if length != d.remaining.len() {
            return Err(if length > d.remaining.len() {
                WireError::Truncated
            } else {
                WireError::Invalid
            });
        }
        let kind = d.byte()?;
        if d.byte()? != 0 {
            return Err(WireError::Unsupported);
        }
        if d.revision()? != 0 || d.revision()? != 0 {
            return Err(WireError::Epoch);
        }
        Ok((kind, d))
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let bytes = self.remaining.get(..n).ok_or(WireError::Truncated)?;
        self.remaining = &self.remaining[n..];
        Ok(bytes)
    }
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        self.take(N)?.try_into().map_err(|_| WireError::Truncated)
    }
    fn byte(&mut self) -> Result<u8, WireError> {
        Ok(self.fixed::<1>()?[0])
    }
    fn revision(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_be_bytes(self.fixed()?))
    }
    fn text(&mut self, bound: usize) -> Result<&'a str, WireError> {
        let n = usize::from(u16::from_be_bytes(self.fixed()?));
        if n > bound {
            return Err(WireError::Oversize);
        }
        core::str::from_utf8(self.take(n)?).map_err(|_| WireError::Invalid)
    }
    fn payload(&mut self) -> Result<&'a [u8], WireError> {
        let n =
            usize::try_from(u32::from_be_bytes(self.fixed()?)).map_err(|_| WireError::Oversize)?;
        if n > MAX_PAYLOAD_BYTES {
            return Err(WireError::Oversize);
        }
        self.take(n)
    }
    fn tag(&mut self) -> Result<WireTag, WireError> {
        let kind = self.byte()?;
        let offset_ns = i128::from_be_bytes(self.fixed()?);
        let microstep = self.revision()?;
        match (kind, offset_ns, microstep) {
            (0, 0, 0) => Ok(WireTag::Never),
            (1, _, _) => Ok(WireTag::finite(offset_ns, microstep)),
            (2, 0, 0) => Ok(WireTag::Forever),
            _ => Err(WireError::Invalid),
        }
    }
    fn finish(self) -> Result<(), WireError> {
        if self.remaining.is_empty() {
            Ok(())
        } else {
            Err(WireError::Invalid)
        }
    }
}

#[cfg(test)]
mod tests;

mod payload;
pub use payload::{PayloadCodec, PayloadError, PortableValue, PostcardCodec};
