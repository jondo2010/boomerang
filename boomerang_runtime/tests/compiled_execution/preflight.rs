//! Owned Federate validation rejects incompatible bindings before initialization.
use super::*;

static ROUTED_INITIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

fn initialize_counted_routed_source() -> RoutedSourceState {
    ROUTED_INITIALIZATIONS.fetch_add(1, Ordering::SeqCst);
    initialize_routed_source()
}

fn counted_source_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counted_routed_source)
        .bind_reaction(BindingSlotIndex::new(1), emit_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

fn wrong_sink_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_sink)
        .bind_reaction(BindingSlotIndex::new(1), receive_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u64>::new())
}

#[test]
fn owned_federate_preflight_rejects_before_initializers() {
    use boomerang_runtime::image::{
        BoundaryFailurePolicy, CodecCapabilityIndex, CodecPolicy, FederationEdgeImage, FlowIndex,
        IdentityTable, PhysicalBoundaryIndex, RecoveryPolicy, RtiImage, RtiMemberImage,
        RtiRouteImage, SecurityPolicy, TimingPolicy, TransportCapabilityIndex, TransportPolicy,
    };
    ROUTED_INITIALIZATIONS.store(0, Ordering::SeqCst);
    let error = execute_owned_federate(
        ROUTED_DEPLOYMENT,
        FederateIndex::new(0),
        FederateBindings::new().bind_enclave(EnclaveIndex::new(0), counted_source_bindings()),
        Config::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::MissingEnclaveBinding { enclave }
            if enclave == EnclaveIndex::new(1)
    ));
    assert_eq!(ROUTED_INITIALIZATIONS.load(Ordering::SeqCst), 0);

    let unpaired_enclaves = [ROUTED_SOURCE_IMAGE];
    let unpaired_federates = [fixture_federate("host", "target", "runtime", s!(0, 1))];
    let unpaired = CompiledDeploymentImage {
        federates: TinyMapView::new(&unpaired_federates),
        enclaves: TinyMapView::new(&unpaired_enclaves),
        ..ROUTED_DEPLOYMENT
    };
    let error = execute_owned_federate(
        unpaired,
        FederateIndex::new(0),
        FederateBindings::new().bind_enclave(EnclaveIndex::new(0), counted_source_bindings()),
        Config::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::ImageValidation { .. }
    ));
    assert_eq!(ROUTED_INITIALIZATIONS.load(Ordering::SeqCst), 0);

    let error = execute_owned_federate(
        ROUTED_DEPLOYMENT,
        FederateIndex::new(0),
        FederateBindings::new()
            .bind_enclave(EnclaveIndex::new(0), counted_source_bindings())
            .bind_enclave(EnclaveIndex::new(1), wrong_sink_bindings())
            .bind_route(
                route_boundary(),
                PayloadType::<u32>::new(),
                PayloadType::<u32>::new(),
            ),
        Config::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::RoutePayloadTypeMismatch {
            direction: RouteDirection::Inbound,
            enclave,
            ..
        } if enclave == EnclaveIndex::new(1)
    ));
    assert_eq!(ROUTED_INITIALIZATIONS.load(Ordering::SeqCst), 0);

    let (source, bindings, source_identity) = {
        #[derive(Clone, Debug)]
        struct Collision;
        (
            EnclaveBindings::new()
                .bind_state(BindingSlotIndex::new(0), initialize_routed_source)
                .bind_reaction(BindingSlotIndex::new(1), emit_routed_value)
                .bind_port(BindingSlotIndex::new(2), PayloadType::<Collision>::new()),
            FederateBindings::new().bind_route(
                route_boundary(),
                PayloadType::<Collision>::new(),
                PayloadType::<Collision>::new(),
            ),
            (
                std::any::TypeId::of::<Collision>(),
                std::any::type_name::<Collision>(),
            ),
        )
    };
    let sink = {
        #[derive(Debug)]
        struct Collision;
        assert_eq!(source_identity.1, std::any::type_name::<Collision>());
        assert_ne!(source_identity.0, std::any::TypeId::of::<Collision>());
        EnclaveBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_routed_sink)
            .bind_reaction(BindingSlotIndex::new(1), receive_routed_value)
            .bind_port(BindingSlotIndex::new(2), PayloadType::<Collision>::new())
    };
    let error = execute_owned_federate(
        ROUTED_DEPLOYMENT,
        FederateIndex::new(0),
        bindings
            .bind_enclave(EnclaveIndex::new(0), source)
            .bind_enclave(EnclaveIndex::new(1), sink),
        Config::default().with_fast_forward(true),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::RoutePayloadTypeMismatch {
            direction: RouteDirection::Inbound,
            enclave,
            ..
        } if enclave == EnclaveIndex::new(1)
    ));

    let cross_federates = [
        fixture_federate("a", "t", "r", s!(0, 1)),
        fixture_federate("b", "t", "r", s!(1, 1)),
    ];
    let cross_members = [FederateIndex::new(0), FederateIndex::new(1)];
    let cross_edges = [FederationEdgeImage::new(
        BoundaryId::new("pipe"),
        FederateIndex::new(0),
        FederateIndex::new(1),
        1_000_000,
    )];
    let rti_members =
        [const { RtiMemberImage::new(RecoveryPolicy::FailStop, r!(0, 0), r!(0, 0), r!(0, 0)) }; 2];
    let rti_routes = [RtiRouteImage::new(
        BoundaryId::new("pipe"),
        FlowIndex::new(0),
        None::<PhysicalBoundaryIndex>,
        None,
        BoundaryFailurePolicy::PropagateStop,
        TransportPolicy::ReliableOrderedFramed,
        CodecPolicy::CanonicalBounded,
        TimingPolicy::BestEffort,
        SecurityPolicy::None,
        TransportCapabilityIndex::new(0),
        CodecCapabilityIndex::new(0),
        FederateIndex::new(0),
        FederateIndex::new(1),
        1_000_000,
    )];
    let rti_identities = ["x"];
    let rti = RtiImage::new(
        TinyMapView::new(&rti_members),
        &[],
        &[],
        TinyMapView::new(&rti_routes),
        IdentityTable::new(&rti_identities),
        IdentityTable::new(&[]),
        IdentityTable::new(&rti_identities),
        IdentityTable::new(&rti_identities),
    );
    let cross = CompiledDeploymentImage {
        federation: GlobalFederationImage::new(&cross_members, &cross_edges),
        federates: TinyMapView::new(&cross_federates),
        coordination: CoordinationProjection::CentralRti(rti),
        ..ROUTED_DEPLOYMENT
    };
    let error = execute_owned_federate(
        cross,
        FederateIndex::new(0),
        FederateBindings::new().bind_enclave(EnclaveIndex::new(0), counted_source_bindings()),
        Config::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::CrossFederateRoute { .. }
    ));
    assert_eq!(ROUTED_INITIALIZATIONS.load(Ordering::SeqCst), 0);
}

#[test]
fn owned_federate_rejects_enclave_without_root_reactor() {
    let rootless_enclaves = [EnclaveImage {
        enclave_id: EnclaveId::new("rootless"),
        reactors: TinyMapView::new(&[]),
        actions: TinyMapView::new(&[]),
        ports: TinyMapView::new(&[]),
        reactions: TinyMapView::new(&[]),
        modes: TinyMapView::new(&[]),
        scopes: TinyMapView::new(&[]),
        reaction_triggers: &[],
        reaction_use_ports: &[],
        reaction_effect_ports: &[],
        reaction_actions: &[],
        reaction_modes: &[],
        scope_descendants: &[],
        scope_logical_actions: &[],
        scope_timer_startups: &[],
        scope_reset_reactions: &[],
        scope_startup_reactions: &[],
        scope_shutdown_reactions: &[],
        startup_actions: &[],
        timer_startup_actions: &[],
        shutdown_reactions: &[],
        shutdown_actions: &[],
        routes: TinyMapView::new(&[]),
        required_bindings: TinyMapView::new(&[]),
        storage_bounds: &StorageBounds::new(0, 0, 0, 0, 0, 0),
    }];
    let rootless_federates = [fixture_federate("host", "target", "runtime", s!(0, 1))];
    let members = [FederateIndex::new(0)];
    let deployment = CompiledDeploymentImage {
        federation: GlobalFederationImage::new(&members, &[]),
        federates: TinyMapView::new(&rootless_federates),
        enclaves: TinyMapView::new(&rootless_enclaves),
        coordination: CoordinationProjection::Local,
    };

    let error = execute_owned_federate(
        deployment,
        FederateIndex::new(0),
        FederateBindings::new().bind_enclave(EnclaveIndex::new(0), EnclaveBindings::new()),
        Config::default(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExecuteOwnedFederateError::ImageValidation { message }
            if message == "image[0].root_reactor references missing reactors[0]"
    ));
}
