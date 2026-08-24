use itertools::Itertools;
use std::sync::{mpsc, Arc, Mutex};

use super::*;
use crate::{port::Contained, runtime};

const TEST_WALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn run_lowered_federation_for_test(
    parts: RuntimeAssembly,
    config: runtime::Config,
) -> Result<
    boomerang_federated::static_runner::FederationEnvs,
    boomerang_federated::StaticFederationRunnerError,
> {
    let federation = parts
        .into_federation()
        .expect("test federation must contain lowered runtime state");
    boomerang_federated::static_runner::run_in_memory(federation, config)
}

#[derive(Clone, Copy)]
struct FederatedIoPorts {
    input: TypedPortKey<u32, Input, Contained>,
    output: TypedPortKey<u32, Output, Contained>,
}

#[derive(Clone)]
struct LocalOnlyPayload {
    _value: Arc<Mutex<u32>>,
}

#[derive(Clone, Copy)]
struct IntentionalFailingCodec;

struct FederatedOutboundCapture {
    mailbox: boomerang_federated::FederateClientMailbox,
}

impl FederatedOutboundCapture {
    fn take(
        parts: RuntimeAssembly,
    ) -> (
        Self,
        tinymap::TinyMap<runtime::EnclaveKey, runtime::Enclave>,
    ) {
        let federation = parts
            .into_federation()
            .expect("federated assembly must contain lowered federation data");
        assert_eq!(federation.graph().endpoint_ids().count(), 1);
        let source = federation
            .federates()
            .values()
            .flat_map(|federate| federate.bridge().routes())
            .next()
            .expect("lowered route exists")
            .source
            .clone();
        let (_, mut federates) = federation.into_parts();
        let (_, enclaves, bridge) = federates
            .remove(&source)
            .expect("source Federate exists")
            .into_parts();
        (
            Self {
                mailbox: bridge.into_mailbox(),
            },
            enclaves,
        )
    }

    fn drain(&mut self) -> Vec<boomerang_federated::FederateToRti> {
        let mut commands = Vec::new();
        while let Some(command) = self.mailbox.try_recv().unwrap() {
            commands.push(command);
        }
        commands
    }
}

impl boomerang_federated::PayloadEncoder<u32> for IntentionalFailingCodec {
    fn encode(&self, _value: &u32) -> Result<Vec<u8>, boomerang_federated::CodecError> {
        Err(boomerang_federated::CodecError::message(
            "intentional codec failure",
        ))
    }
}

impl boomerang_federated::PayloadDecoder<u32> for IntentionalFailingCodec {
    fn decode(&self, _bytes: &[u8]) -> Result<u32, boomerang_federated::CodecError> {
        Ok(0)
    }
}

fn local_only_source_reactor(
) -> impl Reactor<(), Ports = TypedPortKey<LocalOnlyPayload, Output, Contained>> {
    |name: &str,
     state: (),
     parent: Option<AssemblyReactorKey>,
     scope_mode: Option<AssemblyModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let output = ctx.add_output_port::<LocalOnlyPayload>("out")?.contained();
        ctx.finish()?;
        Ok(output)
    }
}

fn local_only_sink_reactor(
) -> impl Reactor<(), Ports = TypedPortKey<LocalOnlyPayload, Input, Contained>> {
    |name: &str,
     state: (),
     parent: Option<AssemblyReactorKey>,
     scope_mode: Option<AssemblyModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let input = ctx.add_input_port::<LocalOnlyPayload>("in")?.contained();
        ctx.finish()?;
        Ok(input)
    }
}

fn local_only_two_enclave_federate() -> impl Reactor<(), Ports = ()> {
    |name: &str,
     state: (),
     parent: Option<AssemblyReactorKey>,
     scope_mode: Option<AssemblyModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let source = ctx.add_child_reactor(local_only_source_reactor(), "source", (), false)?;
        let sink = ctx.add_child_reactor(local_only_sink_reactor(), "sink", (), true)?;
        ctx.connect_port(source, sink, None, false)?;
        ctx.finish()?;
        Ok(())
    }
}

fn nested_federate() -> impl Reactor<(), Ports = ()> {
    |name: &str,
     state: (),
     parent: Option<AssemblyReactorKey>,
     scope_mode: Option<AssemblyModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        ctx.add_child_reactor_with_placement(
            local_only_source_reactor(),
            "inner",
            (),
            ReactorPlacement::federate("inner"),
        )?;
        ctx.finish()?;
        Ok(())
    }
}

fn federated_source_reactor() -> impl Reactor<(), Ports = TypedPortKey<u32, Output, Contained>> {
    |name: &str,
     state: (),
     parent: Option<AssemblyReactorKey>,
     scope_mode: Option<AssemblyModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let output = ctx.add_output_port::<u32>("out")?.contained();
        ctx.finish()?;
        Ok(output)
    }
}

fn federated_startup_source_reactor(
    value: u32,
) -> impl Reactor<(), Ports = TypedPortKey<u32, Output, Contained>> {
    move |name: &str,
          state: (),
          parent: Option<AssemblyReactorKey>,
          scope_mode: Option<AssemblyModeKey>,
          bank_info: Option<runtime::BankInfo>,
          placement: ReactorPlacement,
          assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let output = ctx.add_output_port::<u32>("out")?;
        let startup = ctx.get_startup_action();
        ctx.add_reaction(Some("emit"))
            .with_trigger(startup)
            .with_effect(output)
            .with_reaction_fn(move |ctx, _state, (_startup, mut output)| {
                *output = Some(value);
                ctx.schedule_shutdown(None);
            })
            .finish()?;
        ctx.finish()?;
        Ok(output.contained())
    }
}

fn federated_sink_reactor() -> impl Reactor<(), Ports = TypedPortKey<u32, Input, Contained>> {
    |name: &str,
     state: (),
     parent: Option<AssemblyReactorKey>,
     scope_mode: Option<AssemblyModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let input = ctx.add_input_port::<u32>("in")?.contained();
        ctx.finish()?;
        Ok(input)
    }
}

fn federated_recording_sink_reactor(
    values: Arc<Mutex<Vec<(runtime::Tag, u32)>>>,
) -> impl Reactor<(), Ports = TypedPortKey<u32, Input, Contained>> {
    move |name: &str,
          state: (),
          parent: Option<AssemblyReactorKey>,
          scope_mode: Option<AssemblyModeKey>,
          bank_info: Option<runtime::BankInfo>,
          placement: ReactorPlacement,
          assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let input = ctx.add_input_port::<u32>("in")?;
        let startup = ctx.get_startup_action();
        ctx.add_reaction(Some("shutdown_if_no_input"))
            .with_trigger(startup)
            .with_reaction_fn(|ctx, _state, (_startup,)| {
                ctx.schedule_shutdown(Some(runtime::Duration::milliseconds(100)));
            })
            .finish()?;
        let values = Arc::clone(&values);
        ctx.add_reaction(Some("record"))
            .with_trigger(input)
            .with_reaction_fn(move |ctx, _state, (input,)| {
                if let Some(value) = *input {
                    values.lock().unwrap().push((ctx.get_tag(), value));
                    ctx.schedule_shutdown(None);
                }
            })
            .finish()?;
        ctx.finish()?;
        Ok(input.contained())
    }
}

fn federated_io_reactor() -> impl Reactor<(), Ports = FederatedIoPorts> {
    |name: &str,
     state: (),
     parent: Option<AssemblyReactorKey>,
     scope_mode: Option<AssemblyModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     assembly: &mut Assembly| {
        let mut ctx = assembly.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            ctx.set_scope_mode(scope_mode)?;
        }
        let input = ctx.add_input_port::<u32>("in")?.contained();
        let output = ctx.add_output_port::<u32>("out")?.contained();
        ctx.finish()?;
        Ok(FederatedIoPorts { input, output })
    }
}

fn register_u32_federated_codec(assembly: &mut Assembly) -> Result<(), AssemblyError> {
    assembly.register_federated_codec::<u32, _>(boomerang_federated::SerdeJsonCodec)
}

fn run_with_wall_timeout<T: Send + 'static>(
    label: &'static str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = tx.send(result);
    });

    match rx.recv_timeout(TEST_WALL_TIMEOUT) {
        Ok(Ok(value)) => value,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => panic!("{label} timed out"),
    }
}

fn build_federated_source_sink_parts(
    after: Option<runtime::Duration>,
) -> Result<RuntimeAssembly, AssemblyError> {
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly)?;
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx.add_child_federate(federated_source_reactor(), "source", ())?;
    let sink = ctx.add_child_federate(federated_sink_reactor(), "sink", ())?;
    ctx.connect_port(source, sink, after, false)?;
    ctx.finish()?;

    assembly.into_runtime_assembly(&runtime::Config::default())
}

#[test]
fn test_add_child_federate_sets_enclave_compatible_placement() {
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let _source = ctx
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let main = ctx.finish().unwrap();
    let source = assembly.find_reactor_by_fqn("main/source").unwrap();

    assert!(!assembly.reactor_specs[main].is_enclave);
    let source = &assembly.reactor_specs[source];
    assert!(source.is_enclave);
    assert!(matches!(source.placement(), ReactorPlacement::Federate(spec) if spec.id == "source"));
}

#[test]
fn test_federated_source_sink_lowers_authoritative_runtime_topology() {
    let parts = build_federated_source_sink_parts(None).unwrap();
    let federation = parts
        .federation()
        .expect("source/sink lowering must produce a federation");
    let graph = federation.graph();

    assert_eq!(
        graph
            .federate_ids()
            .map(|federate| federate.as_str())
            .collect_vec(),
        vec!["sink", "source"]
    );
    assert_eq!(graph.endpoint_ids().count(), 1);
    let endpoint = graph.endpoint_ids().next().unwrap();
    assert_eq!(endpoint.as_str(), "main/source/out->main/sink/in");
    assert_eq!(
        graph.endpoint_delay(endpoint),
        Some(boomerang_federated::WireDelay::ZERO)
    );
    assert_eq!(federation.federates().len(), 2);

    let routes = federation
        .federates()
        .values()
        .flat_map(|federate| federate.bridge().routes())
        .collect_vec();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].endpoint.as_str(), "main/source/out->main/sink/in");
    assert_eq!(routes[0].source.as_str(), "source");
    assert_eq!(routes[0].target.as_str(), "sink");
}

#[test]
fn test_delayed_cross_federate_connection_records_delay() {
    let delay = runtime::Duration::milliseconds(10);
    let parts = build_federated_source_sink_parts(Some(delay)).unwrap();
    let federation = parts.federation().unwrap();

    assert_eq!(
        federation
            .graph()
            .endpoint_delay(federation.graph().endpoint_ids().next().unwrap()),
        Some(boomerang_federated::WireDelay::from_nanos(10_000_000))
    );
}

#[test]
fn test_live_in_memory_intentional_codec_failure_is_returned() {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    assembly
        .register_federated_codec::<u32, _>(IntentionalFailingCodec)
        .unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_startup_source_reactor(7), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(
            federated_recording_sink_reactor(Arc::clone(&values)),
            "sink",
            (),
        )
        .unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.finish().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = assembly.into_runtime_assembly(&config).unwrap();
    let error = run_with_wall_timeout("intentional codec failure", move || {
        run_lowered_federation_for_test(parts, config).unwrap_err()
    });

    assert!(error.to_string().contains("intentional codec failure"));
    assert!(values.lock().unwrap().is_empty());
}

#[test]
fn test_cross_federate_connection_without_codec_is_rejected() {
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(federated_sink_reactor(), "sink", ())
        .unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.finish().unwrap();

    let error = match assembly.into_runtime_assembly(&runtime::Config::default()) {
        Ok(_) => panic!("cross-federate connection without codec should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        AssemblyError::UnsupportedFederationTopology { what }
            if what.contains("requires a federated codec")
                && what.contains("register_federated_codec")
    ));
}

#[test]
fn test_cross_federate_physical_connection_is_rejected() {
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(federated_sink_reactor(), "sink", ())
        .unwrap();
    ctx.connect_port(source, sink, None, true).unwrap();
    ctx.finish().unwrap();

    assert!(matches!(
        assembly
            .into_runtime_assembly(&runtime::Config::default())
            .expect_err("cross-federate physical connection should be rejected"),
        AssemblyError::UnsupportedFederationTopology { what }
            if what.contains("cross-federate physical connection")
    ));
}

#[test]
fn test_mixed_local_federated_boundary_is_rejected() {
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_reactor(federated_sink_reactor(), "sink", (), true)
        .unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.finish().unwrap();

    assert!(matches!(
        assembly
            .into_runtime_assembly(&runtime::Config::default())
            .expect_err("mixed local/federated boundary should be rejected"),
        AssemblyError::UnsupportedFederationTopology { what }
            if what.contains("crosses a federated boundary")
                && what.contains("both enclave roots are not federates")
    ));
}

#[test]
fn test_transient_federate_is_rejected() {
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    ctx.add_child_reactor_with_placement(
        federated_source_reactor(),
        "source",
        (),
        ReactorPlacement::Federate(FederateSpec::new("source").transient(true)),
    )
    .unwrap();
    ctx.finish().unwrap();

    assert!(matches!(
        assembly
            .into_runtime_assembly(&runtime::Config::default())
            .expect_err("transient federate should be rejected"),
        AssemblyError::UnsupportedFederationTopology { what }
            if what.contains("transient federate 'source'")
    ));
}

#[test]
fn test_empty_federate_id_is_rejected() {
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    ctx.add_child_reactor_with_placement(
        federated_source_reactor(),
        "source",
        (),
        ReactorPlacement::Federate(FederateSpec::new(" ")),
    )
    .unwrap();
    ctx.finish().unwrap();

    assert!(matches!(
        assembly
            .into_runtime_assembly(&runtime::Config::default())
            .expect_err("empty federate id should be rejected"),
        AssemblyError::UnsupportedFederationTopology { what }
            if what.contains("must have a non-empty id")
    ));
}

#[test]
fn test_duplicate_federate_id_is_rejected() {
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    ctx.add_child_reactor_with_placement(
        federated_source_reactor(),
        "source",
        (),
        ReactorPlacement::Federate(FederateSpec::new("same")),
    )
    .unwrap();
    ctx.add_child_reactor_with_placement(
        federated_sink_reactor(),
        "sink",
        (),
        ReactorPlacement::Federate(FederateSpec::new("same")),
    )
    .unwrap();
    ctx.finish().unwrap();

    assert!(matches!(
        assembly
            .into_runtime_assembly(&runtime::Config::default())
            .expect_err("duplicate federate id should be rejected"),
        AssemblyError::DuplicateFederateId { federate_id }
            if federate_id == "same"
    ));
}

#[test]
fn test_duplicate_federated_endpoint_is_rejected_with_focused_error() {
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(federated_sink_reactor(), "sink", ())
        .unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.finish().unwrap();

    assert!(matches!(
        assembly
            .into_runtime_assembly(&runtime::Config::default())
            .expect_err("duplicate federated endpoint should be rejected"),
        AssemblyError::DuplicateFederatedEndpoint { endpoint }
            if endpoint == "main/source/out->main/sink/in"
    ));
}

#[test]
fn test_nested_federate_scope_is_rejected() {
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    ctx.add_child_reactor_with_placement(
        nested_federate(),
        "outer",
        (),
        ReactorPlacement::federate("outer"),
    )
    .unwrap();
    ctx.finish().unwrap();

    assert!(matches!(
        assembly
            .into_runtime_assembly(&runtime::Config::default())
            .expect_err("nested Federate scopes should be rejected"),
        AssemblyError::UnsupportedFederationTopology { what }
            if what.contains("nested federate 'inner' inside federate 'outer'")
    ));
}

#[test]
fn test_same_federate_cross_enclave_boundary_stays_local() {
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    ctx.add_child_reactor_with_placement(
        local_only_two_enclave_federate(),
        "node",
        (),
        ReactorPlacement::federate("node"),
    )
    .unwrap();
    ctx.finish().unwrap();

    // LocalOnlyPayload deliberately has no federated codec. Lowering succeeds because the
    // boundary changes scheduler Enclaves but does not leave the Federate.
    let parts = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();
    let federation = parts.federation().unwrap();
    assert_eq!(federation.graph().endpoint_ids().count(), 0);
    assert_eq!(
        federation.federates()[&boomerang_federated::FederateId::new("node")]
            .enclaves()
            .len(),
        2
    );
    assert_eq!(
        federation.federates()[&boomerang_federated::FederateId::new("node")]
            .enclaves()
            .values()
            .map(|enclave| enclave.downstream_enclaves.len())
            .sum::<usize>(),
        1
    );
}

#[test]
fn test_local_cross_enclave_connection_does_not_require_federated_codec() {
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_reactor(local_only_source_reactor(), "source", (), true)
        .unwrap();
    let sink = ctx
        .add_child_reactor(local_only_sink_reactor(), "sink", (), true)
        .unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.finish().unwrap();

    let parts = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();

    assert!(parts.federation().is_none());
    let source_enclave = parts.aliases.port_aliases[AssemblyPortKey::from(source)]
        .0
        .enclave_key();
    let sink_enclave = parts.aliases.port_aliases[AssemblyPortKey::from(sink)]
        .0
        .enclave_key();
    assert_ne!(source_enclave, sink_enclave);
    let enclaves = parts.local_enclaves().unwrap();
    assert!(enclaves[source_enclave].upstream_enclaves.is_empty());
    assert_eq!(
        enclaves[source_enclave]
            .downstream_enclaves
            .keys()
            .collect_vec(),
        vec![sink_enclave]
    );
    assert_eq!(
        enclaves[sink_enclave]
            .upstream_enclaves
            .keys()
            .collect_vec(),
        vec![source_enclave]
    );
    assert!(enclaves[sink_enclave].downstream_enclaves.is_empty());
    assert_eq!(
        enclaves
            .values()
            .map(|enclave| enclave.upstream_enclaves.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        enclaves
            .values()
            .map(|enclave| enclave.downstream_enclaves.len())
            .sum::<usize>(),
        1
    );
}

#[test]
fn test_federated_connection_lowers_endpoint_runtime_parts() {
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(federated_sink_reactor(), "sink", ())
        .unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.finish().unwrap();

    let parts = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();

    let source_owner = parts.aliases.port_aliases[AssemblyPortKey::from(source)]
        .0
        .clone();
    let sink_owner = parts.aliases.port_aliases[AssemblyPortKey::from(sink)]
        .0
        .clone();
    assert!(matches!(
        &source_owner,
        EnclaveRef::Federated { federate, .. } if federate.as_str() == "source"
    ));
    assert!(matches!(
        &sink_owner,
        EnclaveRef::Federated { federate, .. } if federate.as_str() == "sink"
    ));
    assert_eq!(source_owner.enclave_key(), sink_owner.enclave_key());

    let federation = parts
        .federation()
        .expect("federated connection lowering must produce a federation");
    assert_eq!(federation.graph().endpoint_ids().count(), 1);
    let endpoint = federation.graph().endpoint_ids().next().unwrap();
    let routes = federation
        .federates()
        .values()
        .flat_map(|federate| federate.bridge().routes())
        .collect_vec();
    assert_eq!(routes.len(), 1);
    assert_eq!(&routes[0].endpoint, endpoint);
    assert_eq!(routes[0].source.as_str(), "source");
    assert_eq!(routes[0].target.as_str(), "sink");
    let sink = boomerang_federated::FederateId::new("sink");
    assert!(federation.federates()[&sink]
        .bridge()
        .inbound_endpoint(endpoint)
        .is_some());
    assert!(federation
        .federates()
        .values()
        .flat_map(|federate| federate.enclaves().values())
        .all(|enclave| {
            enclave.upstream_enclaves.is_empty() && enclave.downstream_enclaves.is_empty()
        }));
}

#[test]
fn test_federated_sender_emits_serialized_msg_command() {
    let delay = runtime::Duration::milliseconds(10);
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_startup_source_reactor(7), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(federated_sink_reactor(), "sink", ())
        .unwrap();
    ctx.connect_port(source, sink, Some(delay), false).unwrap();
    ctx.finish().unwrap();

    let parts = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();
    let (mut outbound, source_enclaves) = FederatedOutboundCapture::take(parts);

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(1));
    let _envs = runtime::execute_enclaves(source_enclaves.into_iter(), config).unwrap();

    let commands = outbound.drain();
    assert_eq!(commands.len(), 1);
    let boomerang_federated::FederateToRti::Msg {
        source,
        target,
        endpoint,
        tag,
        payload,
    } = &commands[0]
    else {
        panic!("lowered sender should emit a protocol MSG")
    };
    assert_eq!(source.as_str(), "source");
    assert_eq!(target.as_str(), "sink");
    assert_eq!(endpoint.as_str(), "main/source/out->main/sink/in");
    assert_eq!(
        *tag,
        boomerang_federated::WireTag::try_from(runtime::Tag::new(delay, 0)).unwrap()
    );
    assert_eq!(payload, b"7");
}

#[test]
fn test_federated_inbound_endpoint_schedules_target_action() {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(
            federated_recording_sink_reactor(Arc::clone(&values)),
            "sink",
            (),
        )
        .unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.finish().unwrap();

    let parts = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();

    let federation = parts.into_federation().unwrap();
    let endpoint = boomerang_federated::EndpointId::new("main/source/out->main/sink/in");
    let sink = boomerang_federated::FederateId::new("sink");
    federation.federates()[&sink]
        .bridge()
        .inbound_endpoint(&endpoint)
        .unwrap()
        .schedule(runtime::Tag::ZERO, b"42")
        .unwrap();
    let (_, mut federates) = federation.into_parts();
    let sink_enclaves = federates.remove(&sink).unwrap().into_parts().1;

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(1));
    let _envs = runtime::execute_enclaves(sink_enclaves.into_iter(), config).unwrap();

    assert_eq!(*values.lock().unwrap(), vec![(runtime::Tag::ZERO, 42)]);
}

#[test]
fn test_zero_delay_distributed_cycle_is_rejected() {
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let a = ctx.add_child_federate(federated_io_reactor(), "a", ());
    let b = ctx.add_child_federate(federated_io_reactor(), "b", ());
    let a = a.unwrap();
    let b = b.unwrap();
    ctx.connect_port(a.output, b.input, None, false).unwrap();
    ctx.connect_port(b.output, a.input, None, false).unwrap();
    ctx.finish().unwrap();

    assert!(matches!(
        assembly
            .into_runtime_assembly(&runtime::Config::default())
            .expect_err("zero-delay distributed cycle should be rejected"),
        AssemblyError::FederationZeroDelayCycle { federates }
            if federates == vec!["a".to_owned(), "b".to_owned()]
    ));
}
