use itertools::Itertools;
use std::sync::{mpsc, Arc, Mutex};

use super::*;
use crate::{port::Contained, runtime};

type RecordedValues = Vec<(runtime::Tag, u32)>;
type RecordedValuePair = (RecordedValues, RecordedValues);

fn run_lowered_federation_for_test(
    parts: RuntimeAssembly,
    config: runtime::Config,
) -> Result<
    boomerang_federated::static_runner::FederationEnvs,
    boomerang_federated::StaticFederationRunnerError,
> {
    let federation = parts
        .federation
        .expect("test federation must contain lowered runtime state");
    boomerang_federated::static_runner::run_in_memory(federation.runtime, parts.enclaves, config)
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
    fn take(parts: &mut RuntimeAssembly) -> Self {
        let federation = parts
            .federation
            .as_mut()
            .expect("federated assembly must contain lowered federation data");
        assert_eq!(federation.plan.endpoints.len(), 1);
        let source = boomerang_federated::FederateId::new(
            federation.plan.endpoints[0].source_federate.clone(),
        );
        let mailbox = federation
            .runtime
            .connections_mut()
            .take_mailbox(&source)
            .expect("source federate mailbox was created during lowering");
        Self { mailbox }
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

fn federated_shutdown_after_startup_sink_reactor(
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
        ctx.add_reaction(Some("shutdown_after_startup"))
            .with_trigger(startup)
            .with_reaction_fn(|ctx, _state, (_startup,)| {
                ctx.schedule_shutdown(Some(runtime::Duration::milliseconds(10)));
            })
            .finish()?;
        let values = Arc::clone(&values);
        ctx.add_reaction(Some("record_unexpected"))
            .with_trigger(input)
            .with_reaction_fn(move |ctx, _state, (input,)| {
                if let Some(value) = *input {
                    values.lock().unwrap().push((ctx.get_tag(), value));
                }
            })
            .finish()?;
        ctx.finish()?;
        Ok(input.contained())
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

fn federated_forwarding_reactor(addend: u32) -> impl Reactor<(), Ports = FederatedIoPorts> {
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
        let output = ctx.add_output_port::<u32>("out")?;
        let startup = ctx.get_startup_action();
        ctx.add_reaction(Some("keep_alive"))
            .with_trigger(startup)
            .with_reaction_fn(|ctx, _state, (_startup,)| {
                ctx.schedule_shutdown(Some(runtime::Duration::milliseconds(100)));
            })
            .finish()?;
        ctx.add_reaction(Some("forward"))
            .with_trigger(input)
            .with_effect(output)
            .with_reaction_fn(move |ctx, _state, (input, mut output)| {
                if let Some(value) = *input {
                    *output = Some(value + addend);
                    ctx.schedule_shutdown(None);
                }
            })
            .finish()?;
        ctx.finish()?;
        Ok(FederatedIoPorts {
            input: input.contained(),
            output: output.contained(),
        })
    }
}

fn federated_startup_recording_io_reactor(
    value: u32,
    values: Arc<Mutex<Vec<(runtime::Tag, u32)>>>,
) -> impl Reactor<(), Ports = FederatedIoPorts> {
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
        let output = ctx.add_output_port::<u32>("out")?;
        let startup = ctx.get_startup_action();
        ctx.add_reaction(Some("emit_startup"))
            .with_trigger(startup)
            .with_effect(output)
            .with_reaction_fn(move |ctx, _state, (_startup, mut output)| {
                *output = Some(value);
                ctx.schedule_shutdown(Some(runtime::Duration::milliseconds(100)));
            })
            .finish()?;
        let values = Arc::clone(&values);
        ctx.add_reaction(Some("record_feedback"))
            .with_trigger(input)
            .with_reaction_fn(move |ctx, _state, (input,)| {
                if let Some(value) = *input {
                    values.lock().unwrap().push((ctx.get_tag(), value));
                    ctx.schedule_shutdown(None);
                }
            })
            .finish()?;
        ctx.finish()?;
        Ok(FederatedIoPorts {
            input: input.contained(),
            output: output.contained(),
        })
    }
}

fn register_u32_federated_codec(assembly: &mut Assembly) -> Result<(), AssemblyError> {
    assembly.register_federated_codec::<u32, _>(boomerang_federated::SerdeJsonCodec)
}

fn route_outbound_commands_through_rti(
    plan: &FederationPlan,
    commands: Vec<boomerang_federated::FederateToRti>,
    connections: &boomerang_federated::FederatedRuntimeConnections,
) -> Vec<runtime::Tag> {
    let topology = federation_topology_from_plan(plan).unwrap();
    let mut rti = boomerang_federated::RtiState::new(topology.clone()).unwrap();
    for federate in &plan.federates {
        let federate_id = boomerang_federated::FederateId::new(federate.id.clone());
        rti.handle_from(
            &federate_id,
            boomerang_federated::FederateToRti::Hello {
                federate_id: federate_id.clone(),
                topology: topology.neighbors_for(&federate_id),
            },
        )
        .unwrap();
    }

    let mut routed_tags = Vec::new();
    for command in commands {
        let boomerang_federated::FederateToRti::Msg {
            source,
            target,
            endpoint,
            tag,
            payload,
        } = command
        else {
            panic!("lowered sender should emit a protocol MSG")
        };
        let deliveries = rti
            .handle_from(
                &source,
                boomerang_federated::FederateToRti::Msg {
                    source: source.clone(),
                    target: target.clone(),
                    endpoint: endpoint.clone(),
                    tag,
                    payload,
                },
            )
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
                let runtime_tag = runtime::Tag::try_from(*delivered_tag).unwrap();
                connections
                    .inbound_endpoint(&target, &endpoint)
                    .expect("lowered inbound endpoint")
                    .schedule(runtime_tag, payload)
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
    let mut assembly = Assembly::new();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_reactor(federated_startup_source_reactor(7), "source", (), true)
        .unwrap();
    let sink = ctx
        .add_child_reactor(
            federated_recording_sink_reactor(Arc::clone(&values)),
            "sink",
            (),
            true,
        )
        .unwrap();
    ctx.connect_port(source, sink, after, false).unwrap();
    ctx.finish().unwrap();

    let RuntimeAssembly { enclaves, .. } = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(100));
    let _envs = runtime::execute_enclaves(enclaves.into_iter(), config).unwrap();

    let recorded_values = values.lock().unwrap().clone();
    recorded_values
}

fn run_in_memory_federated_source_sink(
    after: Option<runtime::Duration>,
) -> (Vec<(runtime::Tag, u32)>, Vec<runtime::Tag>) {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
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
    ctx.connect_port(source, sink, after, false).unwrap();
    ctx.finish().unwrap();

    let mut parts = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();
    let mut outbound = FederatedOutboundCapture::take(&mut parts);
    let RuntimeAssembly {
        enclaves,
        aliases,
        federation,
        ..
    } = parts;
    let federation = federation.expect("source/sink lowering must produce a federation");
    let federation_plan = federation.plan;
    let federated_connections = federation.runtime.connections();

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
    let _source_envs =
        runtime::execute_enclaves(source_enclaves.into_iter(), config.clone()).unwrap();
    let commands = outbound.drain();
    let routed_tags =
        route_outbound_commands_through_rti(&federation_plan, commands, federated_connections);
    let _sink_envs = runtime::execute_enclaves(sink_enclaves.into_iter(), config).unwrap();

    let recorded_values = values.lock().unwrap().clone();
    (recorded_values, routed_tags)
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

    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(value)) => value,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => panic!("{label} timed out"),
    }
}

fn run_live_in_memory_federated_source_sink(
    after: Option<runtime::Duration>,
) -> Vec<(runtime::Tag, u32)> {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
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
    ctx.connect_port(source, sink, after, false).unwrap();
    ctx.finish().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = assembly.into_runtime_assembly(&config).unwrap();
    let _envs = run_lowered_federation_for_test(parts, config).unwrap();

    let recorded_values = values.lock().unwrap().clone();
    recorded_values
}

fn run_live_in_memory_no_message_source_sink() -> Vec<(runtime::Tag, u32)> {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_source_reactor(), "source", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(
            federated_shutdown_after_startup_sink_reactor(Arc::clone(&values)),
            "sink",
            (),
        )
        .unwrap();
    ctx.connect_port(source, sink, None, false).unwrap();
    ctx.finish().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = assembly.into_runtime_assembly(&config).unwrap();
    let _envs = run_lowered_federation_for_test(parts, config).unwrap();

    let recorded_values = values.lock().unwrap().clone();
    recorded_values
}

fn run_live_in_memory_three_federate_chain() -> Vec<(runtime::Tag, u32)> {
    let values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_startup_source_reactor(7), "source", ())
        .unwrap();
    let relay = ctx
        .add_child_federate(federated_forwarding_reactor(1), "relay", ())
        .unwrap();
    let sink = ctx
        .add_child_federate(
            federated_recording_sink_reactor(Arc::clone(&values)),
            "sink",
            (),
        )
        .unwrap();
    ctx.connect_port(source, relay.input, None, false).unwrap();
    ctx.connect_port(relay.output, sink, None, false).unwrap();
    ctx.finish().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = assembly.into_runtime_assembly(&config).unwrap();
    let _envs = run_lowered_federation_for_test(parts, config).unwrap();

    let recorded_values = values.lock().unwrap().clone();
    recorded_values
}

fn run_live_in_memory_fanout() -> RecordedValuePair {
    let left_values = Arc::new(Mutex::new(Vec::new()));
    let right_values = Arc::new(Mutex::new(Vec::new()));
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let source = ctx
        .add_child_federate(federated_startup_source_reactor(7), "source", ())
        .unwrap();
    let left = ctx
        .add_child_federate(
            federated_recording_sink_reactor(Arc::clone(&left_values)),
            "left",
            (),
        )
        .unwrap();
    let right = ctx
        .add_child_federate(
            federated_recording_sink_reactor(Arc::clone(&right_values)),
            "right",
            (),
        )
        .unwrap();
    ctx.connect_port(source, left, None, false).unwrap();
    ctx.connect_port(source, right, None, false).unwrap();
    ctx.finish().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = assembly.into_runtime_assembly(&config).unwrap();
    let _envs = run_lowered_federation_for_test(parts, config).unwrap();

    let recorded_left_values = left_values.lock().unwrap().clone();
    let recorded_right_values = right_values.lock().unwrap().clone();
    (recorded_left_values, recorded_right_values)
}

fn run_live_in_memory_positive_delay_cycle() -> RecordedValuePair {
    let a_values = Arc::new(Mutex::new(Vec::new()));
    let b_values = Arc::new(Mutex::new(Vec::new()));
    let delay = runtime::Duration::milliseconds(10);
    let mut assembly = Assembly::new();
    register_u32_federated_codec(&mut assembly).unwrap();
    let mut ctx = assembly.add_reactor("main", None, None, (), false);
    let a = ctx
        .add_child_federate(
            federated_startup_recording_io_reactor(1, Arc::clone(&a_values)),
            "a",
            (),
        )
        .unwrap();
    let b = ctx
        .add_child_federate(
            federated_startup_recording_io_reactor(2, Arc::clone(&b_values)),
            "b",
            (),
        )
        .unwrap();
    ctx.connect_port(a.output, b.input, Some(delay), false)
        .unwrap();
    ctx.connect_port(b.output, a.input, Some(delay), false)
        .unwrap();
    ctx.finish().unwrap();

    let config = runtime::Config::default().with_fast_forward(true);
    let parts = assembly.into_runtime_assembly(&config).unwrap();
    let _envs = run_lowered_federation_for_test(parts, config).unwrap();

    let recorded_a_values = a_values.lock().unwrap().clone();
    let recorded_b_values = b_values.lock().unwrap().clone();
    (recorded_a_values, recorded_b_values)
}

fn build_federated_source_sink_plan(
    after: Option<runtime::Duration>,
) -> Result<FederationPlan, AssemblyError> {
    Ok(build_federated_source_sink_parts(after)?
        .federation
        .expect("source/sink lowering must produce a federation")
        .plan)
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
fn test_federated_source_sink_topology_plan() {
    let parts = build_federated_source_sink_parts(None).unwrap();
    let federation = parts
        .federation
        .as_ref()
        .expect("source/sink lowering must produce a federation");
    let plan = &federation.plan;

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

    let topology = federation_topology_from_plan(plan).unwrap();
    assert_eq!(
        topology
            .federates
            .iter()
            .map(|federate| federate.as_str())
            .collect_vec(),
        vec!["source", "sink"]
    );
    assert_eq!(topology.edges.len(), 1);
    assert_eq!(topology.edges[0].source.as_str(), "source");
    assert_eq!(topology.edges[0].target.as_str(), "sink");
    assert_eq!(
        topology.edges[0].endpoint.as_str(),
        "main/source/out->main/sink/in"
    );
    assert_eq!(
        topology.edges[0].delay,
        boomerang_federated::WireDelay::ZERO
    );
    assert_eq!(federation.runtime.topology().topology(), &topology);
    assert_eq!(
        federation.runtime.federate_enclaves().len(),
        plan.federates.len()
    );
    for federate in &plan.federates {
        assert_eq!(
            federation
                .runtime
                .federate_enclaves()
                .get(&boomerang_federated::FederateId::new(federate.id.clone())),
            parts.aliases.enclave_aliases.get(federate.reactor)
        );
    }

    let routes = federated_routes_from_plan(plan).unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].endpoint.as_str(), "main/source/out->main/sink/in");
    assert_eq!(routes[0].source.as_str(), "source");
    assert_eq!(routes[0].target.as_str(), "sink");

    assert_eq!(
        parts
            .inter_partition_plan
            .partition_roots
            .iter()
            .filter_map(|root| match &root.kind {
                PartitionRootKind::Federated { federate } => Some(federate.as_str()),
                PartitionRootKind::LocalEnclave => None,
            })
            .collect_vec(),
        vec!["source", "sink"]
    );
    assert_eq!(parts.inter_partition_plan.edges.len(), 1);
    let boundary = &parts.inter_partition_plan.edges[0];
    assert_eq!(boundary.source_port, plan.endpoints[0].source_port);
    assert_eq!(boundary.target_port, plan.endpoints[0].target_port);
    assert!(matches!(
        &boundary.kind,
        BoundaryKind::Federated {
            source_federate,
            target_federate
        } if source_federate == "source" && target_federate == "sink"
    ));
    assert_eq!(boundary.delay, None);
    assert!(!boundary.physical);
}

#[test]
fn test_delayed_cross_federate_connection_records_delay() {
    let delay = runtime::Duration::milliseconds(10);
    let plan = build_federated_source_sink_plan(Some(delay)).unwrap();

    assert_eq!(plan.edges.len(), 1);
    assert_eq!(plan.edges[0].delay, Some(delay));
    let topology = federation_topology_from_plan(&plan).unwrap();
    assert_eq!(
        topology.edges[0].delay,
        boomerang_federated::WireDelay::from_nanos(10_000_000)
    );
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
fn test_live_in_memory_distributed_hello_records_zero_tag() {
    let values = run_with_wall_timeout("live in-memory distributed hello", || {
        run_live_in_memory_federated_source_sink(None)
    });

    assert_eq!(values, vec![(runtime::Tag::ZERO, 7)]);
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
fn test_in_memory_distributed_delayed_connection_matches_local_tag() {
    let delay = runtime::Duration::milliseconds(10);
    let local_values = run_local_source_sink(Some(delay));
    let (federated_values, routed_tags) = run_in_memory_federated_source_sink(Some(delay));

    assert_eq!(local_values, vec![(runtime::Tag::new(delay, 0), 7)]);
    assert_eq!(federated_values, local_values);
    assert_eq!(routed_tags, vec![runtime::Tag::new(delay, 0)]);
}

#[test]
fn test_live_in_memory_distributed_delayed_connection_records_delay_tag() {
    let delay = runtime::Duration::milliseconds(10);
    let values = run_with_wall_timeout("live in-memory delayed federation", move || {
        run_live_in_memory_federated_source_sink(Some(delay))
    });

    assert_eq!(values, vec![(runtime::Tag::new(delay, 0), 7)]);
}

#[test]
fn test_live_in_memory_no_message_topology_terminates_without_timeout() {
    let values = run_with_wall_timeout("live in-memory no-message federation", || {
        run_live_in_memory_no_message_source_sink()
    });

    assert!(values.is_empty());
}

#[test]
fn test_live_in_memory_three_federate_chain_records_relayed_value() {
    let values = run_with_wall_timeout("live in-memory three-federate chain", || {
        run_live_in_memory_three_federate_chain()
    });

    assert_eq!(values, vec![(runtime::Tag::ZERO, 8)]);
}

#[test]
fn test_live_in_memory_fanout_delivers_same_tag_to_each_sink() {
    let (left_values, right_values) =
        run_with_wall_timeout("live in-memory fanout", run_live_in_memory_fanout);

    assert_eq!(left_values, vec![(runtime::Tag::ZERO, 7)]);
    assert_eq!(right_values, vec![(runtime::Tag::ZERO, 7)]);
}

#[test]
fn test_live_in_memory_positive_delay_cycle_records_delayed_feedback() {
    let delay = runtime::Duration::milliseconds(10);
    let (a_values, b_values) = run_with_wall_timeout("live in-memory positive-delay cycle", || {
        run_live_in_memory_positive_delay_cycle()
    });

    assert_eq!(a_values, vec![(runtime::Tag::new(delay, 0), 2)]);
    assert_eq!(b_values, vec![(runtime::Tag::new(delay, 0), 1)]);
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
        AssemblyError::UnsupportedFederationTopology { what }
            if what.contains("duplicate federate id 'same'")
    ));
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

    assert_eq!(parts.inter_partition_plan.edges.len(), 1);
    let boundary = &parts.inter_partition_plan.edges[0];
    assert!(matches!(boundary.kind, BoundaryKind::LocalEnclave));
    assert_eq!(boundary.source_port, source.into());
    assert_eq!(boundary.target_port, sink.into());
    assert!(!boundary.physical);
    assert!(parts.federation.is_none());
    let source_enclave = parts.aliases.enclave_aliases[boundary.source_partition];
    let sink_enclave = parts.aliases.enclave_aliases[boundary.target_partition];
    assert_ne!(source_enclave, sink_enclave);
    assert!(parts.enclaves[source_enclave].upstream_enclaves.is_empty());
    assert_eq!(
        parts.enclaves[source_enclave]
            .downstream_enclaves
            .keys()
            .collect_vec(),
        vec![sink_enclave]
    );
    assert_eq!(
        parts.enclaves[sink_enclave]
            .upstream_enclaves
            .keys()
            .collect_vec(),
        vec![source_enclave]
    );
    assert!(parts.enclaves[sink_enclave].downstream_enclaves.is_empty());
    assert_eq!(
        parts
            .enclaves
            .values()
            .map(|enclave| enclave.upstream_enclaves.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        parts
            .enclaves
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

    let federation = parts
        .federation
        .as_ref()
        .expect("federated connection lowering must produce a federation");
    assert_eq!(federation.plan.endpoints.len(), 1);
    let endpoint = &federation.plan.endpoints[0].id;
    let routes = federation.runtime.connections().routes().collect_vec();
    assert_eq!(routes.len(), 1);
    assert_eq!(&routes[0].endpoint, endpoint);
    assert_eq!(routes[0].source.as_str(), "source");
    assert_eq!(routes[0].target.as_str(), "sink");
    let sink = boomerang_federated::FederateId::new("sink");
    assert!(federation
        .runtime
        .connections()
        .inbound_endpoint(&sink, endpoint)
        .is_some());
    assert!(parts.enclaves.values().all(|enclave| {
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

    let mut parts = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();
    let mut outbound = FederatedOutboundCapture::take(&mut parts);
    let RuntimeAssembly { enclaves, .. } = parts;

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(1));
    let _envs = runtime::execute_enclaves(enclaves.into_iter(), config).unwrap();

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

    let RuntimeAssembly {
        enclaves,
        federation,
        ..
    } = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();

    let federation = federation.expect("federated lowering must produce a federation");
    let federated_connections = federation.runtime.connections();
    let endpoint = boomerang_federated::EndpointId::new("main/source/out->main/sink/in");
    let sink = boomerang_federated::FederateId::new("sink");
    federated_connections
        .inbound_endpoint(&sink, &endpoint)
        .unwrap()
        .schedule(runtime::Tag::ZERO, b"42")
        .unwrap();

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(1));
    let _envs = runtime::execute_enclaves(enclaves.into_iter(), config).unwrap();

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
        AssemblyError::UnsupportedFederationTopology { what }
            if what.contains("distributed zero-delay cycle")
    ));
}
