# Cargo Boomerang Launcher Support and Artifact Reporting

## Status and intent

This document defines the intended architecture for host-launcher support and
human-readable executable reporting in `cargo-boomerang`. It is normative for
this change: implementation details may evolve, but they must preserve the
ownership boundaries and observable behavior stated here.

The change has two related goals:

1. stop generating copies of host-only tracing and execution-summary code in
   every launcher; and
2. tell a human which stable executable was published and which exact
   executable is being run, without changing machine-readable CLI output.

## Boundary principles

- `boomerang_runtime` owns execution semantics and `FederateExecution`, not
  host process policy, filesystem protocols, or tracing-subscriber setup.
- A configured runtime backend owns the capabilities of the launcher it
  produces. Capabilities must not be inferred from the Rust target triple.
- Host-only facilities must be absent from launchers for targets that do not
  provide a filesystem, process environment, terminal, or
  `tracing-subscriber`.
- Generated code should describe the application image and connect selected
  capabilities. Reusable handwritten behavior belongs in a library.
- Human-readable progress goes to stderr through `CommandOutput`. Stdout
  remains reserved for stable machine-readable command results.

## Runtime-backend capabilities

Launcher generation shall resolve an internal capability description from the
federate's configured runtime backend. The current `std` backend selects hosted
launcher support. Future embedded or otherwise constrained backends may select
no host support or a different support implementation.

The capability description is an internal `cargo-boomerang` concern, initially
represented by a closed choice between hosted support and no host support. It
shall determine at least:

- which support dependency, if any, is added to the generated manifest;
- which initialization hook, if any, is called before execution; and
- which completion-reporting hook, if any, receives the resulting
  `FederateExecution`.

The backend selection is authoritative. `cargo-boomerang` must not inspect a
target triple and guess that a filesystem, environment variables, stderr, or a
tracing subscriber is available. Unsupported combinations shall fail during
analysis or launcher generation with an error naming the federate, selected
backend, and missing capability.

This design does not require a public extension API for third-party runtime
backends. A small closed internal representation is sufficient until more than
the hosted backend is implemented.

For hosted launchers, workspace resolution shall locate the `boomerang_util`
package through the same resolved, lockfile-backed package graph used for
generated runtime dependencies. Its package identity, `launcher` feature, and
manifest entry become ordinary generated-workspace inputs, so changing the
support dependency invalidates the generated cache without a separate cache
mechanism.

## Hosted launcher support

The existing `boomerang_util` workspace crate shall own reusable policy for
generated hosted launchers under a `launcher` feature and `launcher` module.
This evolves the crate's existing host-oriented runner and tracing role instead
of introducing another small support crate. It remains deliberately separate
from `boomerang_runtime`: depending on the runtime must not imply a filesystem,
environment variables, terminal detection, JSON summary transport, or
`tracing-subscriber`.

The `launcher` feature shall depend directly on `boomerang_runtime` and
`tracing-subscriber`. It must not enable `boomerang_util`'s legacy `runner` or
`replay` features, or their dependency on the high-level `boomerang` facade.
This keeps the generated launcher dependency graph limited to the facilities
the hosted launcher actually uses.

The module shall expose a small, documented API for:

- best-effort installation of the default hosted tracing subscriber; and
- optional emission of the versioned execution summary from a borrowed
  `FederateExecution`.

Tracing initialization shall retain current behavior: use the environment
filter, default to tracing off, write to stderr, respect terminal and
`NO_COLOR` detection, and tolerate a subscriber that was already installed.

Execution-summary emission shall retain the existing private protocol:

- no file is created unless `BOOMERANG_EXECUTION_SUMMARY_V1` is present;
- the file is created without overwriting an existing path;
- schema version 1 embeds `Stats` through its serde representation, while
  final-tag values remain decimal strings so the full logical-time range is
  representable; and
- write failures fail the launcher, because a supervising
  `cargo-boomerang run` requested the result.

`FederateExecution` remains the only source of execution statistics. The
support crate serializes a host-launch protocol view; it does not redefine or
own runtime statistics.

The generated hosted `main` shall therefore contain only the application
wiring and calls equivalent to:

1. initialize hosted launcher support;
2. construct generated bindings;
3. call `execute_owned_federate`;
4. pass `&FederateExecution` to the optional hosted summary writer; and
5. return success.

The generated manifest shall depend on `boomerang_util` with only its
`launcher` feature instead of depending directly on `tracing-subscriber`.
Non-hosted launchers shall not enable that feature and shall not contain the
summary environment constant, filesystem operations, or tracing initialization
calls.

## Data flow

The build and execution flow is:

```text
ResolvedDeployment
  -> compiler lowering
OwnedCompiledDeployment
  -> federate slice rendered by cargo-boomerang
generated launcher source + backend-selected support dependency
  -> Cargo build
verified launcher executable
  -> immutable bundle publication
stable published executable
  -> cargo-boomerang run copies and re-verifies it
invocation-private executable
  -> launcher executes OwnedCompiledDeployment
FederateExecution
  -> optional hosted summary file
cargo-boomerang validates and returns ExecutionSummary
```

The compiler owns lowering into `OwnedCompiledDeployment` and its typed runtime
image. `cargo-boomerang` renders that already-lowered image; launcher-support
selection must not move semantic lowering or runtime key allocation into code
generation.

The summary file is an out-of-band protocol between a hosted generated process
and its supervising `cargo-boomerang run` process. It is not part of the
compiled deployment image, runtime semantics, or stdout contract.

## Build path reporting

`cargo boomerang build` shall report paths only after immutable bundle
publication succeeds. Temporary Cargo target paths and staging paths are
implementation details and must not be presented as the build result.

For each federate artifact recorded in the published deployment document, the
command shall emit one deterministic Cargo-style stderr status line containing
the federate identity and the stable published executable path. Ordering shall
match the canonical federate order used by the published document. The
published `deployment.json` path remains the sole stdout result.

Path reporting shall use the existing `CommandOutput` policy so that:

- `--quiet` suppresses it;
- color follows the existing CLI and Cargo color policy;
- nested Cargo diagnostics retain their existing ordering; and
- library entry points using the silent output policy do not acquire direct
  terminal output.

The report describes a published build artifact, not an assertion that the
artifact can execute on the host. Cross-compiled and embedded artifacts may be
reported even though `cargo boomerang run` cannot launch them locally.

## Run path reporting

`cargo boomerang run` verifies the selected published artifact, copies it into
an invocation-private directory, and executes the copy. Immediately before
process creation, it shall emit a Cargo-style stderr status line containing the
exact copied executable path passed to the operating system. This replaces a
generic deployment-only running message as the authoritative execution-path
report; the deployment and federate identities may remain as context.

The reported run path is intentionally ephemeral. It answers "what executable
did this invocation execute?" while build output answers "what stable
executable did publication produce?"

The line shall be emitted only after copy and verification succeed and before
the child can produce output, preserving causal stderr ordering. It shall obey
the same quiet, verbosity, and color rules as all other `CommandOutput` status
lines. Launcher stdout and stderr remain child-owned and continue to flow
through the existing run result handling.

## Errors and protocol validation

Moving the writer must not weaken the supervisor's validation. The host-side
reader in `cargo-boomerang` shall continue to reject symlinks and non-regular
files, oversized documents, unknown fields, unsupported schema versions,
malformed counter types or final-tag decimals, and values outside host numeric
ranges.

The environment-variable name and schema-v1 document shape form a private
versioned contract shared by `boomerang_util::launcher` and `cargo-boomerang`.
The launcher module shall export the environment-variable constant for the
supervisor to use. The writer and defensive reader remain in their owning
layers, with an existing generated-toolchain execution test serving as their
schema compatibility test. The protocol must remain private; it is not a
general runtime serialization API.

Failures must retain context that distinguishes launcher execution failure,
summary production failure, and summary validation failure. Path-reporting
errors are ordinary stderr write failures and shall propagate through the
existing `CommandOutput` error path.

## Verification strategy

Verification should extend existing seams rather than create parallel test
infrastructure:

- extend generated-source and generated-manifest assertions to prove hosted
  launchers call `boomerang_util::launcher`, no longer contain handwritten
  tracing or filesystem summary implementations, and no longer depend directly
  on `tracing-subscriber`;
- exercise hosted summary emission through the existing generated-toolchain
  execution path, preserving current schema and execution-statistics checks;
- retain and extend the existing summary-reader security and malformed-input
  tests;
- extend `build_reports_cargo_style_progress_without_polluting_stdout` to
  assert stable published executable paths, canonical ordering, stderr-only
  output, quiet behavior, and the unchanged stdout result;
- extend the existing run/toolchain seam to assert that stderr names the exact
  invocation-private executable before child output; and
- use existing backend-validation tests to show that launcher capabilities
  follow backend selection and are not inferred from target triples.

The final verification gate includes formatting, focused `cargo-boomerang`
tests, generated-toolchain tests in their existing locked/offline mode, and the
full workspace test suite. Feature lanes that are mutually exclusive remain
separate rather than using workspace-wide `--all-features`.

## Legacy `boomerang_util` lifecycle

The hosted launcher module is the intended successor to the older
`boomerang_util` execution conveniences, not an unrelated API that should grow
alongside them indefinitely.

`runner::build_and_run_reactor` currently combines command-line parsing,
builder-side assembly and lowering, diagram generation, replay setup, and
runtime execution in one process. Generated launchers replace that production
architecture: lowering happens before code generation, and the generated
program executes an `OwnedCompiledDeployment`. New production examples and
documentation shall use `cargo-boomerang` and generated launchers rather than
the legacy runner.

The older APIs shall migrate according to their actual role:

- the legacy `test-tracing` feature shall enable `launcher`, and
  `test_tracing::init_with_directive` shall delegate to the canonical launcher
  tracing implementation before becoming a deprecated compatibility path once
  its current test consumers use the launcher API;
- `runner::build_and_test_reactor` may remain temporarily as an existing fast
  unit/integration-test seam, but test support shall ultimately have an
  explicitly test-oriented home rather than preserving the production runner
  abstraction; and
- `runner::build_and_run_reactor`, the `runner` module, and runner-coupled
  replay CLI behavior shall be deprecated once equivalent generated-launcher
  or `cargo-boomerang` workflows exist and repository examples have migrated.

Deprecation shall follow a demonstrated replacement, not force tests through a
slower generated-toolchain path or silently discard replay and diagram
capabilities. The current change establishes the canonical replacement and
shares its tracing implementation with the legacy helper, but broad runner
migration and removal remain a follow-up change.

## Implementation slices

The implementation should land as two reviewable slices on the same branch:

1. add backend-selected hosted launcher support, migrate generated launchers,
   preserve the execution-summary protocol, and share the canonical tracing
   implementation with the legacy test helper; then
2. add stable build-artifact and exact run-executable stderr reporting through
   `CommandOutput`.

The first slice establishes capability ownership. The second consumes existing
bundle and run boundaries and does not change launcher semantics.

## Non-goals

- implementing an embedded runtime backend or embedded launcher;
- inferring facilities from target triples;
- adding filesystem or tracing requirements to `boomerang_runtime`;
- making the execution-summary protocol public or stable for external users;
- changing `FederateExecution`, execution statistics, or compiled-image
  representations;
- changing bundle fingerprints, artifact verification, or stdout schemas;
- redesigning distributed launcher injection or transport selection;
- immediately removing the legacy runner or forcing existing runtime tests
  through generated Cargo projects; and
- dropping replay or diagram behavior before replacement workflows exist.
