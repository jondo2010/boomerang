use boomerang::prelude::component_payload as emit_component;
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/declaration.rs"));
controller_declaration!(emit_component);
include!(concat!(env!("OUT_DIR"), "/expected.rs"));
#[cfg(not(feature = "incorrect-export-abi"))]
const MANIFEST: boomerang::runtime::binding::BindingManifest =
    controller::__boomerang::payload::binding_manifest();
// Simulate an incorrectly exported header while keeping the real payload fingerprint intact.
#[cfg(feature = "incorrect-export-abi")]
const MANIFEST: boomerang::runtime::binding::BindingManifest =
    boomerang::runtime::binding::BindingManifest::new(
        controller::__boomerang::payload::binding_manifest().descriptor_fingerprint(),
        4,
    );
const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
    EXPECTED,
    MANIFEST.descriptor_fingerprint(),
);
#[cfg(feature = "fingerprint-mismatch")]
const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
    boomerang::runtime::binding::DescriptorFingerprint::new([0; 32]),
    MANIFEST.descriptor_fingerprint(),
);
// Independent compatibility contract for this fixture, not derived from the actual payload.
#[cfg(not(feature = "abi-mismatch"))]
const EXPECTED_MACRO_ABI: u32 = 3;
#[cfg(feature = "abi-mismatch")]
const EXPECTED_MACRO_ABI: u32 = 4;
const _: () = assert!(
    EXPECTED_MACRO_ABI == MANIFEST.macro_abi(),
    "payload macro ABI mismatch"
);
#[test]
fn payload_preserves_owned_implementation_and_actual_compatibility() {
    assert_eq!(
        controller::__boomerang::payload::state_Controller().count,
        21
    );
    let _typed_port = controller::__boomerang::payload::port_Controller_2fcommand;
    let _typed_reaction = controller::__boomerang::payload::reaction_Controller_2fcontrol;
    assert_eq!(MANIFEST.descriptor_fingerprint(), EXPECTED);
}

pub type ParentValue = u32;
#[derive(Debug)]
pub struct ParentState {
    /// Parent-module value retained by the relative-path payload state constructor.
    pub value: ParentValue,
}
fn initialize_parent() -> ParentState {
    ParentState { value: 13 }
}
fn parent_value() -> ParentValue {
    29
}
boomerang::component_payload! {
    pub mod relative {
        macro_rules! parent_increment { () => { super::parent_value() } }
        #[reactor(contract = "example.relative", contract_version = 1,
            bounds(queue_capacity = 1, payload_bytes = 4, state_bytes = 4, scratch_bytes = 4),
            state = super::ParentState, state_init = super::initialize_parent)]
        pub fn Root(#[output] output: super::ParentValue) -> impl Reactor {
            reaction! { tick(startup) -> output {
                state.value += super::parent_value() + parent_increment!();
                *output = Some(state.value);
            } }
        }
    }
}
#[test]
fn payload_paths_resolve_in_the_declarations_owning_module() {
    assert_eq!(relative::__boomerang::payload::state_Root().value, 13);
    let _: boomerang::runtime::PayloadType<ParentValue> =
        relative::__boomerang::payload::port_Root_2foutput;
}
