use super::support;

/// Exercises package ownership, named selection, separate host modes and typed target delivery.
#[test]
fn topology_imports_two_named_components_from_the_selected_payload_package() {
    let _guard = support::toolchain_lock();
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path();
    let boomerang = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("boomerang");
    std::fs::create_dir_all(root.join("components/src")).unwrap();
    std::fs::create_dir_all(root.join("topology/src")).unwrap();
    std::fs::create_dir_all(root.join(".cargo")).unwrap();
    std::fs::write(
        root.join(".cargo/config.toml"),
        "[build]\nrustflags = [\"--cfg\", \"component_configured_flag\"]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"
[workspace]
members = ["components", "topology"]
resolver = "2"
[workspace.dependencies]
boomerang = {{ path = {:?} }}
"#,
            boomerang.to_str().unwrap()
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("components/Cargo.toml"),
        r#"
[package]
name = "named-components"
version = "0.0.0"
edition = "2021"
[dependencies]
boomerang.workspace = true
"#,
    )
    .unwrap();
    std::fs::write(root.join("components/src/lib.rs"), r#"
pub mod nodes {
    boomerang::component! {
        pub mod r#source {
            use boomerang::prelude::*;
            #[reactor(contract = "example.source", contract_version = 1,
                bounds(queue_capacity = 16, payload_bytes = 2048, state_bytes = 1024, scratch_bytes = 1024))]
            pub fn Node(#[output] value: u32) -> impl Reactor {
                reaction! { Start(startup) -> value { *value = Some(42); } }
            }
        }
    }
    // Same contract/root as the selected source: this sibling must compile
    // without demanding a descriptor fingerprint for an unused component.
    boomerang::component! {
        pub mod unused {
            use boomerang::prelude::*;
            #[reactor(contract = "example.source", contract_version = 1,
                bounds(queue_capacity = 16, payload_bytes = 2048, state_bytes = 1024, scratch_bytes = 1024))]
            pub fn Node(#[output] value: u32) -> impl Reactor {
                reaction! { Start(startup) -> value { *value = Some(99); } }
            }
        }
    }
    boomerang::component! {
        pub mod sink {
            use boomerang::prelude::*;
            #[reactor(contract = "example.sink", contract_version = 1,
                bounds(queue_capacity = 16, payload_bytes = 2048, state_bytes = 1024, scratch_bytes = 1024))]
            pub fn Node(#[input] value: u32) -> impl Reactor {
                reaction! { Receive(value) {
                    assert_eq!(*value, Some(42));
                    println!("named component received 42");
                    ctx.schedule_shutdown(None);
                } }
            }
        }
    }
}
"#).unwrap();
    std::fs::write(
        root.join("topology/Cargo.toml"),
        r#"
[package]
name = "named-topology"
version = "0.0.0"
edition = "2021"
[dependencies]
boomerang.workspace = true
named-components = { path = "../components" }
"#,
    )
    .unwrap();
    std::fs::write(root.join("topology/src/lib.rs"), r#"
#![allow(unexpected_cfgs)]
#[cfg(not(any(component_configured_flag, component_encoded_flag)))]
compile_error!("the generated compiler wrapper must preserve Cargo's effective flags");
use boomerang::prelude::*;
pub fn topology() -> Result<boomerang::builder::compiler::ApplicationTopology, boomerang::builder::compiler::TopologyBuildError> {
    let mut assembly = Assembly::new();
    let source = named_components::nodes::source::Node().build("source", (), None, None, None, false, &mut assembly).unwrap();
    let sink = named_components::nodes::sink::Node().build("sink", (), None, None, None, false, &mut assembly).unwrap();
    assembly.add_port_connection::<u32, _, _>(source.value, sink.value, None, false).unwrap();
    Ok(assembly.application_topology().unwrap())
}
"#).unwrap();
    std::fs::write(
        root.join("Boomerang.toml"),
        r#"
schema = 1
[topology]
package = "named-topology"
entry = "named_topology::topology"
[deployments.demo]
bindings.source = { package = "named-components", component = "nodes::source" }
bindings.sink = { package = "named-components", component = "nodes :: r#sink" }
federates.host = { groups = ["placement/source"], runtime = "std", recovery = "fail-stop" }
"#,
    )
    .unwrap();
    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    let target = support::toolchain_target().join("named-components");
    support::with_target_directory(&target, || {
        let bundle = cargo_boomerang::build(root, "demo").unwrap();
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&bundle).unwrap()).unwrap();
        let bindings = document["bindings"].as_array().unwrap();
        assert_eq!(bindings.len(), 2);
        for binding in bindings {
            assert_eq!(binding["package"]["name"], "named-components");
            assert_eq!(binding["descriptor"]["package"], "named-components");
            assert!(binding["descriptor"]["module"]
                .as_str()
                .unwrap()
                .contains("nodes"));
        }
        let executable = bundle
            .parent()
            .unwrap()
            .join(document["artifacts"][0]["path"].as_str().unwrap());
        let output = std::process::Command::new(executable).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8(output.stdout)
            .unwrap()
            .contains("named component received 42"));
    });
    // Encoded flags override configured flags according to Cargo's own precedence;
    // the tool still adds descriptor mode after that resolution.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "--workspace"])
        .arg(root)
        .args(["check", "--deployment", "demo"])
        .env("CARGO_TARGET_DIR", &target)
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            "--cfg\u{1f}component_encoded_flag",
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
