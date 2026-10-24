# Cargo-native deployments

Boomerang applications use ordinary Cargo packages. Each application owns its
manifest, build script, and entry point. Cargo resolves dependencies and builds
the executable. Application builds do not require `cargo-boomerang`.

## Build a local application

Run the physical-input example from the repository root:

```sh
cargo run -p physical-input
```

Its build script expands metadata from the shared component declaration.
Its target entry point expands payload from the same declaration.
The build script generates Rust definitions beneath Cargo's `OUT_DIR`.
The entry point includes those definitions and starts the runtime explicitly.

See [physical-input](../examples/physical-input/README.md) for the application workflow.
Read `boomerang_codegen` crate rustdoc for compiler and build-script APIs.

## Build a deployment with multiple executables

Each Federate and the central RTI use a checked-in application package.
Build packages for their required targets and toolchains with Cargo.
Shared host declarations supply consistent topology and deployment configuration.
Each executable embeds its compiler-produced coordination identity.
Admission rejects members from incompatible deployments.

The [native federation fixture](../boomerang_codegen/tests/fixtures/federated/README.md)
contains independently buildable controller, sensor, and RTI packages.
Its process integration test starts the RTI, connects the members, and checks
payload exchange and failure handling:

```sh
bash boomerang_codegen/tests/run-applications.sh
```

Applications or external process tools own startup and shutdown.
The runtime crates provide transport and coordination APIs.
They do not build or launch other application packages.

## Optional CLI

`cargo-boomerang` provides the optional `monitor` command when built with its
`monitor` feature. It receives telemetry from an existing application.
The tool has no `build`, `check`, or `run` command.
It does not consume `Boomerang.toml`, generate Cargo workspaces, install compiler
wrappers, or maintain an application compilation cache.

Deployment scaffolding and artifact packaging are separate future work.

## Efficient centralized coordination

The central RTI must preserve payload admission, grant ordering, cumulative
completion, and bounded progress under delayed reports.
The normative contract is in the
[`boomerang_central_rti::compiled` module rustdoc](../boomerang_central_rti/src/compiled/mod.rs).
That contract includes positive-delay cycles and excludes unproved zero-delay optimizations.
