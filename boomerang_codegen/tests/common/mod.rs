use boomerang_builder::compiler::*;
use boomerang_codegen::*;

boomerang::component_metadata! {
    pub mod controller {
        #[reactor(contract = "example.controller", contract_version = 1,
            bounds(queue_capacity = 16, payload_bytes = 1024, state_bytes = 512, scratch_bytes = 256))]
        pub fn Controller(#[state(default = 0)] count: u32) -> impl Reactor {
            reaction! { tick(startup) { state.count += 1; } }
        }
    }
}
pub fn topology() -> ApplicationTopology {
    let mut topology = TopologyBuilder::new("app").unwrap();
    let enclave = topology.enclave("first").unwrap();
    topology
        .component("first", controller::definition(), &enclave)
        .unwrap();
    topology
        .component("second", controller::definition(), &enclave)
        .unwrap();
    topology.finish().unwrap()
}
pub fn configuration() -> DeploymentConfiguration {
    DeploymentConfiguration {
        name: "test".into(),
        placements: vec![PlacementAssignment::new(
            PlacementGroupId::new("placement/first").unwrap(),
            FederateId::new("local").unwrap(),
        )],
        federates: vec![FederateConfig::new(
            FederateId::new("local").unwrap(),
            TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
            RuntimeBackendId::new("reference").unwrap(),
            boomerang_runtime::image::RecoveryPolicy::FailStop,
        )],
        coordination: CoordinationSelection::Local,
        boundaries: vec![],
    }
}
pub fn target() -> TargetContext {
    TargetContext::new("x86_64-unknown-linux-gnu").unwrap()
}
pub fn compilation() -> Compilation {
    Compilation::new(
        topology(),
        configuration(),
        FederateId::new("local").unwrap(),
        target(),
    )
}
pub fn register(compilation: &mut Compilation, name: &str, path: &str) {
    compilation
        .register(
            ImplementationId::new(name).unwrap(),
            controller::__boomerang::metadata::descriptor(),
            PayloadExports::new(path).unwrap(),
        )
        .unwrap();
}
pub fn select(compilation: &mut Compilation, component: &str, implementation: &str) {
    compilation
        .select(
            ComponentInstanceId::new(component).unwrap(),
            ImplementationId::new(implementation).unwrap(),
        )
        .unwrap();
}
pub fn ready() -> Compilation {
    let mut c = compilation();
    register(
        &mut c,
        "controller",
        "crate::controller::__boomerang::payload",
    );
    select(&mut c, "first", "controller");
    select(&mut c, "second", "controller");
    c
}
