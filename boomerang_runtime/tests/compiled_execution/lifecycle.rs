//! Single-enclave lifecycle, typed storage, validation, and unsupported-route contracts.
use super::*;

fn panicking_initializer() -> CounterState {
    panic!("injected initializer panic")
}

#[test]
#[cfg(feature = "bounded-tracing")]
fn bounded_runtime_trace_distinguishes_physical_and_logical_admission() {
    if !run_runtime_trace_test(
        "lifecycle::bounded_runtime_trace_distinguishes_physical_and_logical_admission",
    ) {
        return;
    }
    use boomerang_runtime::{
        Action, AsyncEvent, AsyncEventTarget, CommonContext, Enclave, EnclaveKey, Reactor,
        Scheduler,
    };
    use tracing_bounded::Value;
    let (subscriber, capture) = tracing_bounded::BoundedSubscriber::new(tracing_bounded::Config {
        records: 128,
        level: tracing::level_filters::LevelFilter::TRACE,
        targets: &["boomerang::runtime"],
        ..Default::default()
    })
    .unwrap();
    tracing::subscriber::with_default(subscriber, || {
        let _producer = capture.prepare_current_thread().unwrap();
        // Include compiled preflight/construction and its identity spans in the audit.
        execute_owned(
            &IMAGE,
            reference_bindings(),
            Config::default().with_fast_forward(true),
        )
        .unwrap();
        let mut enclave = Enclave::default();
        let reactor = enclave.insert_reactor(Reactor::new("input", ()).boxed(), None);
        let scope = enclave.root_scope(reactor);
        let action =
            enclave.insert_action(|key| Action::<u32>::new("key", key, None, false).boxed());
        enclave.insert_action_scope(action, scope);
        let sender = enclave.create_send_context(EnclaveKey::new(7));
        let mut scheduler = Scheduler::new(
            EnclaveKey::new(7),
            enclave,
            Config::default().with_fast_forward(true),
            None,
        );
        scheduler.startup();
        // Use the keyboard's real nonblocking submission API. The scheduler assigns
        // the physical tag; the logical tag must survive unchanged.
        assert_eq!(
            sender.try_schedule_async(AsyncEvent::Physical {
                time: std::time::Instant::now(),
                target: AsyncEventTarget::Action(action),
                value: Box::new(42_u32),
            }),
            Some(true)
        );
        assert_eq!(
            sender.try_schedule_async(AsyncEvent::Logical {
                tag: Tag::new(Duration::seconds(1), 3),
                target: AsyncEventTarget::Action(action),
                value: Box::new(99_u32),
            }),
            Some(true)
        );
        while scheduler.try_next().unwrap() {}
    });
    let records = capture.snapshot();
    fn field<'a>(record: &'a tracing_bounded::Record, name: &str) -> Option<&'a Value> {
        record
            .fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| &field.value)
    }
    let admissions: Vec<_> = records
        .iter()
        .filter(|record| {
            field(record, "event") == Some(&Value::Str("runtime.event.admitted".into()))
                && field(record, "kind") == Some(&Value::Str("action".into()))
        })
        .collect();
    assert_eq!(admissions.len(), 2, "{records:?}; {:?}", capture.loss());
    for (record, origin) in admissions.iter().zip(["physical", "logical"]) {
        assert_eq!(field(record, "origin"), Some(&Value::Str(origin.into())));
        assert_eq!(field(record, "enclave"), Some(&Value::U64(7)));
        assert_eq!(field(record, "action"), Some(&Value::U64(0)));
        assert_eq!(
            field(record, "tag_kind"),
            Some(&Value::Str("finite".into()))
        );
        assert!(field(record, "value").is_none());
        assert!(field(record, "payload").is_none());
    }
    assert_eq!(
        field(admissions[1], "tag_offset_ns"),
        Some(&Value::I128(1_000_000_000))
    );
    assert_eq!(field(admissions[1], "tag_microstep"), Some(&Value::U64(3)));
    assert_eq!(capture.loss().unsupported_value.value, 0);
    assert_eq!(capture.loss().invalid_context.value, 0);
    assert_eq!(capture.lifecycle_loss().span_admission.value, 0);
}

#[test]
fn runtime_lifecycle_trace_validates_identity_before_construction() {
    if !run_runtime_trace_test(
        "lifecycle::runtime_lifecycle_trace_validates_identity_before_construction",
    ) {
        return;
    }
    let (result, events) = capture_runtime(|| {
        execute_owned(
            &IMAGE,
            reference_bindings(),
            Config::default().with_fast_forward(true),
        )
    });
    result.unwrap();
    let lifecycle = lifecycle_events(&events);
    let names = lifecycle
        .iter()
        .map(|event| event["fields"]["event"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "runtime.preflight.started",
            "runtime.preflight.completed",
            "runtime.construction.started",
            "runtime.construction.completed",
        ]
    );
    assert_eq!(lifecycle[1]["span"]["name"], "runtime.enclave");
    assert_eq!(
        lifecycle[1]["span"]["enclave_id"],
        IMAGE.enclave_id.as_str()
    );
    assert_eq!(lifecycle[2]["fields"]["phase"], "storage");

    let invalid = EnclaveImage {
        enclave_id: EnclaveId::new(" invalid"),
        ..IMAGE
    };
    let (result, events) =
        capture_runtime(|| execute_owned(&invalid, EnclaveBindings::new(), Config::default()));
    assert!(result.is_err());
    assert_eq!(
        lifecycle_events(&events)
            .iter()
            .map(|event| event["fields"]["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["runtime.preflight.started", "runtime.preflight.rejected"]
    );

    let (result, events) =
        capture_runtime(|| execute_owned(&IMAGE, EnclaveBindings::new(), Config::default()));
    assert!(matches!(
        result,
        Err(ExecuteOwnedError::Storage(
            OwnedStorageError::MissingBinding { .. }
        ))
    ));
    let lifecycle = lifecycle_events(&events);
    assert_eq!(
        lifecycle
            .iter()
            .map(|event| event["fields"]["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["runtime.preflight.started", "runtime.preflight.rejected"]
    );
    assert_eq!(lifecycle[1]["fields"]["reason"], "binding");
    assert!(!lifecycle.iter().any(|event| event["fields"]["event"]
        .as_str()
        .is_some_and(|name| name.starts_with("runtime.construction"))));
}

#[test]
fn runtime_lifecycle_trace_reports_initializer_unwind() {
    if !run_runtime_trace_test("lifecycle::runtime_lifecycle_trace_reports_initializer_unwind") {
        return;
    }
    let (_, events) = capture_runtime(|| {
        std::panic::catch_unwind(|| {
            execute_owned(
                &IMAGE,
                EnclaveBindings::new()
                    .bind_state(BindingSlotIndex::new(0), panicking_initializer)
                    .bind_reaction(BindingSlotIndex::new(1), increment_counter),
                Config::default(),
            )
        })
    });
    let lifecycle = lifecycle_events(&events);
    let cancelled = lifecycle.last().unwrap();
    assert_eq!(
        cancelled["fields"]["event"],
        "runtime.construction.cancelled"
    );
    assert_eq!(cancelled["fields"]["phase"], "storage");
    assert_eq!(cancelled["fields"]["reason"], "unwind");
}

#[test]
fn local_federate_lifecycle_trace_covers_all_construction_phases() {
    if !run_runtime_trace_test(
        "lifecycle::local_federate_lifecycle_trace_covers_all_construction_phases",
    ) {
        return;
    }
    let (result, events) = capture_runtime(|| {
        execute_owned_federate(
            ROUTED_DEPLOYMENT,
            FederateIndex::new(0),
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(0), source_bindings())
                .bind_enclave(EnclaveIndex::new(1), sink_bindings())
                .bind_route(
                    route_boundary(),
                    PayloadType::<u32>::new(),
                    PayloadType::<u32>::new(),
                ),
            Config::default().with_fast_forward(true),
        )
    });
    result.unwrap();
    let lifecycle = lifecycle_events(&events);
    let phases = lifecycle
        .iter()
        .filter_map(|event| event["fields"]["phase"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        ["storage", "storage", "routes", "routes", "backend", "backend", "workers", "workers"]
    );
    assert!(lifecycle.iter().skip(1).all(|event| {
        event["span"]["name"] == "runtime.federate"
            && event["span"]["federate"] == 0
            && event["span"]["federate_id"] == "host"
            && event["span"]["ownership"] == "local"
    }));
}

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
