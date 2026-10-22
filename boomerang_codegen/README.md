# boomerang_codegen

`boomerang_codegen` is a host compiler for explicitly selected Boomerang components.
It uses `boomerang_builder` for semantic validation and lowering, then generates Rust
scheduler tables and typed payload bindings. It does not discover packages, invoke
Cargo, replace `main`, or start the runtime.

The default feature set is empty. Enable `cargo-build` only in a build dependency
when using the `OUT_DIR` publisher. The target application needs `boomerang_runtime`
and `boomerang_tinymap` (named `tinymap`), plus its selected component payloads. It
does not need the builder or codegen crate as normal dependencies.

## Plain Cargo application

The complete runnable example is [tests/fixtures/native](tests/fixtures/native).
It has a checked-in manifest, `build.rs`, shared component declaration, and `main`:

```sh
cargo run --manifest-path boomerang_codegen/tests/fixtures/native/Cargo.toml --locked --offline
```

The package enables `payload` by default. Its build script explicitly emits metadata
with `boomerang::component_metadata!`; its target explicitly emits payload with
`boomerang::component_payload!`. A shared declarative macro supplies the declaration
to both entry points. Helpers and reaction bodies stay inside that declaration so
metadata emission can omit them. Imports, attributes, and dependencies outside the
declaration must also respect host/target isolation. A self-contained application
must not list itself as a build dependency.

For component dependencies, select metadata features in build dependencies and
payload features in normal dependencies, with `default-features = false` where
needed. Use Cargo resolver 2 or newer. Cargo aliases are Rust path spellings;
`ImplementationId` is the independently selected semantic identity.

The essential build-script calls are:

```rust,ignore
use boomerang_codegen::{cargo_build::CargoBuild, Compilation, PayloadExports};

let mut cargo = CargoBuild::from_env()?;
cargo.track_file("declaration.rs")?;
cargo.track_file("topology.rs")?;
cargo.track_env("APP_VARIANT")?;
let mut compilation = Compilation::new(
    topology(), deployment_configuration(cargo.target().triple()),
    selected_federate, cargo.target().clone(),
);
compilation.register(
    implementation.clone(),
    controller::__boomerang::metadata::descriptor(),
    PayloadExports::new("crate::controller::__boomerang::payload")?,
)?;
compilation.select(first_instance, implementation.clone())?;
compilation.select(second_instance, implementation)?;
cargo.compile(&compilation, &mut std::io::stdout())?;
```

`DeploymentConfiguration` explicitly supplies a name, `PlacementAssignment`s,
`FederateConfig`s, `CoordinationSelection`, and `BoundaryBinding`s. The configured
target of the selected federate must match `cargo.target().triple()`. There is no
host-target fallback. Construct the compilation using the complete captured target
context, including `CARGO_CFG_*` facts.

The application's entry point owns execution:

```rust,ignore
mod generated { include!(concat!(env!("OUT_DIR"), "/boomerang.rs")); }
fn main() -> Result<(), Box<dyn std::error::Error>> {
    boomerang_runtime::execute_owned_federate_slice_with_observations(
        generated::FEDERATE,
        &generated::FEDERATE_VIEW,
        generated::generated_bindings(),
        boomerang_runtime::Config::default().with_fast_forward(true),
        &[],
    )?;
    Ok(())
}
```

Generated definitions are immutable tables and a callable binding constructor.
Including them has no execution side effects. Descriptor fingerprint and macro ABI
assertions use independently generated metadata and the actual payload's
`binding_manifest()`. Invalid Rust paths fail during host generation; missing target
exports and incompatible payloads fail during target compilation.

## Pure compilation and explicit publication

`Compilation::compile(&self) -> Result<CompilationOutput, CompilationError>` returns
an owned validated deployment image and the selected federate's source. It does not
read environment variables or touch files. Registration order does not affect output.
Multiple instances may share a registration; each instance must explicitly select
an implementation. Duplicate registrations/selections fail without replacing the
original entry. Builder contract and version validation remains authoritative.

`DeploymentInputs::lower()` is the shared entry point for callers that already have
`ImplementationBinding`s, including CLI discovery. The public `render` module consumes
builder images, a map keyed by `ImplementationId`, and explicit `BindingConfiguration`.
`PayloadExports::with_manifest` supports the remaining CLI legacy export convention;
Cargo-native callers use `PayloadExports::new`.

`BindingConfiguration` maps component instances to installed physical source keys and
records whether the application installs a manual clock. Applications must install the
matching runtime physical-input configuration and clock before execution. For a
distributed deployment, the generated binding constructor accepts RTI client bindings
and a sink and returns a `Result`. The application or CLI owns transport, coordination
images, RTI process startup, and runtime execution; the compiler does not start them.

Diagnostics expose a stage, code, deployment, relevant component/implementation or
boundary, and original builder errors through `std::error::Error::source()`.

## Cargo adapter ownership

`CargoBuild::from_env()` requires `TARGET` and `OUT_DIR`; it captures `CARGO_CFG_*`.
`from_values` and `from_context` provide deterministic dependency injection without
changing process-global environment. Declare every file/directory and environment
value used by your topology through `track_file` and `track_env`; these emit Cargo
rerun directives alongside target inputs. Source dependencies still use Cargo's normal
Rust dependency tracking. Directive inputs containing newlines or NUL are rejected.

The adapter owns only `OUT_DIR/boomerang.rs` and its temporary publication files. It
writes a complete temporary file in the same directory and atomically replaces the
final file. Identical bytes preserve modification time. Failed compilation invalidates
previous success output. Unrelated files and interrupted non-final temporary files
are preserved. There is no configurable output filename, nested Cargo invocation, or
private compilation cache.
