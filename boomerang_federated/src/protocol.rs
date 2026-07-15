use std::fmt;

/// Stable identity of one participant in a federation.
///
/// A federate is a topology vertex with its own protocol connection and logical-time state. This
/// identifier authenticates that connection and addresses `NET`, `LTC`, `TAG`, message source,
/// and message target state. One federate can own many cross-federate endpoints.
///
/// This is distinct from [`EndpointId`], which identifies one routed logical connection between
/// federates rather than either participant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FederateId(String);

impl FederateId {
    /// Create a stable protocol identity from its wire representation.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Return the wire representation of this federate identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for FederateId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for FederateId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for FederateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Stable identity of one serialized cross-federate logical connection.
///
/// An endpoint identifies a specific route, normally derived from its source and target ports. It
/// selects the payload codec, topology edge, and target runtime action used for a message. Several
/// endpoints may share the same source and target [`FederateId`] values.
///
/// This is a logical route identity, not a federate identity, socket address, or transport
/// connection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EndpointId(String);

impl EndpointId {
    /// Create a stable route identity from its wire representation.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Return the wire representation of this endpoint identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for EndpointId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for EndpointId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for EndpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A protocol tag independent of process-local clocks and architecture-sized integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WireTag {
    Never,
    Finite { offset_ns: i128, microstep: u64 },
    Forever,
}

impl WireTag {
    pub const NEVER: Self = Self::Never;
    pub const ZERO: Self = Self::Finite {
        offset_ns: 0,
        microstep: 0,
    };
    pub const FOREVER: Self = Self::Forever;

    pub const fn finite(offset_ns: i128, microstep: u64) -> Self {
        Self::Finite {
            offset_ns,
            microstep,
        }
    }

    pub fn is_finite(self) -> bool {
        matches!(self, Self::Finite { .. })
    }

    pub fn offset_ns(self) -> Option<i128> {
        match self {
            Self::Finite { offset_ns, .. } => Some(offset_ns),
            Self::Never | Self::Forever => None,
        }
    }

    pub fn microstep(self) -> Option<u64> {
        match self {
            Self::Finite { microstep, .. } => Some(microstep),
            Self::Never | Self::Forever => None,
        }
    }

    /// Apply a logical connection delay using Boomerang's delayed-action tag rule.
    ///
    /// A zero delay preserves the source tag. A positive delay advances the offset and resets the
    /// microstep to zero. Sentinel tags remain sentinels.
    pub fn checked_delay(self, delay: WireDelay) -> Option<Self> {
        match self {
            Self::Never => Some(Self::Never),
            Self::Forever => Some(Self::Forever),
            Self::Finite {
                offset_ns,
                microstep,
            } if delay.is_zero() => Some(Self::Finite {
                offset_ns,
                microstep,
            }),
            Self::Finite { offset_ns, .. } => offset_ns
                .checked_add(i128::from(delay.as_nanos()))
                .map(|offset_ns| Self::Finite {
                    offset_ns,
                    microstep: 0,
                }),
        }
    }
}

impl fmt::Display for WireTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireTag::Never => f.write_str("[NEVER]"),
            WireTag::Forever => f.write_str("[FOREVER]"),
            WireTag::Finite {
                offset_ns,
                microstep,
            } => write!(f, "[{offset_ns}ns+{microstep}]"),
        }
    }
}

/// A nonnegative logical delay on a cross-federate edge, represented in nanoseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WireDelay {
    nanos: u64,
}

impl WireDelay {
    pub const ZERO: Self = Self { nanos: 0 };

    pub const fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    pub const fn as_nanos(self) -> u64 {
        self.nanos
    }

    pub const fn is_zero(self) -> bool {
        self.nanos == 0
    }
}

/// A directed cross-federate edge with the minimum logical delay on that endpoint.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TopologyEdge {
    pub source: FederateId,
    pub target: FederateId,
    pub endpoint: EndpointId,
    pub delay: WireDelay,
}

impl TopologyEdge {
    pub fn new(
        source: impl Into<FederateId>,
        target: impl Into<FederateId>,
        endpoint: impl Into<EndpointId>,
        delay: WireDelay,
    ) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            endpoint: endpoint.into(),
            delay,
        }
    }
}

/// The incoming and outgoing neighbor view for one federate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NeighborStructure {
    pub federate_id: FederateId,
    pub upstream: Vec<TopologyEdge>,
    pub downstream: Vec<TopologyEdge>,
}

/// Static topology used by the RTI for TAG/NET/LTC decisions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FederatedTopology {
    pub federates: Vec<FederateId>,
    pub edges: Vec<TopologyEdge>,
}

impl FederatedTopology {
    pub fn new(federates: impl IntoIterator<Item = FederateId>) -> Self {
        Self {
            federates: federates.into_iter().collect(),
            edges: Vec::new(),
        }
    }

    pub fn with_edges(
        federates: impl IntoIterator<Item = FederateId>,
        edges: impl IntoIterator<Item = TopologyEdge>,
    ) -> Self {
        Self {
            federates: federates.into_iter().collect(),
            edges: edges.into_iter().collect(),
        }
    }

    pub fn add_federate(&mut self, federate_id: impl Into<FederateId>) {
        let federate_id = federate_id.into();
        if !self.federates.contains(&federate_id) {
            self.federates.push(federate_id);
        }
    }

    pub fn add_edge(&mut self, edge: TopologyEdge) {
        self.add_federate(edge.source.clone());
        self.add_federate(edge.target.clone());
        self.edges.push(edge);
    }

    pub fn contains_federate(&self, federate_id: &FederateId) -> bool {
        self.federates.contains(federate_id)
    }

    pub fn incoming_edges<'a>(
        &'a self,
        federate_id: &'a FederateId,
    ) -> impl Iterator<Item = &'a TopologyEdge> + 'a {
        self.edges
            .iter()
            .filter(move |edge| &edge.target == federate_id)
    }

    pub fn outgoing_edges<'a>(
        &'a self,
        federate_id: &'a FederateId,
    ) -> impl Iterator<Item = &'a TopologyEdge> + 'a {
        self.edges
            .iter()
            .filter(move |edge| &edge.source == federate_id)
    }

    pub fn neighbors_for(&self, federate_id: &FederateId) -> NeighborStructure {
        let mut neighbors = NeighborStructure {
            federate_id: federate_id.clone(),
            upstream: self.incoming_edges(federate_id).cloned().collect(),
            downstream: self.outgoing_edges(federate_id).cloned().collect(),
        };
        neighbors.upstream.sort();
        neighbors.downstream.sort();
        neighbors
    }
}

/// Messages sent from a federate client to the centralized RTI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FederateToRti {
    Hello {
        federate_id: FederateId,
        topology: NeighborStructure,
    },
    Net {
        federate_id: FederateId,
        tag: WireTag,
    },
    Ltc {
        federate_id: FederateId,
        tag: WireTag,
    },
    Msg {
        source: FederateId,
        target: FederateId,
        endpoint: EndpointId,
        tag: WireTag,
        payload: Vec<u8>,
    },
    Stop {
        federate_id: FederateId,
    },
}

/// Messages sent from the centralized RTI to a federate client.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RtiToFederate {
    Start {
        start_unix_epoch_ns: i128,
    },
    Tag {
        tag: WireTag,
    },
    Msg {
        source: FederateId,
        endpoint: EndpointId,
        tag: WireTag,
        payload: Vec<u8>,
    },
    Stop,
    Error {
        message: String,
    },
}

/// A serde-friendly frame wrapper. Codec implementations can replace this framing later.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ProtocolFrame {
    FederateToRti(FederateToRti),
    RtiToFederate(RtiToFederate),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_tags_order_sentinels_and_finite_tags() {
        assert!(WireTag::Never < WireTag::ZERO);
        assert!(WireTag::ZERO < WireTag::finite(0, 1));
        assert!(WireTag::finite(10, 0) < WireTag::Forever);
    }

    #[test]
    fn wire_tag_delay_preserves_zero_delay_and_resets_positive_delay_microstep() {
        let tag = WireTag::finite(5, 3);

        assert_eq!(tag.checked_delay(WireDelay::ZERO), Some(tag));
        assert_eq!(
            tag.checked_delay(WireDelay::from_nanos(10)),
            Some(WireTag::finite(15, 0))
        );
        assert_eq!(
            WireTag::Forever.checked_delay(WireDelay::from_nanos(10)),
            Some(WireTag::Forever)
        );
    }

    #[cfg(feature = "serde-json-codec")]
    #[test]
    fn wire_tags_round_trip_through_serde_json() {
        for tag in [
            WireTag::Never,
            WireTag::ZERO,
            WireTag::finite(42, 7),
            WireTag::Forever,
        ] {
            let encoded = serde_json::to_vec(&tag).unwrap();
            let decoded: WireTag = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(decoded, tag);
        }
    }
}
