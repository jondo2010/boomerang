use std::{fmt::Display, time::Duration};

use serde::{Deserialize, Serialize};

mod bincodec;
mod client;
mod clock;
mod rti;
#[cfg(test)]
mod tests;

/// Timestamps are represented as the duration since the UNIX epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[repr(transparent)]
pub struct Timestamp(Duration);

impl Timestamp {
    pub fn now() -> Self {
        Self(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("System time before UNIX epoch"),
        )
    }

    pub fn offset(&self, offset: Duration) -> Self {
        Self(self.0 + offset)
    }
}

impl From<Duration> for Timestamp {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[repr(transparent)]
pub struct FederateId(usize);

impl tinymap::Key for FederateId {
    fn index(&self) -> usize {
        self.0
    }
}

impl From<usize> for FederateId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl Display for FederateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FederateId({})", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    /// Offset from origin of logical time
    pub timestamp: Timestamp,
    /// Superdense timestep.
    pub microstep: u32,
}

/// A timestamped message to forward to another federate.
///
/// With centralized coordination, all such messages flow through the RTI.
/// With decentralized coordination, tagged messages are sent peer-to-peer between federates and are marked with
/// MSG_TYPE_P2P_TAGGED_MESSAGE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The ID of the destination reactor port.
    pub dest_reactor_port_id: u16,
    /// The destination federate ID.
    pub dest_federate_id: FederateId,
    pub message: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortAbsent {
    pub port_id: u16,
    /// Federate ID of the destination federate.
    /// This is needed for the centralized coordination so that the RTI knows where to forward the message.
    pub federate_id: FederateId,
    /// Intended time of the absent message
    pub tag: Tag,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Reject(#[from] RejectReason),

    #[error("Connection unexpectedly closed")]
    HangUp,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

//// Rejection codes
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum RejectReason {
    /// Federation ID does not match.
    #[error("Federation ID does not match")]
    FederationIdDoesNotMatch = 1,
    /// Federate with the specified ID has already joined.
    #[error("Federate ID in use")]
    FederateIdInUse,
    /// Federate ID out of range.
    #[error("Federate ID out of range")]
    FederateIdOutOfRange,
    /// Incoming message is not expected.
    #[error("Unexpected message")]
    UnexpectedMessage,
    /// Connected to the wrong server.
    #[error("Connected to the wrong server")]
    WrongServer,
    /// HMAC authentication failed.
    #[error("HMAC authentication failed")]
    HmacDoesNotMatch,
}

/// Each federate needs to have a unique ID between 0 and NUMBER_OF_FEDERATES-1.
///
/// Each federate, when starting up, should send this message to the RTI. This is its first message
/// to the RTI.
///
/// The RTI will respond with either `Reject`, `Ack`, or `UdpPort`.
///
/// If the federate is a C target LF program, the generated federate code does this by calling
/// synchronize_with_other_federates(), passing to it its federate ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FedIds {
    /// Federate ID.
    pub federate_id: FederateId,
    /// Federation ID
    pub federation_id: String,
}

/// A message that informs the RTI about connections between this federate and other federates where
/// messages are routed through the RTI. Currently, this only includes logical connections when the
/// coordination is centralized. This information is needed for the RTI to perform the centralized
/// coordination.
///
/// Note: Only information about the immediate neighbors is required. The RTI can transitively
/// obtain the structure of the federation based on each federate's immediate neighbor information.
///
/// Note: The upstream and downstream connections are transmitted on the same message to prevent
/// (at least to some degree) the scenario where the RTI has information about one, but not the
/// other (which is a critical error).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeighborStructure {
    /// Federate's connection to upstream federates (by direct connection).
    ///
    /// The delay is the minimum "after" delay of all connections from the upstream federate.
    pub upstream: Vec<(FederateId, Duration)>,
    /// Federate's downstream federates (by direct connection).
    pub downstream: Vec<FederateId>,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum ClockSyncStat {
    /// No synchronization should be performed at all.
    Off,
    /// Only the initial clock synchronization is enabled.
    Init,
    /// The port number for the UDP server
    On(u16),
}

impl ClockSyncStat {
    /// Returns `true` if the clock sync stat is [`Init`] or [`On`].
    #[must_use]
    pub fn is_on(&self) -> bool {
        matches!(self, Self::Init | Self::On(_))
    }

    /// Returns `true` if the clock sync stat is [`Off`].
    ///
    /// [`Off`]: ClockSyncStat::Off
    #[must_use]
    pub fn is_off(&self) -> bool {
        matches!(self, Self::Off)
    }
}

/// The HMAC tag is composed of the following order:
/// * One byte equal to MSG_TYPE_FED_RESPONSE.
/// * Two bytes (ushort) giving the federate ID.
/// * Eight bytes for received RTI's nonce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hmac {}

/// A message from federate to RTI as a response to the RTI Hello message. The
/// federate sends this message to RTI for HMAC-based authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FedResponse {
    /// Federate's nonce
    pub nonce: u64,
    /// Federate ID
    pub federate_id: FederateId,
    /// HMAC tag
    pub hmac: Hmac,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RtiMsg {
    /// A rejection of the previously received message.
    Reject(RejectReason),

    /// Acknowledgment of the previously received message. This message carries no payload.
    Ack,

    /// Acknowledgment of the previously received `FedIds` message sent by the RTI to the federate with a payload indicating the UDP port to use for clock synchronization.
    UdpPort(ClockSyncStat),

    /// A message from a federate to an RTI containing the federation ID and the federate ID.
    FedIds(FedIds),

    /// Byte identifying a message from an RTI to a federate containing RTI's 8-byte random nonce
    /// for HMAC-based authentication. The RTI sends this message to an incoming federate when TCP
    /// connection is established between the RTI and the federate.
    ///
    /// The next eight bytes are RTI's 8-byte nonce (RTI nonce).
    RtiNonce,

    /// A message from federate to RTI as a response to the RTI Hello message.
    FedResponse(FedResponse),

    /// Byte identifying a message from RTI to a federate as a response to the FED_RESPONSE message.
    ///
    /// The RTI sends this message to federate for HMAC-based authentication.
    ///
    /// The message contains, in this order:
    /// * One byte equal to MSG_TYPE_RTI_RESPONSE.
    /// * 32 bytes for HMAC tag based on SHA256.
    /// The HMAC tag is composed of the following order:
    /// * One byte equal to MSG_TYPE_RTI_RESPONSE.
    /// * Eight bytes for received federate's nonce.
    RtiResponse,

    /// Byte identifying a timestamp message, which is 64 bits long.
    ///
    /// Each federate sends its starting physical time as a message of this type, and the RTI
    /// broadcasts to all the federates the starting logical time as a message of this type.
    Timestamp(Timestamp),

    /// A message to forward to another federate.
    ///
    /// NOTE: This is currently not used. All messages are tagged, even on physical connections,
    /// because if "after" is used, the message may preserve the logical timestamp rather than
    /// using the physical time.
    Message(Message),

    /// The federate is ending its execution.
    Resign,

    TaggedMessage(Tag, Message),

    /// A next event tag (NET) message sent from a federate in centralized coordination.
    ///
    /// This message from a federate tells the RTI the tag of the earliest event on that federate's
    /// event queue. In other words, absent any further inputs from other federates, this will be
    /// the least tag of the next set of reactions on that federate. If the event queue is empty and
    /// a timeout time has been specified, then the timeout time will be sent. If there is no
    /// timeout time, then FOREVER will be sent. Note that if there are physical actions and the
    /// earliest event on the event queue has a tag that is ahead of physical time (or the queue is
    /// empty), the federate should try to regularly advance its tag (and thus send NET messages) to
    /// make sure downstream federates can make progress.
    NextEventTag(Tag),

    /// A time advance grant (TAG) sent by the RTI to a federate in centralized coordination.
    ///
    /// This message is a promise by the RTI to the federate that no later message sent to the federate
    /// will have a tag earlier than or equal to the tag carried by this TAG message.
    TagAdvanceGrant(Tag),

    /// A provisional time advance grant (PTAG) sent by the RTI to a federate in centralized coordination.
    ///
    /// This message is a promise by the RTI to the federate that no later message sent to the federate
    /// will have a tag earlier than or equal to the tag carried by this TAG message.
    ProvisionalTagAdvanceGrant(Tag),

    /// A logical tag complete (LTC) message sent by a federate to the RTI.
    LogicalTagComplete(Tag),

    /// A stop request. This message is first sent to the RTI by a federate that would like to stop
    /// execution at the specified tag. The RTI will forward the `StopRequest` to all other
    /// federates. Those federates will either agree to the requested tag or propose a larger tag.
    /// The RTI will collect all proposed tags and broadcast the largest of those to all federates.
    /// All federates will then be expected to stop at the granted tag.
    ///
    /// NOTE: The RTI may reply with a larger tag than the one specified in this message.
    /// It has to be that way because if any federate can send a StopRequest` message that specifies
    /// the stop time on all other federates, then every federate depends on every other federate
    /// and time cannot be advanced.  Hence, the actual stop time may be nondeterministic.
    /// If, on the other hand, the federate requesting the stop is upstream of every other federate,
    /// then it should be possible to respect its requested stop tag.
    StopRequest(Tag),

    /// A federate's reply to a `StopRequest` that was sent by the RTI. The payload is a proposed
    /// stop tag that is at least as large as the one sent to the federate in a `StopRequest`
    /// message.
    StopRequestReply(Tag),

    /// Sent by the RTI indicating that the stop request from some federate has been granted.
    /// The payload is the tag at which all federates have agreed that they can stop.
    StopGranted(Tag),

    /// An address query message, sent by a federate to RTI to ask for another federate's address
    /// and port number.
    ///
    /// The reply from the RTI will a port number (an int32_t), which is `None` if the RTI does not
    /// know yet (it has not received `AddressAdvertisement` from the other federate), followed by
    /// the IP address of the other federate (an IPV4 address, which has length INET_ADDRSTRLEN).
    AddressQuery(FederateId),

    /// A message advertising the port for the TCP connection server of a federate. This is utilized
    /// in decentralized coordination as well as for physical connections in centralized
    /// coordination.
    ///
    /// * The next four bytes (or sizeof(int32_t)) will be the port number.
    /// The sending federate will not wait for a response from the RTI and assumes its request will
    /// be processed eventually by the RTI.
    AddressAdvertisement,

    /// A first message that is sent by a federate directly to another federate after establishing a socket connection to send messages directly to the federate.
    /// The response from the remote federate is expected to be `Ack`, but if the remote federate does not expect this federate or federation to connect, it will respond instead with `Reject`.
    P2PSendingFedId(FedIds),

    /// A message to send directly to another federate.
    P2PMessage(Message),

    /// A timestamped message to send directly to another federate.
    ///
    /// This is a variant of [`TaggedMessage`] that is used in P2P connections between federates.
    /// Having a separate message type for P2P connections between federates will be useful in
    /// preventing crosstalk.
    P2PTaggedMessage(Tag, Message),

    /// Byte identifying a message that a downstream federate sends to its upstream counterpart to
    /// request that the socket connection be closed.
    ///
    /// This is the only message that should flow upstream on such socket connections.
    CloseRequest,

    /// A timestamp sent according to PTP.
    /// T1 is the first message in a PTP exchange.
    ClockSyncT1,

    /// Prompts the master to send a T4.
    ClockSyncT3,

    /// A timestamp sent according to PTP.
    ClockSyncT4,

    /// Coded probe message.
    ///
    /// This message is sent by the server (master) right after [`ClockSyncT4`] (t1) with a
    /// new physical clock snapshot t2.
    ///
    /// At the receiver, the previous [`ClockSyncT4`] message and this message are assigned a
    /// receive timestamp r1 and r2. If |(r2 - r1) - (t2 - t1)| < GUARD_BAND, then the current
    /// clock sync cycle is considered pure and can be processed.
    ///
    /// @see Geng, Yilong, et al.  "Exploiting a natural network effect for scalable, fine-grained
    /// clock synchronization."
    ClockSyncCodedProbe,

    /// A port absent message, informing the receiver that a given port will not have event for the current logical time.
    PortAbsent(PortAbsent),

    /// A message that informs the RTI about connections between this federate and other federates.
    NeighborStructure(NeighborStructure),
}
