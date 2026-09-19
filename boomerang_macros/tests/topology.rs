#![allow(unexpected_cfgs)]
use boomerang::builder::compiler::TopologyBuilder;
use boomerang::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
fn make_state() -> usize {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    7
}

boomerang::component! {
    mod example {
        use super::*;
        #[reactor(contract = "test.topology", contract_version = 1, bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn root(
            #[input] input: u32,
            #[logical_action(min_delay = 3 msec)] logical: u32,
            #[output] output: u32,
            #[physical_action(min_delay = 7 msec)] physical: u32,
            #[state(default = make_state())] value: usize,
        ) {
            reaction! { named(startup, input, logical) -> output, physical { } }
            reaction! { (shutdown) { } }
            mode! { initial Active {
                reaction! { (reset) -> history(Idle) { } }
            }}
            mode! { Idle {
                reaction! { (input) -> reset(Active) { } }
            }}
        }
    }
}
#[test]
fn generated_topology_matches_legacy_without_constructing_state() {
    let before = CONSTRUCTIONS.load(Ordering::SeqCst);
    let mut app = TopologyBuilder::new("application/root").unwrap();
    let enclave = app.enclave("root").unwrap();
    let _ports = app
        .component("root", example::definition(), &enclave)
        .unwrap();
    let generated = app.finish().unwrap();
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), before);
    let mut assembly = Assembly::new();
    example::Root()
        .build(
            "root",
            example::RootState::default(),
            None,
            None,
            None,
            false,
            &mut assembly,
        )
        .unwrap();
    let legacy = assembly.application_topology().unwrap();
    assert_eq!(generated, legacy);
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), before + 1);
}

boomerang::component! {
    mod identifiers {
        use boomerang::prelude::*;
        #[reactor(contract = "test.identifiers", contract_version = 1, bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn root(#[input] root: u32, #[output] topology: u32) {
            reaction! { (root) -> topology {} }
        }
    }
}
#[test]
fn user_port_names_cannot_shadow_constructor_locals() {
    let mut app = TopologyBuilder::new("application/identifiers").unwrap();
    let enclave = app.enclave("identifiers").unwrap();
    let ports = app
        .component("identifiers", identifiers::definition(), &enclave)
        .unwrap();
    assert_eq!(ports.root.id().to_string(), "identifiers/root");
    assert_eq!(ports.topology.id().to_string(), "identifiers/topology");
    assert_eq!(app.finish().unwrap().reactions().count(), 1);
}

boomerang::component! {
    mod banked {
        use boomerang::prelude::*;
        #[reactor(contract = "test.banked", contract_version = 1, bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn root(#[input] values: [u32; 2]) { reaction! { (values) {} } }
    }
}
boomerang::component! {
    mod generic {
        use boomerang::prelude::*;
        #[reactor(contract = "test.generic", contract_version = 1, bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn root<T: boomerang::runtime::ReactorData>(#[input] value: T) { reaction! { (value) {} } }
    }
}
#[test]
fn unsupported_topology_forms_keep_hosted_component_builds_functional() {
    let mut assembly = Assembly::new();
    let banked = banked::Root()
        .build("banked", (), None, None, None, false, &mut assembly)
        .unwrap();
    assert_eq!(banked.values.len(), 2);
    let _generic = generic::Root::<u32>()
        .build("generic", (), None, None, None, false, &mut assembly)
        .unwrap();
    assert_eq!(assembly.application_topology().unwrap().ports().count(), 3);
}

boomerang::component! {
    mod topology_named_root {
        use boomerang::prelude::*;
        #[reactor(contract = "test.topology-named-root", contract_version = 1, bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn topology(#[input] value: u32) {
            reaction! { (value) {} }
        }
    }
}
boomerang::component! {
    mod definition_named_root {
        use boomerang::prelude::*;
        #[reactor(contract = "test.definition-named-root", contract_version = 1, bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn definition(#[input] value: u32) {
            reaction! { (value) {} }
        }
    }
}
mod helper_names {
    pub struct TopologyPorts(pub u32);
}
boomerang::component! {
    mod helper_items {
        use boomerang::prelude::*;
        use super::helper_names::TopologyPorts;
        struct Definition(u32);
        pub fn helper_names_work() -> u32 {
            let definition = Definition(3);
            let ports = TopologyPorts(4);
            definition.0 + ports.0
        }
        #[reactor(contract = "test.helper-items", contract_version = 1, bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn root(#[input] value: u32) {
            reaction! { (value) {} }
        }
    }
}
#[test]
fn generated_types_preserve_root_and_helper_names() {
    let mut app = TopologyBuilder::new("application/component").unwrap();
    let enclave = app.enclave("component").unwrap();
    let ports = app
        .component("component", topology_named_root::definition(), &enclave)
        .unwrap();
    assert_eq!(ports.value.id().to_string(), "component/value");
    let mut legacy = Assembly::new();
    topology_named_root::Topology()
        .build("component", (), None, None, None, false, &mut legacy)
        .unwrap();
    assert_eq!(
        app.finish().unwrap(),
        legacy.application_topology().unwrap()
    );

    let mut app = TopologyBuilder::new("application/component").unwrap();
    let enclave = app.enclave("component").unwrap();
    let ports = app
        .component("component", definition_named_root::definition(), &enclave)
        .unwrap();
    assert_eq!(ports.value.id().to_string(), "component/value");
    let mut legacy = Assembly::new();
    definition_named_root::Definition()
        .build("component", (), None, None, None, false, &mut legacy)
        .unwrap();
    assert_eq!(
        app.finish().unwrap(),
        legacy.application_topology().unwrap()
    );

    let mut app = TopologyBuilder::new("application/component").unwrap();
    let enclave = app.enclave("component").unwrap();
    let ports = app
        .component("component", helper_items::definition(), &enclave)
        .unwrap();
    assert_eq!(ports.value.id().to_string(), "component/value");
    let mut legacy = Assembly::new();
    helper_items::Root()
        .build("component", (), None, None, None, false, &mut legacy)
        .unwrap();
    assert_eq!(
        app.finish().unwrap(),
        legacy.application_topology().unwrap()
    );
    assert_eq!(helper_items::helper_names_work(), 7);
}
