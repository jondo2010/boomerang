//! Mechanical rendering of the already lowered coordination projection.
use anyhow::{bail, Context, Result};
use boomerang_builder::compiler::OwnedCompiledDeployment;
use boomerang_runtime::image::*;
use proc_macro2::TokenStream;
use quote::quote;
use tinymap::SliceRange;

/// Emits standalone coordination tables without scheduler images or payload bindings.
///
/// Every table and relationship range is borrowed from its original owner. This is
/// serialization of lowering output, never another federation analysis pass.
pub(super) fn render_coordination(compiled: &OwnedCompiledDeployment) -> Result<TokenStream> {
    compiled
        .validate()
        .context("invalid compiled coordination deployment")?;
    let identities = compiled
        .federates()
        .values()
        .map(|member| member.id().as_str())
        .collect::<Vec<_>>();
    compiled.with_coordination(|projection| {
        let CoordinationProjection::CentralRti(image) = projection else {
            bail!("standalone coordination requires central-rti");
        };
        let tables = render_image(&image);
        Ok(quote! {
            const COORDINATION_MEMBERS: IdentityTable<'static, FederateIndex> =
                TinyMapView::new(&[#(#identities),*]);
            #tables
        })
    })
}

/// Emits each dense table and packed relationship slice in canonical owner order.
fn render_image(image: &RtiImage<'_>) -> TokenStream {
    let members = image.members().iter().map(|(key, member)| {
        let recovery = recovery_policy(image.member_recovery_policy(key));
        let direct = range(member.direct_incoming_range());
        let transitive = range(member.transitive_incoming_range());
        let downstream = range(member.affected_downstream_range());
        let capacity = member.in_transit_capacity();
        quote!(RtiMemberImage::new(#recovery, #direct, #transitive, #downstream, #capacity))
    });
    let dependencies = image.dependencies().iter().map(|entry| {
        let source = entry.source().as_u32();
        let delay = entry.delay_nanos();
        quote!(RtiDependencyImage::new(FederateIndex::new(#source), #delay))
    });
    let downstream = image.affected_downstream_entries().iter().map(|key| {
        let value = key.as_u32();
        quote!(FederateIndex::new(#value))
    });
    let routes = image.routes().iter().map(|(key, route)| {
        let boundary = image.route_boundary(key).as_str();
        let flow = route.flow().as_u32();
        let input = physical_key(route.physical_input());
        let output = physical_key(route.physical_output());
        let failure = failure_policy(image.route_failure_policy(key));
        let transport = transport_policy(image.route_transport_policy(key));
        let codec = codec_policy(image.route_codec_policy(key));
        let timing = timing_policy(image.route_timing_policy(key));
        let security = security_policy(image.route_security_policy(key));
        let transport_key = route.transport_capability().as_u32();
        let codec_key = route.codec_capability().as_u32();
        let source = route.source().as_u32();
        let target = route.target().as_u32();
        let delay = route.delay_nanos();
        quote! {
            RtiRouteImage::new(
                BoundaryId::new(#boundary), FlowIndex::new(#flow), #input, #output,
                #failure, #transport, #codec, #timing, #security,
                TransportCapabilityIndex::new(#transport_key), CodecCapabilityIndex::new(#codec_key),
                FederateIndex::new(#source), FederateIndex::new(#target), #delay,
            )
        }
    });
    let flows = image.flows().values();
    let physical = image.physical_boundaries().values();
    let transport = image.transport_capabilities().values();
    let codec = image.codec_capabilities().values();
    quote! {
        const COORDINATION_IMAGE: RtiImage<'static> = RtiImage::new(
            TinyMapView::new(&[#(#members),*]),
            &[#(#dependencies),*],
            &[#(#downstream),*],
            TinyMapView::new(&[#(#routes),*]),
            TinyMapView::new(&[#(#flows),*]),
            TinyMapView::new(&[#(#physical),*]),
            TinyMapView::new(&[#(#transport),*]),
            TinyMapView::new(&[#(#codec),*]),
        );
    }
}

/// Renders a relationship range against its unchanged original backing slice.
fn range<T>(value: SliceRange<T>) -> TokenStream {
    let start = value.start();
    let len = value.len();
    quote!(SliceRange::new(#start, #len))
}

/// Renders only a key from the physical-boundary table's own domain.
fn physical_key(value: Option<PhysicalBoundaryIndex>) -> TokenStream {
    value.map_or_else(
        || quote!(None),
        |key| {
            let value = key.as_u32();
            quote!(Some(PhysicalBoundaryIndex::new(#value)))
        },
    )
}

/// Defines exhaustive mechanical mappings for closed schema policies.
macro_rules! policy_renderer {
    ($function:ident, $ty:ident, $($variant:ident),+ $(,)?) => {
        #[doc = concat!("Renders the closed ", stringify!($ty), " selection.")]
        fn $function(value: $ty) -> TokenStream {
            match value {
                $($ty::$variant => quote!($ty::$variant),)+
            }
        }
    };
}

policy_renderer!(
    recovery_policy,
    RecoveryPolicy,
    FailStop,
    RestartReset,
    TransientRejoin,
    RedundantFailover,
    ApplicationStateTransfer,
    CheckpointRestore
);
policy_renderer!(
    failure_policy,
    BoundaryFailurePolicy,
    PropagateStop,
    ProduceAbsence,
    BoundedSafeValue,
    EnterDegradedMode,
    SwitchToStandby
);
policy_renderer!(transport_policy, TransportPolicy, ReliableOrderedFramed);
policy_renderer!(codec_policy, CodecPolicy, CanonicalBounded);
policy_renderer!(
    timing_policy,
    TimingPolicy,
    HardBound,
    SoftTarget,
    BestEffort
);
policy_renderer!(
    security_policy,
    SecurityPolicy,
    None,
    IntegrityOnly,
    Authenticated,
    AuthenticatedEncrypted
);
