boomerang::component! {
    pub mod controller {
        use boomerang::prelude::*;
        use cargo_native_helper::value;
        pub fn helper() -> u32 { value() }
        #[reactor(contract = "example.controller", contract_version = 1,
            bounds(queue_capacity = 16, payload_bytes = 1024,
                   state_bytes = 512, scratch_bytes = 256))]
        pub fn Controller(#[output] command: u32,
            #[state(default = helper())] count: u32) -> impl Reactor {
            reaction! { control(startup) -> command { *command = Some(state.count + helper()); } }
        }
    }
}
#[cfg(feature = "metadata")]
#[test]
fn metadata_is_executable_without_implementation_helpers() {
    let descriptor = controller::__boomerang::metadata::descriptor();
    assert_eq!(descriptor.contract_id().as_str(), "example.controller");
    let mut topology = boomerang::builder::compiler::TopologyBuilder::new("app/root").unwrap();
    let enclave = topology.enclave("root").unwrap();
    let _ports = topology
        .component("root", controller::definition(), &enclave)
        .unwrap();
    topology.finish().unwrap();
}
#[cfg(feature = "payload")]
#[test]
fn payload_initializers_are_executable() {
    assert_eq!(
        controller::__boomerang::payload::state_Controller().count,
        21
    );
    let _reaction = controller::__boomerang::payload::reaction_Controller_2fcontrol;
}
#[cfg(all(feature = "metadata", feature = "payload"))]
#[test]
fn coexisting_namespaces_share_canonical_compatibility() {
    assert_eq!(
        controller::__boomerang::metadata::descriptor()
            .fingerprint(),
        <controller::__boomerang::payload::Payload as boomerang::runtime::binding::ComponentPayload>::DESCRIPTOR_FINGERPRINT
    );
}
#[cfg(not(any(feature = "metadata", feature = "payload")))]
#[test]
fn neither_feature_preserves_dynamic_api() {
    use boomerang::prelude::*;
    let mut assembly = Assembly::new();
    controller::Controller()
        .build(
            "root",
            controller::ControllerState::default(),
            None,
            None,
            None,
            false,
            &mut assembly,
        )
        .unwrap();
}
