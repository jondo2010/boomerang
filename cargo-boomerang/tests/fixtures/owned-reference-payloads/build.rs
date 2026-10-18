use std::{env, fs, path::Path};

use boomerang_runtime::binding::{
    component_payload_fingerprint_compile_inputs_key, COMPONENT_DESCRIPTOR_MACRO_ABI,
    PAYLOAD_MACRO_ABI_COMPILE_INPUT,
};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(boomerang_cargo_config_probe)");
    println!("cargo:rustc-check-cfg=cfg(boomerang_facet, values(\"descriptor\", \"payload\"))");
    println!("cargo:rustc-cfg=boomerang_facet=\"payload\"");
    println!(
        "cargo:rustc-check-cfg=cfg(feature, values(\"broken-descriptor\", \"broken-payload\", \"profile-config-probe\", \
         \"natural-quiescence\", \"runtime-failure\", \"warning-diagnostic\"))"
    );
    let manifest_dir = fs::canonicalize(env::var_os("CARGO_MANIFEST_DIR").unwrap()).unwrap();
    let manifest_dir = manifest_dir.to_str().unwrap();
    println!("cargo:rustc-env={PAYLOAD_MACRO_ABI_COMPILE_INPUT}={COMPONENT_DESCRIPTOR_MACRO_ABI}");
    for (component, contract, reactor, fingerprint) in [
        (
            "controller",
            "vehicle.controller",
            "Controller",
            "acc9d9b0812169096b3e222048e5fb9bcacc97e5a6bcffe7f20dc7f38a8e5ba1",
        ),
        (
            "sensor",
            "vehicle.sensor",
            "Sensor",
            "60c2feb91f253dbe4898fa5bf092d8921ec8f20cede8c74c2edb9d9a47fe369b",
        ),
    ] {
        println!(
            "cargo:rustc-env={}=owned_reference_payloads::{component}={fingerprint}",
            component_payload_fingerprint_compile_inputs_key(manifest_dir, contract, 1, reactor)
        );
    }
    let fixtures = Path::new(manifest_dir).join("../workspace");
    println!(
        "cargo:rerun-if-changed={}",
        fixtures.join("vehicle-control/src/lib.rs").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        fixtures.join("sensor-host/src/lib.rs").display()
    );
}
