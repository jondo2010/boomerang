//! Shared coordination definitions and canonical channel identities.
//!
//! These functions consume validated builder output and explicit topology metadata.
//! They preserve versioned hash domains, protocol inputs, and canonical table order.
//! Local scheduler images and artifact policy use separate fingerprints.
mod render;
use anyhow::{Context, Result};
use boomerang_builder::compiler::{ApplicationTopology, OwnedCompiledDeployment};
use boomerang_runtime::image::{
    BoundaryFailurePolicy, CodecPolicy, CoordinationProjection, RecoveryPolicy, RtiDependencyImage,
    SecurityPolicy, TimingPolicy, TransportPolicy,
};
use proc_macro2::TokenStream;

/// Compiler schema included in deployment reports and coordination identity.
pub const COMPILER_SCHEMA: u32 = 1;
/// Hosted canonical channel protocol included in coordination identity.
pub const HOSTED_PROTOCOL: &str = "boomerang.canonical.v1";

/// Hashes an explicitly ordered semantic record with an independent versioned domain.
fn hash(domain: &str, input: &impl serde::Serialize) -> Result<blake3::Hash> {
    let bytes = serde_json::to_vec(input).context("failed to encode canonical identity input")?;
    let mut hasher = blake3::Hasher::new_derive_key(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize())
}

/// Identifies the exact dense identity tables and route-to-member assignments.
///
/// Local coordination returns an error.
pub fn mapping_fingerprint(compiled: &OwnedCompiledDeployment) -> Result<blake3::Hash> {
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
/// Local coordination or a missing topology boundary contract returns an error.
pub fn coordination_fingerprint(
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
                    edge.id().to_canonical_string(),
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
                    m.in_transit_capacity(),
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
                COMPILER_SCHEMA,
                HOSTED_PROTOCOL,
                boomerang_federated::wire::PROTOCOL_VERSION,
                boomerang_federated::wire::CODEC_VERSION,
                (
                    boomerang_federated::wire::MAX_PAYLOAD_BYTES,
                    boomerang_federated::wire::MAX_MEMBER_BYTES,
                    boomerang_federated::wire::MAX_DIAGNOSTIC_BYTES,
                ),
                mapping_fingerprint(compiled)?.as_bytes(),
                members,
                dependencies,
                downstream,
                routes,
                contracts,
            ),
        )
    })
}

/// Validates the capabilities implemented by generated central RTI definitions and bindings.
///
/// Returns `false` for local coordination and `true` for supported central RTI coordination.
/// The generated backend uses TCP, bounded JSON payloads, and fail-stop recovery.
/// Unsupported boundary errors retain the stable boundary identity.
/// Runtime selection and launcher startup restrictions remain the caller's policy.
/// Generic builder lowering and identity hashing do not call this validator.
pub fn validate_generated_backend(compiled: &OwnedCompiledDeployment) -> Result<bool> {
    compiled.with_coordination(|projection| {
        let image = match projection {
            CoordinationProjection::Local => return Ok(false),
            CoordinationProjection::CentralRti(image) => image,
        };
        for (key, _) in image.routes().iter() {
            if image.route_transport_capability(key) != "tcp"
                || image.route_codec_capability(key) != "serde-json"
                || image.route_transport_policy(key) != TransportPolicy::ReliableOrderedFramed
                || image.route_codec_policy(key) != CodecPolicy::CanonicalBounded
                || image.route_failure_policy(key) != BoundaryFailurePolicy::PropagateStop
                || image.route_timing_policy(key) != TimingPolicy::BestEffort
                || image.route_security_policy(key) != SecurityPolicy::None
            {
                anyhow::bail!(
                    "unsupported generated central-rti boundary configuration for '{}'",
                    image.route_boundary(key).as_str()
                );
            }
        }
        for (key, _) in compiled.federates().iter() {
            if image.member_recovery_policy(key) != RecoveryPolicy::FailStop {
                anyhow::bail!("unsupported generated central-rti recovery policy");
            }
        }
        Ok(true)
    })
}

/// Emit immutable central RTI tables, identity, codec, and wire admission definitions.
///
/// The topology supplies boundary contract identities and versions for hashing.
/// Generated public names remain available at the including module boundary.
/// An internal namespace prevents collisions with scheduler definition imports.
/// This function emits no scheduler tables, payload bindings, startup, or execution.
/// Local coordination returns an error.
/// Unsupported generated-backend capabilities return an error before source emission.
pub fn render_definitions(
    compiled: &OwnedCompiledDeployment,
    topology: &ApplicationTopology,
) -> Result<TokenStream> {
    validate_generated_backend(compiled)?;
    let image = render::render_coordination(compiled)?;
    let identity = coordination_fingerprint(compiled, topology)?;
    let bytes = identity.as_bytes().iter();
    let mapping = mapping_fingerprint(compiled)?;
    let mapping_bytes = mapping.as_bytes().iter();
    Ok(quote::quote! {
        mod __boomerang_coordination {
            use boomerang_runtime::image::*;
            use tinymap::{SliceRange, TinyMapRef};
            #image
            /// Shared semantic compatibility claim; local image and artifact claims are distinct.
            pub const WIRE_COORDINATION_FINGERPRINT: boomerang_federated::wire::CoordinationFingerprint =
                boomerang_federated::wire::CoordinationFingerprint::new([#(#bytes),*]);
            /// Shared identity used by the central RTI and each member.
            pub const COORDINATION_IDENTITY: boomerang_central_rti::compiled::CoordinationIdentity =
                WIRE_COORDINATION_FINGERPRINT;
            /// Exact dense table mapping shared by the closed roster.
            pub const WIRE_MAPPING: [u8; 32] = [#(#mapping_bytes),*];
            /// Baseline canonical codec with the declared route profile's encoded-message bound.
            pub type WirePayloadCodec<T> = boomerang_federated::wire::PostcardCodec<T, {boomerang_federated::wire::MAX_PAYLOAD_BYTES}>;
            /// Binds portable admission directly to the generated typed RTI image.
            pub fn wire_contract() -> boomerang_federated::wire::Contract<'static, FederateIndex, RtiRouteIndex, RtiRouteImage<'static>> {
                boomerang_federated::wire::Contract::new(WIRE_COORDINATION_FINGERPRINT, WIRE_MAPPING,
                    COORDINATION_MEMBERS, *COORDINATION_IMAGE.routes(), |route| (route.source(), route.target()))
            }
            /// Creates the exact baseline handshake for a member of the generated roster.
            pub fn wire_handshake(member: FederateIndex) -> Option<boomerang_federated::wire::Handshake<'static>> {
                use boomerang_federated::wire::*;
                COORDINATION_MEMBERS.get(member).map(|member| Handshake {
                    protocol: PROTOCOL_VERSION, codec: CODEC_VERSION, coordination: WIRE_COORDINATION_FINGERPRINT,
                    epoch: 0, incarnation: 0, mapping: WIRE_MAPPING, member,
                })
            }
        }
        pub use __boomerang_coordination::{
            COORDINATION_IMAGE, COORDINATION_MEMBERS, COORDINATION_IDENTITY,
            WIRE_COORDINATION_FINGERPRINT, WIRE_MAPPING, WirePayloadCodec, wire_contract, wire_handshake,
        };
    })
}
