use boomerang::prelude::*;

#[reactor(contract = "example.payload", contract_version = 7)]
pub fn payload() -> impl Reactor {}

#[cfg(feature = "__boomerang_payload")]
#[test]
fn payload_mode_exports_matching_binding_manifest() {
    let descriptor = boomerang_builder::ComponentDescriptor::__from_macro(
        "example.payload",
        7,
        boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI,
        vec![boomerang_builder::ReactorSlot {
            id: boomerang_builder::ReactorSlotId::new("Payload").unwrap(),
            parent: None,
        }],
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
    );

    assert_eq!(
        __boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
        descriptor.descriptor_fingerprint_input().fingerprint(),
    );
    assert_eq!(
        __boomerang::BINDING_MANIFEST.macro_abi(),
        boomerang_builder::COMPONENT_DESCRIPTOR_MACRO_ABI,
    );
}
