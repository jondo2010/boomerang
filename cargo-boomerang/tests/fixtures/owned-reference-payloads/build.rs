use std::{env, fs, path::Path};

use boomerang_runtime::binding::{
    payload_fingerprint_compile_input_key, COMPONENT_DESCRIPTOR_MACRO_ABI,
    PAYLOAD_MACRO_ABI_COMPILE_INPUT,
};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(boomerang_cargo_config_probe)");
    println!(
        "cargo:rustc-check-cfg=cfg(feature, values(\"__boomerang_descriptor\", \
         \"broken-descriptor\", \"broken-payload\", \"profile-config-probe\", \
         \"runtime-failure\", \"warning-diagnostic\"))"
    );
    let manifest_dir = fs::canonicalize(env::var_os("CARGO_MANIFEST_DIR").unwrap()).unwrap();
    let manifest_dir = manifest_dir.to_str().unwrap();
    println!("cargo:rustc-env={PAYLOAD_MACRO_ABI_COMPILE_INPUT}={COMPONENT_DESCRIPTOR_MACRO_ABI}");
    for (contract, reactor, fingerprint) in [
        (
            "vehicle.controller",
            "Controller",
            "5d653759745efef472762f00cd6ecbf0654f58f4f98c5a71758ab413b374c42f",
        ),
        (
            "vehicle.sensor",
            "Sensor",
            "fd8927818c1f722adab20ec868c920758153f21fdb26f46781495c38dc2623a1",
        ),
    ] {
        println!(
            "cargo:rustc-env={}={fingerprint}",
            payload_fingerprint_compile_input_key(manifest_dir, contract, 1, reactor)
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
