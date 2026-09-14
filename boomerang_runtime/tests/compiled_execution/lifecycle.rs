//! Single-enclave lifecycle, typed storage, validation, and unsupported-route contracts.
use super::*;

#[test]
fn compiled_reference_executes_startup_to_shutdown() {
    let result = execute_owned(
        &IMAGE,
        reference_bindings(),
        Config::default().with_fast_forward(true),
    )
    .unwrap();

    assert_eq!(
        result
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap()
            .count,
        1
    );
    assert!(matches!(
        result.state::<CounterState>(StateSlotIndex::new(1)),
        Err(StateAccessError::OutOfRange { slot }) if slot == StateSlotIndex::new(1)
    ));
    assert!(matches!(
        result.state::<u32>(StateSlotIndex::new(0)),
        Err(StateAccessError::TypeMismatch { slot, expected, found })
            if slot == StateSlotIndex::new(0)
                && expected == std::any::type_name::<u32>()
                && found == std::any::type_name::<CounterState>()
    ));
    assert_eq!(result.final_tag(), Tag::new(Duration::nanoseconds(5), 0));
    assert!(result.stats().processed_tags() > 0);
    assert!(result.stats().processed_reactions() > 0);
}

#[test]
fn compiled_reference_terminal_only_execution_returns_never() {
    let result = execute_owned(
        &IMAGE,
        reference_bindings(),
        Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::ZERO),
    )
    .unwrap();

    assert_eq!(result.final_tag(), Tag::NEVER);
}

#[test]
fn compiled_reference_final_tag_includes_work_coalesced_with_shutdown() {
    let result = execute_owned(
        &COALESCED_IMAGE,
        reference_bindings(),
        Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::nanoseconds(1)),
    )
    .unwrap();

    assert_eq!(
        result
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap()
            .count,
        2
    );
    assert_eq!(result.final_tag(), Tag::new(Duration::nanoseconds(1), 0));
}

#[test]
fn compiled_reference_returns_typed_image_validation_error() {
    fn validate_local_image(image: EnclaveImage<'static>) -> ExecuteOwnedError<'static> {
        match execute_owned(&image, EnclaveBindings::new(), Config::default()) {
            Err(error) => error,
            Ok(_) => panic!("invalid image must fail validation"),
        }
    }

    let invalid_image = EnclaveImage {
        enclave_id: EnclaveId::new(" invalid"),
        ..IMAGE
    };
    let error = validate_local_image(invalid_image);

    assert!(matches!(
        error,
        ExecuteOwnedError::ImageValidation(ImageValidationError::InvalidStableId {
            kind: "enclave",
            index: 0,
            id: " invalid",
        })
    ));
}

#[test]
fn compiled_reference_rejects_routes_until_route_execution_is_supported() {
    let route_free_image = EnclaveImage {
        routes: TinyMapView::new(&ROUTES),
        ..ROUTED_IMAGE
    };
    execute_owned(
        &route_free_image,
        reference_bindings().bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new()),
        Config::default().with_fast_forward(true),
    )
    .expect("the same image must execute when its route table is empty");

    for direction in [RouteDirection::Inbound, RouteDirection::Outbound] {
        let routes = [fixture_route(
            "compiled/reference",
            PortIndex::new(0),
            direction,
            TimingDomain::Logical,
            0,
        )];
        let routed = EnclaveImage {
            routes: TinyMapView::new(&routes),
            ..ROUTED_IMAGE
        };
        match execute_owned(
            &routed,
            reference_bindings().bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new()),
            Config::default().with_fast_forward(true),
        ) {
            Ok(_) => panic!("{direction:?} routes must require a route-capable executor"),
            Err(ExecuteOwnedError::RoutesUnsupported { count: 1 }) => {}
            Err(error) => panic!("unexpected route rejection: {error}"),
        }
    }
}
