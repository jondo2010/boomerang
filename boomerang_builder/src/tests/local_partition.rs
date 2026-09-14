//! Local Assembly partition lowering contracts.

use super::*;
use std::sync::{Arc, Mutex};

/// Payload intentionally lacking serialization support.
#[derive(Clone)]
struct LocalOnlyPayload {
    _value: Arc<Mutex<u32>>,
}

/// Declare the source port for a local cross-Enclave connection.
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

/// Declare the destination port for a local cross-Enclave connection.
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

/// Local partition lowering accepts payloads without a serialization contract.
#[test]
fn test_local_cross_enclave_connection_accepts_non_serde_payload() {
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
    assert!(parts.enclaves.values().any(|enclave| {
        !enclave.upstream_enclaves.is_empty() || !enclave.downstream_enclaves.is_empty()
    }));
}
