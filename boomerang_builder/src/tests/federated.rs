use itertools::Itertools;
use std::sync::{Arc, Mutex};

use super::*;
use crate::{port::Contained, runtime};

#[derive(Clone, Copy)]
struct FederatedIoPorts {
    input: TypedPortKey<u32, Input, Contained>,
    output: TypedPortKey<u32, Output, Contained>,
}

#[derive(Clone)]
struct LocalOnlyPayload {
    _value: Arc<Mutex<u32>>,
}

fn local_only_source_reactor(
) -> impl Reactor<(), Ports = TypedPortKey<LocalOnlyPayload, Output, Contained>> {
    |name: &str,
     state: (),
     parent: Option<BuilderReactorKey>,
     scope_mode: Option<BuilderModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     env: &mut EnvBuilder| {
        let mut builder = env.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            builder.set_scope_mode(scope_mode)?;
        }
        let output = builder
            .add_output_port::<LocalOnlyPayload>("out")?
            .contained();
        builder.finish()?;
        Ok(output)
    }
}

fn local_only_sink_reactor(
) -> impl Reactor<(), Ports = TypedPortKey<LocalOnlyPayload, Input, Contained>> {
    |name: &str,
     state: (),
     parent: Option<BuilderReactorKey>,
     scope_mode: Option<BuilderModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     env: &mut EnvBuilder| {
        let mut builder = env.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            builder.set_scope_mode(scope_mode)?;
        }
        let input = builder
            .add_input_port::<LocalOnlyPayload>("in")?
            .contained();
        builder.finish()?;
        Ok(input)
    }
}

fn federated_source_reactor() -> impl Reactor<(), Ports = TypedPortKey<u32, Output, Contained>> {
    |name: &str,
     state: (),
     parent: Option<BuilderReactorKey>,
     scope_mode: Option<BuilderModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     env: &mut EnvBuilder| {
        let mut builder = env.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            builder.set_scope_mode(scope_mode)?;
        }
        let output = builder.add_output_port::<u32>("out")?.contained();
        builder.finish()?;
        Ok(output)
    }
}

fn federated_startup_source_reactor(
    value: u32,
) -> impl Reactor<(), Ports = TypedPortKey<u32, Output, Contained>> {
    move |name: &str,
          state: (),
          parent: Option<BuilderReactorKey>,
          scope_mode: Option<BuilderModeKey>,
          bank_info: Option<runtime::BankInfo>,
          placement: ReactorPlacement,
          env: &mut EnvBuilder| {
        let mut builder = env.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            builder.set_scope_mode(scope_mode)?;
        }
        let output = builder.add_output_port::<u32>("out")?;
        let startup = builder.get_startup_action();
        builder
            .add_reaction(Some("emit"))
            .with_trigger(startup)
            .with_effect(output)
            .with_reaction_fn(move |_ctx, _state, (_startup, mut output)| {
                *output = Some(value);
            })
            .finish()?;
        builder.finish()?;
        Ok(output.contained())
    }
}

fn federated_sink_reactor() -> impl Reactor<(), Ports = TypedPortKey<u32, Input, Contained>> {
    |name: &str,
     state: (),
     parent: Option<BuilderReactorKey>,
     scope_mode: Option<BuilderModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     env: &mut EnvBuilder| {
        let mut builder = env.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            builder.set_scope_mode(scope_mode)?;
        }
        let input = builder.add_input_port::<u32>("in")?.contained();
        builder.finish()?;
        Ok(input)
    }
}

fn federated_recording_sink_reactor(
    values: Arc<Mutex<Vec<(runtime::Tag, u32)>>>,
) -> impl Reactor<(), Ports = TypedPortKey<u32, Input, Contained>> {
    move |name: &str,
          state: (),
          parent: Option<BuilderReactorKey>,
          scope_mode: Option<BuilderModeKey>,
          bank_info: Option<runtime::BankInfo>,
          placement: ReactorPlacement,
          env: &mut EnvBuilder| {
        let mut builder = env.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            builder.set_scope_mode(scope_mode)?;
        }
        let input = builder.add_input_port::<u32>("in")?;
        let values = Arc::clone(&values);
        builder
            .add_reaction(Some("record"))
            .with_trigger(input)
            .with_reaction_fn(move |ctx, _state, (input,)| {
                if let Some(value) = *input {
                    values.lock().unwrap().push((ctx.get_tag(), value));
                }
            })
            .finish()?;
        builder.finish()?;
        Ok(input.contained())
    }
}

fn federated_io_reactor() -> impl Reactor<(), Ports = FederatedIoPorts> {
    |name: &str,
     state: (),
     parent: Option<BuilderReactorKey>,
     scope_mode: Option<BuilderModeKey>,
     bank_info: Option<runtime::BankInfo>,
     placement: ReactorPlacement,
     env: &mut EnvBuilder| {
        let mut builder = env.add_reactor(name, parent, bank_info, state, placement);
        if let Some(scope_mode) = scope_mode {
            builder.set_scope_mode(scope_mode)?;
        }
        let input = builder.add_input_port::<u32>("in")?.contained();
        let output = builder.add_output_port::<u32>("out")?.contained();
        builder.finish()?;
        Ok(FederatedIoPorts { input, output })
    }
}

fn register_u32_federated_codec(env_builder: &mut EnvBuilder) -> Result<(), BuilderError> {
    env_builder.register_federated_codec::<u32, _>(boomerang_federated::SerdeJsonCodec)
}

fn wire_delay_from_runtime(delay: Option<runtime::Duration>) -> boomerang_federated::WireDelay {
    let Some(delay) = delay else {
        return boomerang_federated::WireDelay::ZERO;
    };

    boomerang_federated::WireDelay::from_nanos(delay.whole_nanoseconds().try_into().unwrap())
}

fn runtime_tag_to_wire_tag(tag: runtime::Tag) -> boomerang_federated::WireTag {
    if tag == runtime::Tag::NEVER {
        boomerang_federated::WireTag::NEVER
    } else if tag == runtime::Tag::FOREVER {
        boomerang_federated::WireTag::FOREVER
    } else {
        boomerang_federated::WireTag::finite(
            tag.offset().whole_nanoseconds(),
            tag.microstep().try_into().unwrap(),
        )
    }
}

fn wire_tag_to_runtime_tag(tag: boomerang_federated::WireTag) -> runtime::Tag {
    match tag {
        boomerang_federated::WireTag::Never => runtime::Tag::NEVER,
        boomerang_federated::WireTag::Forever => runtime::Tag::FOREVER,
        boomerang_federated::WireTag::Finite {
            offset_ns,
            microstep,
        } => runtime::Tag::new(
            runtime::Duration::nanoseconds(offset_ns.try_into().unwrap()),
            microstep.try_into().unwrap(),
        ),
    }
}

fn topology_from_plan(plan: &FederationPlan) -> boomerang_federated::FederatedTopology {
    boomerang_federated::FederatedTopology::with_edges(
        plan.federates
            .iter()
            .map(|federate| boomerang_federated::FederateId::new(federate.id.clone())),
        plan.edges.iter().map(|edge| {
            boomerang_federated::TopologyEdge::new(
                edge.source_federate.clone(),
                edge.target_federate.clone(),
                edge.endpoint.as_str(),
                wire_delay_from_runtime(edge.delay),
            )
        }),
    )
}

fn route_outbound_commands_through_rti(
    plan: &FederationPlan,
    commands: Vec<runtime::FederatedOutboundCommand>,
    inbound_endpoints: &runtime::FederatedInboundEndpointRegistry,
) -> Vec<runtime::Tag> {
    let topology = topology_from_plan(plan);
    let mut rti = boomerang_federated::RtiState::new(topology.clone());

    for federate in &plan.federates {
        let federate_id = boomerang_federated::FederateId::new(federate.id.clone());
        rti.handle(boomerang_federated::FederateToRti::Hello {
            federate_id: federate_id.clone(),
            topology: topology.neighbors_for(&federate_id),
        })
        .unwrap();
    }

    let mut routed_tags = Vec::new();
    for command in commands {
        let runtime::FederatedOutboundCommand::Msg(message) = command;
        let edge = plan
            .edges
            .iter()
            .find(|edge| edge.endpoint.as_str() == message.endpoint.as_str())
            .unwrap();
        let source = boomerang_federated::FederateId::new(edge.source_federate.clone());
        let target = boomerang_federated::FederateId::new(edge.target_federate.clone());
        let endpoint = boomerang_federated::EndpointId::new(message.endpoint.as_str());
        let tag = runtime_tag_to_wire_tag(message.tag);
        let deliveries = rti
            .handle(boomerang_federated::FederateToRti::Msg {
                source: source.clone(),
                target: target.clone(),
                endpoint: endpoint.clone(),
                tag,
                payload: message.payload,
            })
            .unwrap();

        assert_eq!(deliveries.len(), 1);
        let delivery = &deliveries[0];
        assert_eq!(delivery.federate_id, target);
        match &delivery.message {
            boomerang_federated::RtiToFederate::Msg {
                source: delivered_source,
                endpoint: delivered_endpoint,
                tag: delivered_tag,
                payload,
            } => {
                assert_eq!(delivered_source, &source);
                assert_eq!(delivered_endpoint, &endpoint);
                let runtime_tag = wire_tag_to_runtime_tag(*delivered_tag);
                inbound_endpoints
                    .schedule(&message.endpoint, runtime_tag, payload)
                    .unwrap();
                routed_tags.push(runtime_tag);
            }
            other => panic!("expected RTI-routed MSG delivery, got {other:?}"),
        }
    }

    routed_tags
}

fn run_local_source_sink(after: Option<runtime::Duration>) -> Vec<(runtime::Tag, u32)> {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut env_builder = EnvBuilder::new();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let source = builder
        .add_child_reactor(federated_startup_source_reactor(7), "source", (), true)
        .unwrap();
    let sink = builder
        .add_child_reactor(
            federated_recording_sink_reactor(Arc::clone(&values)),
            "sink",
            (),
            true,
        )
        .unwrap();
    builder.connect_port(source, sink, after, false).unwrap();
    builder.finish().unwrap();

    let BuilderRuntimeParts { enclaves, .. } = env_builder
        .into_runtime_parts(&runtime::Config::default())
        .unwrap();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(100));
    let _envs = runtime::execute_enclaves(enclaves.into_iter(), config);

    let recorded_values = values.lock().unwrap().clone();
    recorded_values
}

fn run_in_memory_federated_source_sink(
    after: Option<runtime::Duration>,
) -> (Vec<(runtime::Tag, u32)>, Vec<runtime::Tag>) {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut env_builder = EnvBuilder::new();
    register_u32_federated_codec(&mut env_builder).unwrap();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let source = builder
        .add_child_federate(federated_startup_source_reactor(7), "source", ())
        .unwrap();
    let sink = builder
        .add_child_federate(
            federated_recording_sink_reactor(Arc::clone(&values)),
            "sink",
            (),
        )
        .unwrap();
    builder.connect_port(source, sink, after, false).unwrap();
    builder.finish().unwrap();

    let BuilderRuntimeParts {
        enclaves,
        aliases,
        federation_plan,
        federated_outbound,
        federated_inbound_endpoints,
        ..
    } = env_builder
        .into_runtime_parts(&runtime::Config::default())
        .unwrap();

    let source_reactor = federation_plan
        .federates
        .iter()
        .find(|federate| federate.id == "source")
        .unwrap()
        .reactor;
    let sink_reactor = federation_plan
        .federates
        .iter()
        .find(|federate| federate.id == "sink")
        .unwrap()
        .reactor;
    let source_enclave_key = aliases.enclave_aliases[source_reactor];
    let sink_enclave_key = aliases.enclave_aliases[sink_reactor];

    let mut source_enclaves = Vec::new();
    let mut sink_enclaves = Vec::new();
    for (enclave_key, enclave) in enclaves {
        if enclave_key == source_enclave_key {
            source_enclaves.push((enclave_key, enclave));
        } else if enclave_key == sink_enclave_key {
            sink_enclaves.push((enclave_key, enclave));
        }
    }
    assert_eq!(source_enclaves.len(), 1);
    assert_eq!(sink_enclaves.len(), 1);

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(100));
    let _source_envs = runtime::execute_enclaves(source_enclaves.into_iter(), config.clone());
    let commands = federated_outbound.drain().unwrap();
    let routed_tags = route_outbound_commands_through_rti(
        &federation_plan,
        commands,
        &federated_inbound_endpoints,
    );
    let _sink_envs = runtime::execute_enclaves(sink_enclaves.into_iter(), config);

    let recorded_values = values.lock().unwrap().clone();
    (recorded_values, routed_tags)
}

fn build_federated_source_sink_plan(
    after: Option<runtime::Duration>,
) -> Result<FederationPlan, BuilderError> {
    let mut env_builder = EnvBuilder::new();
    register_u32_federated_codec(&mut env_builder)?;
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let source = builder.add_child_federate(federated_source_reactor(), "source", ())?;
    let sink = builder.add_child_federate(federated_sink_reactor(), "sink", ())?;
    builder.connect_port(source, sink, after, false)?;
    builder.finish()?;

    let parts = env_builder.into_runtime_parts(&runtime::Config::default())?;
    Ok(parts.federation_plan)
}

#[test]
fn test_add_child_federate_sets_enclave_compatible_placement() {
    let mut env_builder = EnvBuilder::new();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let _source = builder
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let main = builder.finish().unwrap();
    let source = env_builder.find_reactor_by_fqn("main/source").unwrap();

    assert!(!env_builder.reactor_builders[main].is_enclave);
    let source = &env_builder.reactor_builders[source];
    assert!(source.is_enclave);
    assert!(matches!(source.placement(), ReactorPlacement::Federate(spec) if spec.id == "source"));
}

#[test]
fn test_federated_source_sink_topology_plan() {
    let plan = build_federated_source_sink_plan(None).unwrap();

    assert_eq!(plan.federates.len(), 2);
    assert_eq!(
        plan.federates
            .iter()
            .map(|federate| federate.id.as_str())
            .collect_vec(),
        vec!["source", "sink"]
    );
    assert_eq!(plan.edges.len(), 1);
    assert_eq!(plan.endpoints.len(), 1);
    let edge = &plan.edges[0];
    assert_eq!(edge.source_federate, "source");
    assert_eq!(edge.target_federate, "sink");
    assert_eq!(edge.delay, None);
    assert_eq!(plan.endpoints[0].id, edge.endpoint);
    assert_eq!(plan.endpoints[0].source_port_fqn, "main/source/out");
    assert_eq!(plan.endpoints[0].target_port_fqn, "main/sink/in");
}

#[test]
fn test_delayed_cross_federate_connection_records_delay() {
    let delay = runtime::Duration::milliseconds(10);
    let plan = build_federated_source_sink_plan(Some(delay)).unwrap();

    assert_eq!(plan.edges.len(), 1);
    assert_eq!(plan.edges[0].delay, Some(delay));
}

#[test]
fn test_in_memory_distributed_hello_matches_local_enclave() {
    let local_values = run_local_source_sink(None);
    let (federated_values, routed_tags) = run_in_memory_federated_source_sink(None);

    assert_eq!(local_values, vec![(runtime::Tag::ZERO, 7)]);
    assert_eq!(federated_values, local_values);
    assert_eq!(routed_tags, vec![runtime::Tag::ZERO]);
}

#[test]
fn test_in_memory_distributed_delayed_connection_matches_local_tag() {
    let delay = runtime::Duration::milliseconds(10);
    let local_values = run_local_source_sink(Some(delay));
    let (federated_values, routed_tags) = run_in_memory_federated_source_sink(Some(delay));

    assert_eq!(local_values, vec![(runtime::Tag::new(delay, 0), 7)]);
    assert_eq!(federated_values, local_values);
    assert_eq!(routed_tags, vec![runtime::Tag::new(delay, 0)]);
}

#[test]
fn test_cross_federate_connection_without_codec_is_rejected() {
    let mut env_builder = EnvBuilder::new();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let source = builder
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = builder
        .add_child_federate(federated_sink_reactor(), "sink", ())
        .unwrap();
    builder.connect_port(source, sink, None, false).unwrap();
    builder.finish().unwrap();

    let error = match env_builder.into_runtime_parts(&runtime::Config::default()) {
        Ok(_) => panic!("cross-federate connection without codec should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        BuilderError::UnsupportedFederationTopology { what }
            if what.contains("requires a federated codec")
                && what.contains("register_federated_codec")
    ));
}

#[test]
fn test_local_cross_enclave_connection_does_not_require_federated_codec() {
    let mut env_builder = EnvBuilder::new();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let source = builder
        .add_child_reactor(local_only_source_reactor(), "source", (), true)
        .unwrap();
    let sink = builder
        .add_child_reactor(local_only_sink_reactor(), "sink", (), true)
        .unwrap();
    builder.connect_port(source, sink, None, false).unwrap();
    builder.finish().unwrap();

    let parts = env_builder
        .into_runtime_parts(&runtime::Config::default())
        .unwrap();

    assert!(parts.federation_plan.is_empty());
    assert_eq!(parts.federated_inbound_endpoints.len(), 0);
    assert!(parts.enclaves.values().any(|enclave| {
        !enclave.upstream_enclaves.is_empty() || !enclave.downstream_enclaves.is_empty()
    }));
}

#[test]
fn test_federated_connection_lowers_endpoint_runtime_parts() {
    let mut env_builder = EnvBuilder::new();
    register_u32_federated_codec(&mut env_builder).unwrap();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let source = builder
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = builder
        .add_child_federate(federated_sink_reactor(), "sink", ())
        .unwrap();
    builder.connect_port(source, sink, None, false).unwrap();
    builder.finish().unwrap();

    let parts = env_builder
        .into_runtime_parts(&runtime::Config::default())
        .unwrap();

    assert_eq!(parts.federation_plan.endpoints.len(), 1);
    assert_eq!(parts.federated_inbound_endpoints.len(), 1);
    assert!(parts.federated_outbound.is_empty().unwrap());
    assert!(parts.enclaves.values().all(|enclave| {
        enclave.upstream_enclaves.is_empty() && enclave.downstream_enclaves.is_empty()
    }));
}

#[test]
fn test_federated_sender_emits_serialized_msg_command() {
    let delay = runtime::Duration::milliseconds(10);
    let mut env_builder = EnvBuilder::new();
    register_u32_federated_codec(&mut env_builder).unwrap();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let source = builder
        .add_child_federate(federated_startup_source_reactor(7), "source", ())
        .unwrap();
    let sink = builder
        .add_child_federate(federated_sink_reactor(), "sink", ())
        .unwrap();
    builder
        .connect_port(source, sink, Some(delay), false)
        .unwrap();
    builder.finish().unwrap();

    let BuilderRuntimeParts {
        enclaves,
        federated_outbound,
        ..
    } = env_builder
        .into_runtime_parts(&runtime::Config::default())
        .unwrap();

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(1));
    let _envs = runtime::execute_enclaves(enclaves.into_iter(), config);

    let commands = federated_outbound.drain().unwrap();
    assert_eq!(commands.len(), 1);
    let runtime::FederatedOutboundCommand::Msg(message) = &commands[0];
    assert_eq!(message.endpoint.as_str(), "main/source/out->main/sink/in");
    assert_eq!(message.tag, runtime::Tag::new(delay, 0));
    assert_eq!(message.payload, b"7");
}

#[test]
fn test_federated_inbound_registry_schedules_target_action() {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut env_builder = EnvBuilder::new();
    register_u32_federated_codec(&mut env_builder).unwrap();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let source = builder
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = builder
        .add_child_federate(
            federated_recording_sink_reactor(Arc::clone(&values)),
            "sink",
            (),
        )
        .unwrap();
    builder.connect_port(source, sink, None, false).unwrap();
    builder.finish().unwrap();

    let BuilderRuntimeParts {
        enclaves,
        federated_inbound_endpoints,
        ..
    } = env_builder
        .into_runtime_parts(&runtime::Config::default())
        .unwrap();

    let endpoint = runtime::FederatedEndpointId::new("main/source/out->main/sink/in");
    federated_inbound_endpoints
        .schedule(&endpoint, runtime::Tag::ZERO, b"42")
        .unwrap();

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(1));
    let _envs = runtime::execute_enclaves(enclaves.into_iter(), config);

    assert_eq!(*values.lock().unwrap(), vec![(runtime::Tag::ZERO, 42)]);
}

#[test]
fn test_zero_delay_distributed_cycle_is_rejected() {
    let mut env_builder = EnvBuilder::new();
    register_u32_federated_codec(&mut env_builder).unwrap();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);
    let a = builder.add_child_federate(federated_io_reactor(), "a", ());
    let b = builder.add_child_federate(federated_io_reactor(), "b", ());
    let a = a.unwrap();
    let b = b.unwrap();
    builder
        .connect_port(a.output, b.input, None, false)
        .unwrap();
    builder
        .connect_port(b.output, a.input, None, false)
        .unwrap();
    builder.finish().unwrap();

    assert!(matches!(
        env_builder
            .into_runtime_parts(&runtime::Config::default())
            .expect_err("zero-delay distributed cycle should be rejected"),
        BuilderError::UnsupportedFederationTopology { what }
            if what.contains("distributed zero-delay cycle")
    ));
}
