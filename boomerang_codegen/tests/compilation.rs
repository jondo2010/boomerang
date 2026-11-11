mod common;
use boomerang_builder::compiler::*;
use boomerang_codegen::*;
use common::*;

#[test]
fn payload_paths_reject_source_injection() {
    assert!(PayloadExports::new("crate::controller::__boomerang::payload").is_ok());
    let mut compilation = compilation();
    let result = register_component!(
        compilation,
        "",
        metadata = common::controller,
        payload = target_only::controller
    );
    let error = result.unwrap_err();
    assert_eq!(error.diagnostic.stage, Stage::Registration);
    assert_eq!(error.diagnostic.code, DiagnosticCode::InvalidInput);
    assert!(std::error::Error::source(&error)
        .unwrap()
        .downcast_ref::<InvalidStableId>()
        .is_some());
    for path in [
        "crate::controller; panic!(\"injected\")",
        "",
        "Fn(u32)",
        "Foo::<u32>",
    ] {
        assert!(PayloadExports::new(path).is_err(), "{path}");
    }
}
#[test]
fn shared_registration_is_deterministic_and_does_not_generate_main() {
    let mut a = ready();
    register(&mut a, "unused", "crate::unused::__boomerang::payload");
    let mut b = compilation();
    register(&mut b, "unused", "crate::unused::__boomerang::payload");
    let mut compilation_evaluations = 0;
    let mut identity_evaluations = 0;
    let implementation = register_component!(
        {
            compilation_evaluations += 1;
            &mut b
        },
        {
            identity_evaluations += 1;
            "controller"
        },
        metadata = common::controller,
        payload = crate::controller,
    )
    .unwrap();
    assert_eq!(implementation.as_str(), "controller");
    assert_eq!(compilation_evaluations, 1);
    assert_eq!(identity_evaluations, 1);
    select(&mut b, "second", "controller");
    select(&mut b, "first", "controller");
    let a = a.compile().unwrap();
    let b = b.compile().unwrap();
    assert_eq!(a.source, b.source);
    assert_eq!(format!("{:?}", a.image), format!("{:?}", b.image));
    let parsed = syn::parse_file(&a.source).unwrap();
    assert!(!parsed
        .items
        .iter()
        .any(|item| matches!(item,syn::Item::Fn(f) if f.sig.ident=="main")));
    assert_eq!(
        a.image
            .enclaves()
            .values()
            .map(|e| e.reactors().len())
            .sum::<usize>(),
        2
    );
}
#[test]
fn each_instance_selects_its_own_implementation() {
    let mut c = compilation();
    register(&mut c, "alpha", "crate::alpha::__boomerang::payload");
    register(&mut c, "beta", "crate::beta::__boomerang::payload");
    select(&mut c, "first", "beta");
    select(&mut c, "second", "alpha");
    let output = c.compile().unwrap();
    let bindings = output
        .image
        .enclaves()
        .values()
        .flat_map(|e| e.required_bindings().iter())
        .collect::<Vec<_>>();
    assert!(bindings.iter().any(|b|matches!(b,RequiredBinding::State {component,implementation,..} if component.to_string()=="first" && implementation.as_str()=="beta")));
    assert!(bindings.iter().any(|b|matches!(b,RequiredBinding::State {component,implementation,..} if component.to_string()=="second" && implementation.as_str()=="alpha")));
}
#[test]
fn missing_selection_and_registration_have_context() {
    let error = compilation().compile().unwrap_err();
    assert_eq!(error.diagnostic.stage, Stage::Resolve);
    assert_eq!(error.diagnostic.deployment.as_deref(), Some("test"));
    assert_eq!(
        error.diagnostic.component.as_ref().unwrap().to_string(),
        "first"
    );
    let mut c = compilation();
    select(&mut c, "first", "missing");
    select(&mut c, "second", "missing");
    let error = c.compile().unwrap_err();
    assert_eq!(error.diagnostic.code, DiagnosticCode::MissingRegistration);
    assert_eq!(error.diagnostic.implementation.unwrap().as_str(), "missing");
}
#[test]
fn duplicates_fail_without_replacing_the_original_selection() {
    let mut c = ready();
    let error = c
        .register(
            ImplementationId::new("controller").unwrap(),
            controller::__boomerang::metadata::descriptor(),
            PayloadExports::new("crate::other").unwrap(),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic.code, DiagnosticCode::DuplicateRegistration);
    assert_eq!(
        c.select(
            ComponentInstanceId::new("first").unwrap(),
            ImplementationId::new("other").unwrap()
        )
        .unwrap_err()
        .diagnostic
        .code,
        DiagnosticCode::DuplicateSelection
    );
    c.compile().unwrap();
}
#[test]
fn incompatible_contract_and_version_retain_builder_error() {
    for (contract, version) in [("wrong", 1), ("example.controller", 2)] {
        let mut topology = ApplicationTopologyBuilder::new("app").unwrap();
        topology
            .add_component(ComponentInstance::new("first", contract, version).unwrap())
            .unwrap();
        let mut c = Compilation::new(
            topology.finish().unwrap(),
            configuration(),
            FederateId::new("local").unwrap(),
            target(),
        );
        register(
            &mut c,
            "controller",
            "crate::controller::__boomerang::payload",
        );
        select(&mut c, "first", "controller");
        let e = c.compile().unwrap_err();
        assert_eq!(e.diagnostic.code, DiagnosticCode::Resolve);
        assert_eq!(
            e.diagnostic.component.as_ref().unwrap().to_string(),
            "first"
        );
        assert!(e.to_string().contains("provides contract"));
    }
}
#[test]
fn target_must_match_selected_federate() {
    let c = Compilation::new(
        topology(),
        configuration(),
        FederateId::new("local").unwrap(),
        TargetContext::new("aarch64-unknown-linux-gnu").unwrap(),
    );
    assert_eq!(
        c.compile().unwrap_err().diagnostic.code,
        DiagnosticCode::TargetMismatch
    );
}

#[test]
fn lowering_errors_preserve_component_and_original_builder_error() {
    let descriptor = boomerang_builder::ComponentDescriptor::try_new(
        ContractId::new("example.controller").unwrap(),
        1,
        boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        boomerang_builder::DescriptorBounds::default(),
    )
    .unwrap();
    let mut c = compilation();
    c.register(
        ImplementationId::new("empty").unwrap(),
        descriptor,
        PayloadExports::new("crate::empty").unwrap(),
    )
    .unwrap();
    select(&mut c, "first", "empty");
    select(&mut c, "second", "empty");
    let error = c.compile().unwrap_err();
    assert_eq!(error.diagnostic.stage, Stage::Lower);
    assert_eq!(
        error.diagnostic.component.as_ref().unwrap().to_string(),
        "first"
    );
    assert!(std::error::Error::source(&error)
        .unwrap()
        .is::<CompileError>());
}

#[test]
fn missing_manual_clock_reports_the_component_and_binding() {
    let original = controller::__boomerang::metadata::descriptor();
    let mut reactors = original.reactor_slots().to_vec();
    reactors[0].capabilities = Some(boomerang_builder::InitializerCapabilities {
        source: None,
        actions: vec![],
        clock_control: true,
    });
    let descriptor = boomerang_builder::ComponentDescriptor::try_new(
        original.contract_id().clone(),
        original.contract_version(),
        original.macro_abi(),
        reactors,
        original.port_slots().to_vec(),
        original.action_slots().to_vec(),
        original.reaction_slots().to_vec(),
        original.mode_slots().to_vec(),
        original.state_slots().to_vec(),
        original.codec_slots().to_vec(),
        original.relationships().to_vec(),
        original.placement_groups().to_vec(),
        original.enclaves().to_vec(),
        original.bounds(),
    )
    .unwrap();
    let mut topology = TopologyBuilder::new("app").unwrap();
    let enclave = topology.enclave("first").unwrap();
    topology
        .component("first", controller::definition(), &enclave)
        .unwrap();
    let mut c = Compilation::new(
        topology.finish().unwrap(),
        configuration(),
        FederateId::new("local").unwrap(),
        target(),
    );
    c.register(
        ImplementationId::new("sourced").unwrap(),
        descriptor,
        PayloadExports::new("crate::sourced").unwrap(),
    )
    .unwrap();
    select(&mut c, "first", "sourced");
    let error = c.compile().unwrap_err();
    assert_eq!(error.diagnostic.stage, Stage::Render);
    assert_eq!(
        error.diagnostic.component.as_ref().unwrap().to_string(),
        "first"
    );
    assert_eq!(
        error.diagnostic.implementation.as_ref().unwrap().as_str(),
        "sourced"
    );
    assert_eq!(
        error.diagnostic.binding.as_deref(),
        Some("state_Controller")
    );
}
