# Static Federate Deployment Architecture

This document defines normative deployment architecture for Cargo-native Boomerang
applications. It specializes the [logical graph architecture](./architecture.md).
Crate rustdoc defines API details. The [deployment roadmap](./federated-deployment-roadmap.md)
defines delivery order.

The hosted Cargo-native migration implements the build model below. Constrained
execution and deployment-independent boundary replay remain architecture requirements,
not completed features. Bare-metal acceptance remains deferred to
[#135](https://github.com/jondo2010/boomerang/issues/135),
[#136](https://github.com/jondo2010/boomerang/issues/136), and
[#137](https://github.com/jondo2010/boomerang/issues/137).

## Decision summary

- Every compiled application contains at least one Federate.
- One Federate produces one application binary. Local execution requires no RTI.
- Multiple Federates use independent application packages and explicitly select a coordination backend.
- The `central-rti` backend requires one additional RTI application. A future peer-to-peer backend requires no RTI artifact.
- Source declares logical component instances, Enclaves, placement groups, contracts, and constraints.
- Host compilation resolves implementation selection and placement before global analysis and lowering.
- Global analysis sees the complete resolved application. Target dependencies contain only the selected local payloads and required runtime support.
- Production artifacts execute pre-lowered images. Startup must not construct the application graph or repeat lowering.
- Generated bindings reference typed target symbols. Runtime component discovery and symbol lookup are excluded.
- Cargo owns dependency resolution and target compilation. Application entry points own runtime startup and shutdown.

## Terminology and hierarchy

| Concept | Meaning |
| --- | --- |
| Logical component instance | A stable application unit with an external contract and one selected implementation. |
| Placement group | A source-declared unit of placement with stable identity and explicit constraints. |
| Enclave | An independently scheduled logical-time domain inside a Federate. |
| Federate | A stable participant identity with its own application lifecycle and executable. |
| Deployment unit | An executable and process or protection-domain instance. Initially, one unit corresponds to one Federate. |
| Compute node | An external host, board, ECU, or SoC. One node can run several Federates. |
| Execution resource | A CPU, core, accelerator, or similar resource. Its assignment is separate from Federate membership. |

Enclave boundaries define scheduler ownership and logical-time semantics. They do
not establish process, memory, security, safety, or mixed-criticality isolation.
Those properties require explicit platform guarantees, resource budgets, timing
evidence, failure semantics, and qualification.

Topology, implementation bindings, and Federate membership remain static for each
compiled deployment. Runtime topology mutation, implementation selection, and plugin
loading are excluded.

Placement constraints include co-location, separation, capabilities, and resource
requirements. Deployment assigns existing placement groups. It must not silently create or split
Enclaves. Co-located Enclaves remain separate scheduler domains. Alternative
implementations must not silently change the externally placeable group surface.

## Architectural representations

### `ApplicationTopology`

`ApplicationTopology` contains target-neutral structure, contracts, stable identities,
relationships, Enclaves, placement groups, boundaries, and binding slots. It contains
no runtime state, reaction closures, channels, protocol sessions, or allocation keys.

Semantic records and relationships use typed stable identities. Canonical ordering
must make topology equality, inspection, and fingerprint inputs independent of
input iteration or package-discovery order.

Nested relationships also require canonical order. Action relations precede port
relations. Each category preserves declaration positions and stable target order.
Mode memberships use stable mode order. Duplicate memberships are invalid.

Graph algorithms can use private temporary dense indexes. Those indexes must not
escape into semantic models or become durable identities. Structural inspection
belongs to `ApplicationTopology`. Mutable queues, aliases, and scheduler diagnostics
remain runtime-owned.

### `ResolvedDeployment`

Resolution combines topology with explicit implementation, placement, target,
runtime, coordination, codec, transport, and policy selections.

Every component instance has exactly one implementation. Every placement group
belongs to exactly one Federate. All source constraints must hold. No implementation
or placement choice remains unresolved after this phase.

### `CompiledDeployment`

Lowering produces immutable scheduler images, required payload bindings, storage
requirements, the backend-neutral federation graph, and a selected coordination
projection. `RtiImage` is the central backend projection, not the canonical graph.

Permanent typed dense keys belong only to compiled images and runtime storage.
Lowering assigns them after stable-ID canonicalization and deployment slicing.
Each key remains local to its declared image domain.

The normative representation boundaries are in
[`boomerang_builder::compiler`](../boomerang_builder/src/compiler/mod.rs) and
[`boomerang_runtime::image`](../boomerang_runtime/src/image/mod.rs).

## Component contracts and package boundaries

A component contract identifies its version, external ports, payload schemas,
timing semantics, recordable boundaries, capabilities, and required resource bounds.
Alternative implementations can differ internally. They must satisfy the same
external contract at the selected logical component site.

The Cargo package is the minimum strict-slicing unit. Independently placed
implementations for incompatible targets require separate packages. Module-level
dependency slicing inside one package is not an architecture guarantee.

One component declaration supplies metadata and payload APIs. Metadata excludes
state constructors, reaction bodies, driver objects, HAL access, and implementation
helpers. Target-only dependencies must also remain outside the host dependency graph.
Structural signature types must be available on the host.

Metadata and payload features are additive. Explicit `component_metadata!` and
`component_payload!` entry points select output independently of package features.
Direct reactor attributes retain the dynamic construction API. `boomerang/builder`
controls builder dependencies; it does not select macro output.

Metadata supplies stable slots, topology relationships, declared storage bounds,
a structural fingerprint, and a macro ABI version. Payload supplies concrete typed
initializers, reaction functions, and the matching compatibility header.

Generated bindings must check metadata fingerprints and macro ABI versions against
the target payload. Rust compilation must check state, reference, payload, and
codec types. Missing exports and incompatible bindings are build errors.

Several logical instances can reuse one implementation's function symbols. Each
instance must retain distinct state and action storage. Stable implementation slots
remain distinct from instance-qualified logical identities.

Payload visibility, custom state constructors, export naming, and supported syntax
are specified in [`boomerang_macros` rustdoc](../boomerang_macros/src/lib.rs).

## Cargo build ownership

Each checked-in application package owns its manifest, build script, and entry
point. A build script supplies topology, deployment configuration, target facts,
implementation registrations, and selections directly to `boomerang_codegen`.
A separate topology package is optional.

A single-package application shares declarations through ordinary Rust source or
a declarative macro. It must not depend on itself as a build dependency.
Multi-package applications consume public component APIs through Cargo dependencies.

Build dependencies select metadata. Normal dependencies select payloads. Cargo
resolver 2 or newer isolates these dependency roles. Dependencies must disable
default features when those defaults introduce unwanted implementation code.

Target triples, toolchains, profiles, linkers, linker scripts, and runners use
ordinary Cargo configuration. Independent target builds use independent Cargo
invocations. Feature unification within each invocation follows Cargo rules.
Cargo features must not substitute for explicit placement or coordination policy.

Topology-affecting files and environment values are explicit tracked build inputs.
The compiler must not silently substitute the host target for the selected target.
Shared deployment inputs must produce compatible coordination tables across
independently built members.

The pure compiler performs no Cargo discovery, environment access, filesystem I/O,
or process execution. Its Cargo adapter publishes generated Rust beneath `OUT_DIR`.
Publication must not expose incomplete source or preserve stale output after failure.
Identical output preserves the published file's modification time.

Topology and capability assumptions are compiler inputs. Addresses, credentials,
secrets, and mutable host assignments remain deployment or runtime configuration.

The API and publication contract is in
[`boomerang_codegen` rustdoc](../boomerang_codegen/src/lib.rs).

## Global analysis and lowering

Global analysis must validate the complete deployment before producing executable
slices. It covers stable identities, contracts, placement coverage, constraints,
Enclave dependencies, reaction relationships, modes, schemas, capabilities, resource
bounds, and supported distributed cycles.

Federation analysis derives direct routes, transitive dependencies, minimum-delay
paths, and affected downstream sets. Backend projection consumes this same canonical
analysis. It must not introduce another graph-analysis path.

| Relationship | Lowered delivery |
| --- | --- |
| Same Enclave | Direct local binding. |
| Different Enclaves in one Federate | In-process scheduler event route. |
| Different Federates | Serialized endpoint with selected coordination support. |
| Recordable boundary | The selected route plus a scheduler admission observation point. |
| Replayed implementation | A trace source that satisfies the same logical boundary contract. |

Direct reaction relations remain Enclave-local. Cross-Enclave dependencies require
explicit connections. Each lowered scheduler boundary has matching inbound and
outbound routes with the same stable identity, timing domain, and delay.

Same-Federate routes require no federation codec unless serialized recording needs
one. Cross-Federate routes require compatible codec and transport capabilities.
Recordable local routes must expose an equivalent admission point. An adapter must
preserve tags and ordering without an observable scheduling change.

Unsupported semantics must fail at the earliest phase with sufficient information.
Errors must identify the relevant stable entity and source context when available.
Target type and layout errors remain target-compilation checks.

## Fingerprints and determinism

Distinct identities make distinct compatibility claims:

| Identity | Required meaning |
| --- | --- |
| Descriptor fingerprint and macro ABI | Metadata and target payload describe the same component structure and binding interface. |
| Deployment provenance | Canonical topology, implementation selections, placements, targets, policies, compiler versions, and compiled images identify the deployment inputs. |
| Coordination fingerprint | Shared contracts, protocol and codec versions, canonical wire mappings, dependency projection, and policies agree before admission. |
| Federate-image fingerprint | The member's local scheduler image, bindings, and storage requirements agree. Different members normally have different values. |
| Artifact digest | The exact executable bytes agree. This is separate from shared peer compatibility. |
| Boundary-contract fingerprint | Stable boundary identity, direction, schema, encoding, timing, and ordering agree across recording and replay deployments. |

Coordination admission must reject mismatched identities before accepting dense
references or payloads. Local implementation changes alone must not redefine shared
compatibility. Schema changes require the appropriate contract version change.

Dense keys are not durable identities. Canonical wire table positions are explicit
schema inputs to the coordination identity. Local scheduler keys remain local.
Dense key types must not implement serialization traits. Schemas represent canonical
table positions through explicit integer fields. A fingerprint establishes
consistency, not authentication or authorization.

Identical explicit compiler inputs must produce canonical compiled images.
This guarantee does not promise identical executable bytes across arbitrary toolchains.
Future artifact packaging must produce a whole-deployment provenance identity for
locked Cargo inputs, selected features, toolchains, and artifact digests. It must
retain target, runtime, protocol, capability, resource, and required configuration
metadata without credentials or mutable host assignments. This migration publishes
no deployment bundle. Future packaging must publish only complete validated
artifact sets. Failed builds must not expose a new complete deployment.

Logical scheduling must remain deterministic for the same initial state and input
stream, subject to declared physical-input and reaction-code assumptions. Alternative
implementations are not automatically behaviorally equivalent.

## Compiled images and execution

Images contain immutable scheduler tables, dependency levels, modes, timer semantics,
shutdown actions, routes, binding slots, and resource requirements. Generated Rust
embeds these tables directly. Targets must not deserialize a topology or lower a
graph at startup.

Runtime image validation must check references, ownership, ranges, canonical order,
and cross-table invariants before execution. Concrete representation rules belong
in [`boomerang_runtime::image`](../boomerang_runtime/src/image/mod.rs).

Platform storage, queues, scratch buffers, clocks, wake sources, and coordination
adapters must preserve the same scheduler semantics. Hosted execution permits
heap-backed storage without adding the graph compiler to production dependencies.

The owned host executor and generated applications consume the same image schema
and scheduler algorithm. Reference transports support testing; they do not replace
independent artifact and process execution evidence.

The ordinary local `Assembly` path remains during caller migration. It does not
define a second long-term compiled architecture. Its removal requires a separate
caller migration; this Cargo-native change does not complete that removal.

Each Federate owns its Enclaves, routes, coordination participation, and lifecycle.
Every active distributed Enclave contributes to its Federate-wide frontier.
Split-phase publication, acquisition, fixed-point completion, and failure supervision
remain required. Same-Federate traffic does not traverse a federation backend.

## Coordination backend extensibility

A multi-Federate deployment selects a backend explicitly. Backend-specific target
packages and transports appear only where required. A one-Federate deployment
requires neither a central RTI dependency nor a distributed transport.

The central backend uses one persistent ordered connection per Federate.
Its immutable RTI projection must preserve the compiled participant layout.
An RTI application validates its borrowed projection without member scheduler
images or payload bindings.
Backend selection must not change topology, implementation selection, Federate
boundaries, or logical boundary contracts.

Peer-to-peer coordination remains future work. It must use compiled peer routes.
Runtime topology discovery and dynamic membership negotiation are excluded.
It must preserve declared logical-time guarantees. Unsupported graphs and target
capabilities must fail at build time.

### Efficient centralized coordination

The central RTI must preserve payload admission, grant ordering, cumulative
completion, and bounded progress under delayed reports. These rules apply outside
zero-delay cycles, including positive-delay cycles. Constructive zero-delay
coordination requires separate analysis and conformance evidence.

The complete normative contract is in
[`boomerang_central_rti::compiled`](../boomerang_central_rti/src/compiled/mod.rs).
It includes EIMT, DNET, NET, LTC, in-transit accounting, saturation, and fail-stop behavior.
Direct payload routing remains a separate optimization with its own safety proof.

The hosted framing, exact admission, FIFO, queue reservations, operation deadlines,
cancellation, and global-quiescence contract is in
[`boomerang_central_rti::compiled::hosted`](../boomerang_central_rti/src/compiled/hosted/mod.rs).
Generated-backend capability restrictions are specified in
[`boomerang_codegen`](../boomerang_codegen/src/lib.rs). Unsupported selections must
fail before source publication rather than silently relax policy.
Applications configure addresses and operation timeouts. External process tools
own readiness notification and process supervision.

## Recording and replay

This section retains architecture requirements for future boundary replay.
The existing async-event MCAP implementation does not establish these guarantees.

Recording must observe receiver-side scheduler admission after decoding and
physical-arrival normalization, before delivery to the target action queue.
It records admitted logical boundary events, not RTI control frames or runtime keys.
Recording must not change observable scheduling.

A container-neutral boundary event must contain stable boundary identity,
boundary-contract fingerprint, the complete logical tag including microstep,
a trace-wide admission sequence, and serialized payload. Paced replay also records
elapsed physical time. Deployment-local dense keys are excluded.

A replay implementation replaces the selected logical component while satisfying
its external boundary contract. It resolves stable boundary identities through the
new deployment tables. Injection uses the same scheduler admission semantics.
Boundary compatibility excludes placement and unrelated implementation internals.

Logical, paced, and fast-forward replay must preserve tags and ordering.
Paced replay additionally preserves recorded physical intervals. Unknown boundaries,
contract or schema mismatches, corrupt ordering, decoding errors, and events in the
scheduler's past must terminate replay with an error.

The planned `TraceSink` and `TraceSource` interfaces exchange boundary events
rather than container records. Scheduler trace interfaces must remain container-neutral. MCAP is the initial
hosted container, not a required scheduler representation. Its metadata must retain
boundary and schema identities. Its versioned event envelope must retain tags,
sequence, and payload. Embedded containers must preserve those semantics without
requiring MCAP messages, memory mapping, or summary records in core APIs.

The [logical replay architecture](./architecture.md#replay-and-partition-substitution)
adds substitution and feedback constraints. The roadmap defers this work to its
[optional hosted projections](./federated-deployment-roadmap.md#phase-12---optional-projections-hosted-services-and-optimization).

## Constrained execution requirements

Constrained backends must provide compile-time bounds for state, events, payload
buffers, and scheduler scratch. Target compilation must verify concrete layouts
against declared bounds. An unbounded requirement is a build error for a bounded backend.

Allocator-free execution must not require a global allocator or startup graph
construction. Hosted storage choices do not establish this property. Clock,
transport, recovery, timing, and security guarantees require their own platform evidence.

These requirements remain pending. The hosted acceptance result does not claim
bare-metal support or an early codegen diagnostic for every unsupported target.
The current bare-metal attempt fails because target dependencies require `std`.

## Crate and dependency boundaries

- `boomerang_macros` owns component emission. It does not own deployment policy or orchestration.
- `boomerang_builder::compiler` owns semantic models, validation, analysis, lowering, projections, and structural inspection.
- `boomerang_codegen` renders images and typed bindings. Its optional Cargo adapter publishes source.
- `boomerang_runtime` owns images, scheduler semantics, storage, clocks, admission, and generic coordination interfaces.
- `boomerang_federated` owns portable coordination wire and channel contracts.
- `boomerang_central_rti` owns the central projection's execution, clients, framing, and transports.
- Applications own Cargo dependencies, implementation selection, platform setup, entry points, and lifecycle.

Runtime and protocol crates must not depend on the host compiler,
`boomerang_macros`, Cargo metadata, or package discovery. RTI code must not own application lowering or
scheduler execution. Protocol-specific I/O must remain outside generic scheduler code.

Federate identity and compiled interfaces are unconditional. There is no `federated`
feature that turns an application into a Federate. Optional backend dependencies,
replay containers, and hosted capabilities remain separate concerns.

## Optional CLI

`cargo-boomerang` provides the optional `monitor` command through its `monitor`
feature. It has no `build`, `check`, or `run` command. Applications do not require it.

It does not consume `Boomerang.toml`, generate workspaces, install compiler wrappers,
or maintain an application compilation cache. Future scaffolding and artifact
packaging remain optional tooling. Flashing, provisioning, credentials, remote
lifecycle, containers, and production orchestration remain external responsibilities.

## Verification and acceptance

Verification must cover macro isolation and binding failures, canonical ordering,
contract and placement validation, strict dependency slicing, image validation,
reference/generated equivalence, and independent multi-process execution.
It must cover mismatched coordination identities, participant failures, RTI failure,
and coordinated shutdown. All named architecture types and fields require concise rustdoc.
Crate boundaries must enforce `missing_docs` where permitted. Review must check
private-field documentation for semantic responsibility and identity lifetime.

The same source topology must support a local application and a partitioned
application through deployment inputs. The local compiled application provides the
whole-system CI reference path. Production partitioning also requires independent
artifact and process tests. Neither path replaces the other.

Hosted acceptance includes independent target builds, compatible coordination
tables, direct startup from images, and exclusion of remote payloads and compiler
dependencies. Constrained and replay acceptance additionally require bounded,
allocator-free startup and replacement-implementation replay with unchanged boundary
tags and ordering. Those additional criteria remain open.

## Runnable examples

Run the local [physical-input](../examples/physical-input/README.md) application:

```sh
cargo run -p physical-input
```

Build and execute independent controller, sensor, and RTI fixture applications:

```sh
bash boomerang_codegen/tests/run-applications.sh
```

The [native federation fixture](../boomerang_codegen/tests/fixtures/federated/README.md)
shows shared host declarations and independently buildable target packages.
