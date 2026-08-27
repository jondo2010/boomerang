use boomerang::prelude::*;

#[reactor(contract = "example.payload", contract_version = 7)]
pub fn payload() -> impl Reactor {}

#[cfg(feature = "__boomerang_payload")]
const _: () = boomerang::runtime::binding::assert_descriptor_fingerprint(
    boomerang::runtime::binding::DescriptorFingerprint::new([0; 32]),
    __boomerang::BINDING_MANIFEST.descriptor_fingerprint(),
);
