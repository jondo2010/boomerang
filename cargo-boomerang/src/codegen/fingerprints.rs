//! Compiler-side inputs for channel compatibility and local image identity.
//!
//! Existing descriptor fingerprints establish descriptor/payload ABI compatibility;
//! bundle fingerprints identify reproducible deployment inputs; artifact digests identify
//! exact executable bytes. None can admit heterogeneous Federates: changing one private
//! image or binary must not invalidate otherwise compatible channel peers.
//!
//! This module reuses BLAKE3 with separate versioned domains for the dense mapping,
//! shared coordination semantics, and each local scheduler image. It reuses existing
//! descriptor fingerprints and the canonical image renderer as inputs. Only shared
//! coordination and mapping digests participate in channel admission. Declared contract
//! IDs and versions own payload schemas; local image and artifact claims may differ.

use super::{binding_implementation, rust};
use anyhow::{Context, Result};
use boomerang_builder::{
    compiler::{ApplicationTopology, FederateSlice, OwnedCompiledDeployment},
    host_interchange::DescriptorDriverBinding,
};
use boomerang_runtime::image::{CoordinationProjection, RtiDependencyImage};

/// Hashes an explicitly ordered semantic record with an independent versioned domain.
fn hash(domain: &str, input: &impl serde::Serialize) -> Result<blake3::Hash> {
    let bytes = serde_json::to_vec(input).context("failed to encode canonical identity input")?;
    let mut hasher = blake3::Hasher::new_derive_key(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize())
}

/// Identifies the exact dense identity tables and route-to-member assignments.
pub(super) fn mapping(compiled: &OwnedCompiledDeployment) -> Result<blake3::Hash> {
    compiled.with_coordination(|projection| {
        let CoordinationProjection::CentralRti(image) = projection else {
            anyhow::bail!("wire mapping requires central-rti")
        };
        let members = compiled
            .federates()
            .values()
            .map(|f| f.id().as_str())
            .collect::<Vec<_>>();
        let routes = image
            .routes()
            .iter()
            .map(|(key, r)| {
                (
                    image.route_boundary(key).as_str(),
                    r.source().as_u32(),
                    r.target().as_u32(),
                    r.flow().as_u32(),
                    r.physical_input().map(|k| k.as_u32()),
                    r.physical_output().map(|k| k.as_u32()),
                    r.transport_capability().as_u32(),
                    r.codec_capability().as_u32(),
                )
            })
            .collect::<Vec<_>>();
        hash(
            "boomerang.wire-mapping.v1",
            &(
                members,
                routes,
                image.flows().values().collect::<Vec<_>>(),
                image.physical_boundaries().values().collect::<Vec<_>>(),
                image.transport_capabilities().values().collect::<Vec<_>>(),
                image.codec_capabilities().values().collect::<Vec<_>>(),
            ),
        )
    })
}

/// Hashes shared boundary contracts and the existing analyzed coordination projection.
/// Contract identity/version is the schema authority: a payload schema change must version it.
pub(super) fn coordination(
    compiled: &OwnedCompiledDeployment,
    topology: &ApplicationTopology,
) -> Result<blake3::Hash> {
    compiled.with_coordination(|projection| {
        let CoordinationProjection::CentralRti(image) = projection else {
            anyhow::bail!("wire coordination requires central-rti")
        };
        let mut contracts = Vec::new();
        for edge in compiled.federation().edges() {
            let connection = topology
                .connection(edge.id())
                .context("compiled boundary missing from canonical topology")?;
            for endpoint in [connection.source(), connection.target()] {
                let port = topology.port(endpoint).context("missing boundary port")?;
                let reactor = topology
                    .reactor(port.reactor())
                    .context("missing boundary reactor")?;
                let component = topology
                    .component(reactor.component())
                    .context("missing boundary component")?;
                contracts.push((
                    edge.id(),
                    endpoint.to_string(),
                    component.id().to_string(),
                    component.contract().as_str(),
                    component.contract_version(),
                ));
            }
        }
        let ranges = |range: tinymap::SliceRange<RtiDependencyImage>| (range.start(), range.len());
        let members = image
            .members()
            .iter()
            .map(|(key, m)| {
                (
                    image.member_recovery_policy(key),
                    ranges(m.direct_incoming_range()),
                    ranges(m.transitive_incoming_range()),
                    (
                        m.affected_downstream_range().start(),
                        m.affected_downstream_range().len(),
                    ),
                )
            })
            .collect::<Vec<_>>();
        let dependencies = image
            .dependencies()
            .iter()
            .map(|d| (d.source().as_u32(), d.delay_nanos()))
            .collect::<Vec<_>>();
        let downstream = image
            .affected_downstream_entries()
            .iter()
            .map(|k| k.as_u32())
            .collect::<Vec<_>>();
        let routes = image
            .routes()
            .iter()
            .map(|(key, r)| {
                (
                    r.delay_nanos(),
                    image.route_failure_policy(key),
                    image.route_transport_policy(key),
                    image.route_codec_policy(key),
                    image.route_timing_policy(key),
                    image.route_security_policy(key),
                )
            })
            .collect::<Vec<_>>();
        hash(
            "boomerang.coordination-fingerprint.v1",
            &(
                crate::check::COMPILER_SCHEMA,
                super::HOSTED_PROTOCOL,
                boomerang_federated::wire::PROTOCOL_VERSION,
                boomerang_federated::wire::CODEC_VERSION,
                (
                    boomerang_federated::wire::MAX_PAYLOAD_BYTES,
                    boomerang_federated::wire::MAX_MEMBER_BYTES,
                    boomerang_federated::wire::MAX_DIAGNOSTIC_BYTES,
                ),
                mapping(compiled)?.as_bytes(),
                members,
                dependencies,
                downstream,
                routes,
                contracts,
            ),
        )
    })
}

/// Identifies one actual scheduler image, its direct bindings, descriptors, and storage bounds.
/// The existing canonical image renderer owns the complete scheduler-table encoding.
pub(super) fn federate_image(
    slice: &FederateSlice<'_>,
    bindings: &[DescriptorDriverBinding],
) -> Result<blake3::Hash> {
    let required = slice
        .enclaves()
        .iter()
        .flat_map(|e| e.required_bindings().iter())
        .collect::<Vec<_>>();
    let mut descriptors = std::collections::BTreeMap::new();
    for binding in bindings {
        use boomerang_builder::compiler::RequiredBinding;
        if required.iter().any(|r| {
            let (RequiredBinding::State { component, .. }
            | RequiredBinding::Reaction { component, .. }
            | RequiredBinding::Port { component, .. }
            | RequiredBinding::Action { component, .. }) = r;
            component == binding.component()
        }) {
            descriptors.insert(
                (
                    binding.component().to_string(),
                    binding.implementation().as_str(),
                ),
                binding
                    .descriptor()
                    .descriptor_fingerprint_input()
                    .fingerprint()
                    .to_bytes(),
            );
        }
    }
    let bindings = required
        .iter()
        .map(|r| (binding_implementation(r), r.symbol()))
        .collect::<Vec<_>>();
    hash(
        "boomerang.federate-image.v1",
        &(
            rust::image_fingerprint_input(slice)?,
            bindings,
            descriptors.into_iter().collect::<Vec<_>>(),
        ),
    )
}

#[cfg(test)]
mod tests;
