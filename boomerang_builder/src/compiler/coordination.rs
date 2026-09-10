//! Backend projections derived from canonical federation analysis.
//!
//! The analyzed federation remains authoritative for logical-time and lifecycle
//! semantics. This module contains only mechanical projections of those facts into
//! backend-specific immutable tables. Projection converts stable identities to dense
//! typed keys once; it must not perform reachability, SCC, shortest-path, or other
//! graph analysis.
//! Physical boundary IDs are metadata in this slice; cross-Federate physical
//! connection semantics remain reserved for later physical-time slices.

use std::collections::{BTreeMap, BTreeSet};

use super::federation::AnalyzedFederationGraph;
use super::identity::canonical_identity_text;
use super::{
    BoundaryId, CodecCapabilityId, FederateId, FlowId, PhysicalBoundaryId, ResolvedDeployment,
    TransportCapabilityId,
};
use crate::runtime::image::{
    self as runtime_image, CodecCapabilityIndex, CoordinationProjection, FederateIndex, FlowIndex,
    PhysicalBoundaryIndex, RtiDependencyImage, RtiImage, RtiMemberImage, RtiRouteImage,
    RtiRouteIndex, TransportCapabilityIndex,
};
use tinymap::{SliceRange, TinyMap};

/// Failure to represent an analyzed federation in bounded image coordinates.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CoordinationProjectionError {
    /// A generated table has more entries than its `u32` image range can address.
    #[error("coordination table '{table}' exceeds u32 image capacity")]
    TableTooLarge {
        /// Name of the unrepresentable generated table.
        table: &'static str,
    },
}

/// Heap-backed owner of the dense tables borrowed by [`RtiImage`].
///
/// This is compiler-side storage, not a second semantic model. Generated launchers
/// render the equivalent immutable slices directly into the RTI binary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedRtiImage {
    /// Per-Federate coordination ranges in canonical dense-key order.
    members: TinyMap<FederateIndex, RtiMemberImage>,
    /// Packed range backing storage for members' direct and transitive dependencies.
    ///
    /// Entries have no independent identity: [`RtiMemberImage`] owns each relationship set through
    /// its typed ranges into this slice.
    dependencies: Box<[RtiDependencyImage]>,
    /// Packed range backing storage for members' affected-downstream Federate keys.
    ///
    /// Each value is already a typed [`FederateIndex`]; [`RtiMemberImage`] owns each set through
    /// its affected-downstream range rather than through a synthetic per-entry key.
    affected_downstream: Box<[FederateIndex]>,
    /// Complete deployment-wide RTI route hops keyed independently of local scheduler route halves.
    routes: TinyMap<RtiRouteIndex, OwnedRtiRouteImage>,
    /// Distinct end-to-end flow identities shared by one or more routes.
    flows: TinyMap<FlowIndex, FlowId>,
    /// Canonically ordered stable physical input/output identities.
    physical_boundaries: TinyMap<PhysicalBoundaryIndex, PhysicalBoundaryId>,
    /// Canonically ordered transport capability identities.
    transport_capabilities: TinyMap<TransportCapabilityIndex, TransportCapabilityId>,
    /// Canonically ordered codec capability identities.
    codec_capabilities: TinyMap<CodecCapabilityIndex, CodecCapabilityId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedRtiRouteImage {
    boundary: BoundaryId,
    flow: FlowIndex,
    physical_input: Option<PhysicalBoundaryIndex>,
    physical_output: Option<PhysicalBoundaryIndex>,
    failure_policy: crate::runtime::image::BoundaryFailurePolicy,
    transport_policy: crate::runtime::image::TransportPolicy,
    codec_policy: crate::runtime::image::CodecPolicy,
    timing_policy: crate::runtime::image::TimingPolicy,
    security_policy: crate::runtime::image::SecurityPolicy,
    transport_capability: TransportCapabilityIndex,
    codec_capability: CodecCapabilityIndex,
    source: FederateIndex,
    target: FederateIndex,
    delay_nanos: u64,
}

impl OwnedRtiRouteImage {
    fn image<'a>(&self, boundary: &'a str) -> RtiRouteImage<'a> {
        RtiRouteImage::new(
            runtime_image::BoundaryId::new(boundary),
            self.flow,
            self.physical_input,
            self.physical_output,
            self.failure_policy,
            self.transport_policy,
            self.codec_policy,
            self.timing_policy,
            self.security_policy,
            self.transport_capability,
            self.codec_capability,
            self.source,
            self.target,
            self.delay_nanos,
        )
    }
}

fn identity_text<I: std::fmt::Display, K: tinymap::Key>(values: &TinyMap<K, I>) -> Vec<String> {
    values.values().map(canonical_identity_text).collect()
}

fn borrowed_identities<K: tinymap::Key>(values: &[String]) -> TinyMap<K, &str> {
    values.iter().map(String::as_str).collect()
}

impl OwnedRtiImage {
    /// Borrows this owner as the target-facing immutable RTI image.
    #[must_use]
    pub fn with_image<T>(&self, f: impl FnOnce(RtiImage<'_>) -> T) -> T {
        let route_text = self
            .routes
            .values()
            .map(|route| route.boundary.to_canonical_string())
            .collect::<Vec<_>>();
        let routes = self
            .routes
            .values()
            .zip(&route_text)
            .map(|(route, boundary)| route.image(boundary))
            .collect::<TinyMap<RtiRouteIndex, _>>();
        let flow_text = identity_text(&self.flows);
        let physical_text = identity_text(&self.physical_boundaries);
        let transport_text = identity_text(&self.transport_capabilities);
        let codec_text = identity_text(&self.codec_capabilities);
        let flows = borrowed_identities::<FlowIndex>(&flow_text);
        let physical = borrowed_identities::<PhysicalBoundaryIndex>(&physical_text);
        let transports = borrowed_identities::<TransportCapabilityIndex>(&transport_text);
        let codecs = borrowed_identities::<CodecCapabilityIndex>(&codec_text);
        f(RtiImage::new(
            self.members.as_view(),
            &self.dependencies,
            &self.affected_downstream,
            routes.as_view(),
            flows.as_view(),
            physical.as_view(),
            transports.as_view(),
            codecs.as_view(),
        ))
    }
}

/// Heap-backed coordination projection selected for one compiled deployment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnedCoordinationProjection {
    /// A one-Federate deployment needs no distributed coordinator.
    Local,
    /// A multi-Federate central backend embeds one immutable RTI image.
    CentralRti(Box<OwnedRtiImage>),
}

impl OwnedCoordinationProjection {
    /// Borrows the selected projection in the target-facing representation.
    #[must_use]
    pub fn with_image<T>(&self, f: impl FnOnce(CoordinationProjection<'_>) -> T) -> T {
        match self {
            Self::Local => f(CoordinationProjection::Local),
            Self::CentralRti(image) => {
                image.with_image(|image| f(CoordinationProjection::CentralRti(image)))
            }
        }
    }
}

/// Mechanically maps analyzed stable identities into central-RTI dense tables.
pub(crate) fn project_central_rti(
    analysis: &AnalyzedFederationGraph,
    deployment: &ResolvedDeployment,
) -> Result<OwnedRtiImage, CoordinationProjectionError> {
    let indices = analysis
        .members()
        .iter()
        .enumerate()
        .map(|(index, member)| Ok((member, FederateIndex::new(checked_len("members", index)?))))
        .collect::<Result<BTreeMap<_, _>, CoordinationProjectionError>>()?;
    checked_len("members", analysis.members().len())?;

    let mut members = Vec::with_capacity(analysis.members().len());
    let mut dependencies = Vec::new();
    let mut affected_downstream = Vec::new();
    macro_rules! dense {
        ($key:ty, $values:expr) => {
            dense_identities::<_, $key>(($values).collect())?
        };
    }
    let route_bindings = analysis
        .edges()
        .iter()
        .map(|edge| {
            deployment
                .boundary_binding(edge.id())
                .expect("resolved cross-Federate boundary has one binding")
        })
        .collect::<Vec<_>>();
    let (flows, flow_indices) = dense!(FlowIndex, route_bindings.iter().map(|value| value.flow()));
    let physical = route_bindings.iter().flat_map(|value| {
        [value.physical().input(), value.physical().output()]
            .into_iter()
            .flatten()
    });
    let (physical_boundaries, physical_indices) = dense!(PhysicalBoundaryIndex, physical);
    let (transport_capabilities, transport_capability_indices) = dense!(
        TransportCapabilityIndex,
        route_bindings.iter().map(|value| value.transport())
    );
    let (codec_capabilities, codec_capability_indices) = dense!(
        CodecCapabilityIndex,
        route_bindings.iter().map(|value| value.codec())
    );
    let routes = analysis
        .edges()
        .iter()
        .zip(route_bindings)
        .map(|(edge, binding)| {
            Ok(OwnedRtiRouteImage {
                boundary: edge.id().clone(),
                flow: flow_indices[binding.flow()],
                physical_input: binding.physical().input().map(|id| physical_indices[id]),
                physical_output: binding.physical().output().map(|id| physical_indices[id]),
                failure_policy: binding.policies().failure(),
                transport_policy: binding.policies().transport(),
                codec_policy: binding.policies().codec(),
                timing_policy: binding.policies().timing(),
                security_policy: binding.policies().security(),
                transport_capability: transport_capability_indices[binding.transport()],
                codec_capability: codec_capability_indices[binding.codec()],
                source: indices[edge.source()],
                target: indices[edge.target()],
                delay_nanos: edge.delay().as_nanos(),
            })
        })
        .collect::<Result<TinyMap<RtiRouteIndex, _>, CoordinationProjectionError>>()?;
    for member in analysis.members() {
        let direct = append_dependencies(
            &mut dependencies,
            analysis.direct_incoming(member).unwrap_or_default(),
            &indices,
        )?;
        let transitive = append_dependencies(
            &mut dependencies,
            analysis.transitive_incoming(member).unwrap_or_default(),
            &indices,
        )?;
        let downstream_start = checked_len("affected_downstream", affected_downstream.len())?;
        affected_downstream.extend(
            analysis
                .affected_downstream(member)
                .unwrap_or_default()
                .iter()
                .map(|target| indices[target]),
        );
        let downstream_len = checked_len(
            "affected_downstream",
            affected_downstream.len() - downstream_start as usize,
        )?;
        members.push(RtiMemberImage::new(
            deployment
                .federate(member)
                .expect("analyzed member has a resolved Federate configuration")
                .recovery(),
            direct,
            transitive,
            SliceRange::new(downstream_start, downstream_len),
        ));
    }

    Ok(OwnedRtiImage {
        members: members.into_iter().collect(),
        dependencies: dependencies.into_boxed_slice(),
        affected_downstream: affected_downstream.into_boxed_slice(),
        routes,
        flows,
        physical_boundaries,
        transport_capabilities,
        codec_capabilities,
    })
}

/// Converts a canonical stable-identity set into a dense table and lookup map.
type DenseIdentities<'a, I, K> = (TinyMap<K, I>, BTreeMap<&'a I, K>);

fn dense_identities<'a, I, K>(
    identities: BTreeSet<&'a I>,
) -> Result<DenseIdentities<'a, I, K>, CoordinationProjectionError>
where
    I: Clone + Ord + std::fmt::Display,
    K: tinymap::Key,
{
    let mut values = Vec::with_capacity(identities.len());
    let mut indices = BTreeMap::new();
    let mut identities = identities.into_iter().collect::<Vec<_>>();
    identities.sort_by_cached_key(|identity| canonical_identity_text(*identity));
    for (position, identity) in identities.into_iter().enumerate() {
        values.push(identity.clone());
        indices.insert(
            identity,
            K::from(usize::try_from(checked_len("identities", position)?).expect("u32 fits usize")),
        );
    }
    Ok((values.into_iter().collect(), indices))
}

/// Appends one member's dependencies and returns their flattened range.
fn append_dependencies(
    target: &mut Vec<RtiDependencyImage>,
    values: &[(FederateId, super::federation::FederationDelay)],
    indices: &BTreeMap<&FederateId, FederateIndex>,
) -> Result<SliceRange<RtiDependencyImage>, CoordinationProjectionError> {
    let start = checked_len("dependencies", target.len())?;
    target.extend(
        values
            .iter()
            .map(|(source, delay)| RtiDependencyImage::new(indices[source], delay.as_nanos())),
    );
    let len = checked_len("dependencies", target.len() - start as usize)?;
    Ok(SliceRange::new(start, len))
}

/// Converts one generated table length to its target image representation.
fn checked_len(table: &'static str, len: usize) -> Result<u32, CoordinationProjectionError> {
    u32::try_from(len).map_err(|_| CoordinationProjectionError::TableTooLarge { table })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_tiny_map<K: tinymap::Key, V>(_: &tinymap::TinyMap<K, V>) {}

    #[test]
    fn owned_rti_image_keeps_every_dense_domain_typed() {
        fn assert_field_types(image: &OwnedRtiImage) {
            assert_tiny_map::<FederateIndex, RtiMemberImage>(&image.members);
            assert_tiny_map::<RtiRouteIndex, OwnedRtiRouteImage>(&image.routes);
            assert_tiny_map::<FlowIndex, FlowId>(&image.flows);
            assert_tiny_map::<PhysicalBoundaryIndex, PhysicalBoundaryId>(
                &image.physical_boundaries,
            );
            assert_tiny_map::<TransportCapabilityIndex, TransportCapabilityId>(
                &image.transport_capabilities,
            );
            assert_tiny_map::<CodecCapabilityIndex, CodecCapabilityId>(&image.codec_capabilities);
        }

        let _ = assert_field_types;
    }
}
