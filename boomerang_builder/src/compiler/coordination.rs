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
use super::{FederateId, ResolvedDeployment};
use crate::runtime::image::{
    CodecCapabilityIndex, CoordinationProjection, FederateIndex, FlowIndex, IdentityRange,
    IdentityTable, PhysicalBoundaryIndex, RtiDependencyImage, RtiImage, RtiMemberImage,
    RtiRouteImage, RtiRouteIndex, TransportCapabilityIndex,
};
use tinymap::{TableRange, TinyMap};

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
    /// Concatenated stable identities referenced by RTI records.
    identity_data: Box<str>,
    /// Per-Federate coordination ranges in canonical dense-key order.
    members: TinyMap<FederateIndex, RtiMemberImage>,
    /// Flattened direct and transitive dependency records.
    dependencies: Box<[RtiDependencyImage]>,
    /// Flattened affected-downstream Federate keys.
    affected_downstream: Box<[FederateIndex]>,
    /// Complete deployment-wide RTI route hops keyed independently of local scheduler route halves.
    routes: TinyMap<RtiRouteIndex, RtiRouteImage>,
    /// Distinct end-to-end flow identities shared by one or more routes.
    flows: TinyMap<FlowIndex, IdentityRange>,
    /// Canonically ordered stable physical input/output identities.
    physical_boundaries: TinyMap<PhysicalBoundaryIndex, IdentityRange>,
    /// Canonically ordered transport capability identities.
    transport_capabilities: TinyMap<TransportCapabilityIndex, IdentityRange>,
    /// Canonically ordered codec capability identities.
    codec_capabilities: TinyMap<CodecCapabilityIndex, IdentityRange>,
}

impl OwnedRtiImage {
    /// Borrows this owner as the target-facing immutable RTI image.
    #[must_use]
    pub fn image(&self) -> RtiImage<'_> {
        RtiImage::new(
            &self.identity_data,
            self.members.as_view(),
            &self.dependencies,
            &self.affected_downstream,
            self.routes.as_view(),
            IdentityTable::new(&self.identity_data, self.flows.as_view()),
            IdentityTable::new(&self.identity_data, self.physical_boundaries.as_view()),
            IdentityTable::new(&self.identity_data, self.transport_capabilities.as_view()),
            IdentityTable::new(&self.identity_data, self.codec_capabilities.as_view()),
        )
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
    pub fn image(&self) -> CoordinationProjection<'_> {
        match self {
            Self::Local => CoordinationProjection::Local,
            Self::CentralRti(image) => CoordinationProjection::CentralRti(image.image()),
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
    let mut identity_data = String::new();
    macro_rules! dense {
        ($key:ty, $values:expr) => {
            dense_identities::<_, $key>(&mut identity_data, ($values).collect())?
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
            let boundary = append_identity(&mut identity_data, &edge.id().to_string())?;
            Ok(RtiRouteImage::new(
                boundary,
                flow_indices[binding.flow()],
                binding.physical().input().map(|id| physical_indices[id]),
                binding.physical().output().map(|id| physical_indices[id]),
                binding.policies().failure(),
                binding.policies().transport(),
                binding.policies().codec(),
                binding.policies().timing(),
                binding.policies().security(),
                transport_capability_indices[binding.transport()],
                codec_capability_indices[binding.codec()],
                indices[edge.source()],
                indices[edge.target()],
                edge.delay().as_nanos(),
            ))
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
            TableRange::new(downstream_start, downstream_len),
        ));
    }

    Ok(OwnedRtiImage {
        identity_data: identity_data.into_boxed_str(),
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
type DenseIdentities<'a, I, K> = (TinyMap<K, IdentityRange>, BTreeMap<&'a I, K>);

fn dense_identities<'a, I, K>(
    identity_data: &mut String,
    identities: BTreeSet<&'a I>,
) -> Result<DenseIdentities<'a, I, K>, CoordinationProjectionError>
where
    I: Ord + std::fmt::Display,
    K: tinymap::Key,
{
    let mut values = Vec::with_capacity(identities.len());
    let mut indices = BTreeMap::new();
    let mut identities = identities.into_iter().collect::<Vec<_>>();
    identities.sort_by_cached_key(|identity| canonical_identity_text(*identity));
    for (position, identity) in identities.into_iter().enumerate() {
        values.push(append_identity(identity_data, &identity.to_string())?);
        indices.insert(
            identity,
            K::from(usize::try_from(checked_len("identities", position)?).expect("u32 fits usize")),
        );
    }
    Ok((values.into_iter().collect(), indices))
}

/// Appends one stable identity and returns its checked byte range.
fn append_identity(
    target: &mut String,
    value: &str,
) -> Result<IdentityRange, CoordinationProjectionError> {
    let start = checked_len("identity_data", target.len())?;
    let len = checked_len("identity_data", value.len())?;
    target.push_str(value);
    Ok(IdentityRange::new(start, len))
}

/// Appends one member's dependencies and returns their flattened range.
fn append_dependencies(
    target: &mut Vec<RtiDependencyImage>,
    values: &[(FederateId, super::federation::FederationDelay)],
    indices: &BTreeMap<&FederateId, FederateIndex>,
) -> Result<TableRange<RtiDependencyImage>, CoordinationProjectionError> {
    let start = checked_len("dependencies", target.len())?;
    target.extend(
        values
            .iter()
            .map(|(source, delay)| RtiDependencyImage::new(indices[source], delay.as_nanos())),
    );
    let len = checked_len("dependencies", target.len() - start as usize)?;
    Ok(TableRange::new(start, len))
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
            assert_tiny_map::<RtiRouteIndex, RtiRouteImage>(&image.routes);
            assert_tiny_map::<FlowIndex, IdentityRange>(&image.flows);
            assert_tiny_map::<PhysicalBoundaryIndex, IdentityRange>(&image.physical_boundaries);
            assert_tiny_map::<TransportCapabilityIndex, IdentityRange>(
                &image.transport_capabilities,
            );
            assert_tiny_map::<CodecCapabilityIndex, IdentityRange>(&image.codec_capabilities);
        }

        let _ = assert_field_types;
    }
}
