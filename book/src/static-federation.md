# Static Federation

Federates are an unconditional part of Boomerang's compiled deployment model,
including a local deployment with one Federate. There is no `federated` feature
to enable. A Federate groups one or more Enclaves under a stable deployment
identity; Federates, Enclaves, processes, and hosts remain separate concepts.

Use `cargo boomerang` with a deployment manifest to generate, build, and run a
compiled deployment. The `central-rti` projection produces a separate RTI
executable and independently built Federate executables. The launcher waits for
readiness, supervises their execution, and bounds shutdown and child reaping.
The RTI coordinates logical tags and forwards encoded payloads over the hosted
transport. Generated images carry the shared coordination identity used to
validate membership before execution.

The compiler remains authoritative for membership, route analysis, and timing
constraints. Runtime preflight resolves stable identities to typed dense keys;
normal coordination and payload exchange use those keys. Transport or decoding
failure terminates execution rather than authorizing a logical tag.

The local projection does not need the central RTI dependency. Ordinary local
`Assembly` examples remain supported during migration, but Assembly no longer
constructs federations or exposes `execute_federation_*` runners. Public
compiled authoring improvements and a high-level hosted runner are tracked in
[#234](https://github.com/jondo2010/boomerang/issues/234) and
[#235](https://github.com/jondo2010/boomerang/issues/235); example migration is
tracked in [#138](https://github.com/jondo2010/boomerang/issues/138).

Compiled contracts live in the crates that implement them:

```sh
cargo test -p boomerang_runtime --test compiled_execution --offline
cargo test -p boomerang_central_rti --test compiled_execution --offline
cargo test -p cargo-boomerang --test toolchain run::generated_central_rti_exchanges_tagged_payload --offline -- --exact
```

The RTI crate also provides an in-memory transport for testing and reference
use. It is isolated from the generated deployment transport and is not the
intended production hot path.
