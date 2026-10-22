use boomerang::component_metadata as emit_component;
include!("declaration.rs");
controller_declaration!(emit_component);
fn main() {
    let descriptor = controller::__boomerang::metadata::descriptor();
    assert_eq!(descriptor.contract_id().as_str(), "example.controller");
    assert_eq!(descriptor.macro_abi(), 3);
    let mut topology = boomerang::builder::compiler::TopologyBuilder::new("app/root").unwrap();
    let enclave = topology.enclave("root").unwrap();
    let _ports = topology
        .component("root", controller::definition(), &enclave)
        .unwrap();
    topology.finish().unwrap();
    let bytes = descriptor
        .descriptor_fingerprint_input()
        .fingerprint()
        .to_bytes();
    std::fs::write(std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("expected.rs"),
        format!("const EXPECTED: boomerang::runtime::binding::DescriptorFingerprint = boomerang::runtime::binding::DescriptorFingerprint::new({bytes:?});")).unwrap();
    println!("cargo:rerun-if-changed=declaration.rs");
}
