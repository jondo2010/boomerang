# Static Federate Deployment Architecture

This document defines Boomerang's target architecture for building and deploying applications
across hosted and constrained targets. It is normative design guidance, not a description of the
current implementation and not a step-by-step implementation plan.

It specializes the logical graph model in [Graph Partitioning, Federation, and Replay
Architecture](./architecture.md) for static compilation and physical deployment. This document
defines:

- source-level Enclave and placement-group declarations;
- build-time implementation binding;
- production of one binary per Federate;
- host-side lowering and generated static scheduler images;
- replay through deployment-selected implementations; and
- the feature boundary between Federates and coordination backends.

The logical-time behavior in [Federated Runtime Internals](./federated-runtime.md) remains
applicable. [Static Federated Protocol](./federated-protocol.md) defines the initial `central-rti`
backend; another backend may use a different protocol only if it preserves the same declared
logical-time guarantees. The live `RuntimeFederation` and startup-time runtime-object construction
described by those documents are transitional implementation forms that this architecture replaces
with compiled deployment images.

## Decision summary

Boomerang applications use the following deployment architecture:

- Every application is conceptually a Federate, including a wholly local application.
- Federate structure is unconditional. Only distributed coordination backends, protocols, and
  transports are optional.
- A one-Federate application executes independently without an RTI.
- A production deployment contains one statically specialized binary per Federate.
- A deployment containing multiple Federates explicitly selects a coordination backend. The
  initial `central-rti` backend also produces one RTI binary; a future `peer-to-peer` backend does
  not.
- An Enclave is a scheduler and logical-time domain inside a Federate. It is not a process,
  protection domain, compute node, or safety boundary.
- Source code declares stable logical component instances, Enclaves, placement groups, contracts,
  and placement constraints.
- A separate `Boomerang.toml` selects component implementations and maps placement groups to named
  Federates.
- `cargo-boomerang` resolves all implementation bindings before global analysis, lowers the entire
  resolved deployment on the host, generates one static slice per Federate, and launches an
  independent Cargo build for every target.
- Generated deployment binaries perform no graph construction, validation, deserialization, or
  lowering at startup.
- Strict static slicing is required: a Federate build does not compile implementation packages
  assigned only to another Federate.
- Deployment-capable component packages expose macro-generated descriptor and payload facets from
  one source declaration.
- Generated launchers bind direct target symbols to host-generated stable binding slots. There is
  no runtime component registry.
- `ApplicationTopology` uses canonical stable-ID keyed host collections and stable-ID
  relationships. Permanent dense indices appear only in compiled deployment and runtime images.
- `ApplicationTopology` owns deterministic structural inspection and visualization; transitional
  `Assembly` diagnostics delegate to it, while runtime-state diagnostics remain runtime-owned.
- Every named type and field introduced for this architecture has a concise documentation comment.
- Generated monolithic deployments are the authoritative whole-system development and CI path.
- A `std`-only owned reference executor remains for fast unit tests and compiler validation, but
  there is no permanent second dynamic-runtime architecture.
- Scheduler-boundary recording and replay use stable logical boundary identity. MCAP is initially
  retained behind container-neutral trace interfaces.
- `cargo-boomerang` builds artifacts and supervises local host execution. Flashing, provisioning,
  remote lifecycle, containers, and production orchestration remain external responsibilities.

## Goals

This architecture must support:

- one application source model deployed as one or many Federates;
- one binary per Federate with independent target triples, toolchains, Cargo features, runtime
  backends, and linker configuration;
- hosted, RTOS, and eventually bare-metal targets without requiring graph analysis on the target;
- strict exclusion of non-local implementation code and dependencies;
- complete-system deterministic graph and federation analysis before target compilation;
- a monolithic generated deployment for development and CI;
- local multi-process execution of production-equivalent Federate and, for the initial
  centralized backend, RTI artifacts;
- recording nondeterministic sensor traffic at a scheduler boundary and replaying it with a
  replacement implementation in a different deployment topology;
- stable compatibility checks across production and replay deployments; and
- a credible path toward bounded, allocator-free scheduler execution.

## Non-goals

This architecture does not make `cargo-boomerang` a general deployment orchestrator. It does not
define flashing, secure provisioning, host inventory, container scheduling, systemd integration,
or remote process supervision.

It does not claim safety, security, fault containment, or mixed-criticality guarantees merely
because code is placed in different Enclaves or Federates. Those guarantees require explicit
platform isolation, resource budgets, timing evidence, failure semantics, and qualification work.

It does not support runtime implementation selection, runtime topology mutation, dynamic Federate
membership, or startup-time graph lowering in deployable artifacts.

It does not guarantee behavioral equivalence between alternative component implementations.
Production-to-replay compatibility is defined at declared logical boundaries.

## Terminology and hierarchy

The source and deployment model has this hierarchy:

```text
Application topology
  -> logical component instances
      -> placement groups
          -> Enclaves
              -> reactors, actions, ports, and reactions

Resolved deployment
  -> Federates
      -> one or more placement groups and Enclaves
          -> one generated deployment-unit binary
  -> selected coordination backend
      -> optional coordinator artifact
```

The concepts have distinct meanings:

- **Logical component instance:** A stable application-level unit with an external contract. A
  deployment binds it to one concrete component implementation package.
- **Placement group:** The smallest source-declared region that deployment may assign as a unit.
  It has a stable ID and may carry co-location, separation, capability, or resource constraints.
- **Enclave:** The smallest independently scheduled Boomerang logical-time domain. Multiple
  Enclaves in one Federate communicate in-process and share the Federate lifecycle and address
  space.
- **Federate:** One stable federation-participant identity, process lifecycle, and deployable
  binary. A Federate may contain multiple Enclaves. Its identity is independent of whether the
  selected coordination backend uses an RTI. A Federate is not synonymous with an ECU, SoC, or
  host.
- **Deployment unit:** The executable artifact and process or protection-domain instance. The
  initial architecture maps one Federate to one deployment unit.
- **Compute node:** An external deployment-system concept such as an ECU, SoC, host, or board. A
  compute node may run multiple Federates.
- **Execution resource:** A CPU, core, DSP, GPU, NPU, or similar resource. Mapping Enclaves or
  work onto execution resources is separate from Federate membership.

## Migration context

The current implementation combines declarative analysis and live runtime construction.
`Assembly::into_runtime_assembly` performs partition analysis, federation analysis, connection
lowering, runtime Enclave creation, state/action/port/reaction construction, replay construction,
and final scheduler-graph preparation in one consuming operation in
`boomerang_builder/src/assembly/build.rs`.

The macro and builder model similarly combine structural declarations and executable payloads:

- `#[reactor]` expands to a closure that receives concrete state and mutates an `Assembly` in
  `boomerang_macros/src/reactor.rs`.
- `ReactorSpec` owns concrete state through `Box<dyn BaseReactorState>` in
  `boomerang_builder/src/reactor.rs`.
- `ReactionSpec` owns both dependency relations and a `DeferredReactionFactory` that captures the
  concrete reaction closure in `boomerang_builder/src/reaction.rs`.
- `Scheduler` consumes an `Enclave` and constructs a pinned boxed store, event structures,
  coordination state, and heap-backed scratch buffers at startup in
  `boomerang_runtime/src/sched/mod.rs`.

These couplings are convenient for hosted execution but prevent strict target slicing and require
allocator-backed graph-to-runtime construction. The target architecture separates these phases
without duplicating scheduler semantics.

## Architectural representations

The compiler pipeline uses three primary representations.

### `ApplicationTopology`

`ApplicationTopology` is the target-neutral logical application produced by the topology package
and selected descriptor facets. It contains:

- stable component-instance identities;
- component contracts and implementation descriptor identities;
- reactors, ports, actions, reactions, modes, and connections;
- logical delays and payload schema identities;
- Enclaves and local logical-time dependencies;
- placement groups and source-declared constraints;
- recordable logical boundaries; and
- stable implementation binding slots.

It contains no concrete runtime state, target driver objects, reaction closures, protocol clients,
channels, or runtime allocation keys.

Its canonical in-memory representation uses stable-ID keyed ordered host collections for
components, reactors, ports, actions, reactions, Enclaves, placement groups, boundaries, and other
logical entities. Relationships between those records also use typed stable IDs. Canonical map
ordering makes equality, iteration, serialization, visualization, and fingerprint inputs
independent of declaration or package-discovery order.

Nested reaction collections are canonical too: action relations precede port relations, with each
category ordered by declaration position and then stable target ID. Enabled-mode and reset-mode
memberships are sorted by stable mode ID, and duplicate memberships are rejected as malformed.

Host graph-analysis and lowering algorithms may construct private temporary dense indexes for
efficient traversal. Those indexes are implementation details: they are not stored in
`ApplicationTopology`, exposed as identity, or reused across analysis boundaries.

`ApplicationTopology` also owns structural inspection and visualization of the resolved logical
graph. Its `Debug` output and graph-oriented helpers use stable identities, deterministic ordering,
and expose components, reactors, actions, ports, reactions, connections, Enclaves, and placement
groups without constructing runtime objects. During migration, `Assembly` may delegate its
structural debug output to this representation. Runtime alias maps, queues, scheduler state, and
other mutable-runtime diagnostics remain with their runtime structures.

### `ResolvedDeployment`

`ResolvedDeployment` combines one `ApplicationTopology` with one named deployment from
`Boomerang.toml`. Every choice is explicit and complete:

- each component instance has exactly one selected implementation;
- each placement group belongs to exactly one Federate;
- every Federate has a target and runtime backend;
- every multi-Federate deployment has exactly one selected coordination backend;
- cross-Federate connections have selected codec and transport capabilities; and
- all source constraints have been checked.

No implementation binding or placement decision remains unresolved after this phase.

### `CompiledDeployment`

`CompiledDeployment` is the canonical, immutable result of host-side analysis and lowering. It
contains:

```text
CompiledDeployment
|- deployment fingerprint
|- boundary-contract fingerprints
|- GlobalFederationImage (backend-neutral)
|- FederateImage[]
|  |- Federate identity
|  |- EnclaveImage[]
|  |- local and federated routes
|  |- required payload bindings
|  `- bounded storage requirements
`- CoordinationProjection
   |- Local
   |- CentralRti(RtiImage)
   `- future PeerToPeer(PeerImage[])
```

It is data, not a collection of live runtime objects. It has no channels, closures, threads,
protocol sessions, or open transports. Permanent dense indices are assigned only while lowering
this representation, after canonical global analysis and deployment slicing. Each index is local
to its compiled image or Federate slice and is never a source-level identity.

## Source authoring model

### Topology package

The application workspace contains a host-compatible topology package. It declares logical
component instances and connects them through stable component contracts. It may use ordinary Rust
to parameterize topology, but topology-affecting inputs must be explicit compiler inputs and must
be reflected in the canonical topology output.

The topology package depends on contract and descriptor APIs, not target HALs or component payload
implementations. It names logical binding sites such as `vehicle/sensor`, not packages such as
`sensor-stm32`.

The topology package owns the stable placement-group surface used by `Boomerang.toml`. A selected
implementation descriptor may introduce internal reactors and Enclaves within its assigned group,
but it may not silently rename, split, or add externally placeable groups. A component contract
must expose any implementation-selectable placement surface explicitly.

### Placement groups and Enclaves

Source declares placement groups because they are stable application structure and the unit of
independent placement. Deployment files map groups to Federates; they do not create or split
groups.

Source also declares Enclaves. Enclave boundaries affect scheduler ownership and logical-time
coordination, so they are not silently inferred from process placement. Mapping two Enclaves to one
Federate preserves two scheduler domains and makes their communication in-process. Mapping their
placement groups to different Federates creates serialized boundaries coordinated by the selected
backend when the logical connection permits it.

### Component contracts

A component contract defines only externally relevant structure:

- stable contract ID and version;
- named ports, directions, and payload schema IDs;
- logical timing and delay constraints;
- recordable boundary identities;
- required capabilities; and
- resource-contract fields required by supported backends.

Alternative implementations may have different internal Enclaves, actions, reactions, state, and
resource use. They must satisfy the same external contract to bind at the same logical component
site.

## Dual-facet component packages

A deployment-capable component implementation is a Cargo package in the application workspace. The
Boomerang macros generate two mutually exclusive facets from one source declaration.

The Cargo package is the minimum strict-slicing unit. Code that must be independently placed or
compiled for incompatible targets must live in separate packages. One package may expose multiple
component implementations only when compiling that package as a unit is acceptable. The compiler
does not claim module-level dependency slicing inside one Cargo package.

### Descriptor facet

The host-compiled descriptor facet exposes:

- stable reactor, port, action, reaction, mode, state, and codec slot IDs;
- child structure and topology parameters;
- trigger, use, effect, mode, and scope relationships;
- placement groups, Enclaves, delays, and constraints;
- external contract conformance;
- declared queue, payload, state, and scratch-space bounds; and
- a descriptor fingerprint and macro ABI version.

It does not compile concrete state constructors, reaction bodies, HAL access, target transports, or
target-only dependencies.

### Payload facet

The target-compiled payload facet exposes:

- a generated compatibility header containing the descriptor fingerprint and macro ABI version.

The compatibility header lands before direct typed bindings. A dedicated later slice adds concrete
state initialization, reaction, codec, driver, and static-storage symbols after the compiled image
views and `RequiredBindings` define their host-owned slots and concrete signatures. Codec and
driver exports require source declaration models; this architecture does not introduce placeholders
for them. The header contains neither binding slots nor a function table.

The payload facet may also generate wrappers used by the owned host reference executor. Those
wrappers do not define a separate graph or lowering path.

### Build mode

`cargo-boomerang` selects a reserved, tool-owned descriptor or payload build mode through separate
Cargo invocations. The mode is not a Federate feature and does not encode placement. Enabling both
facets is an error.

Target dependencies must be absent from descriptor mode through generated `cfg` boundaries and
Cargo dependency configuration. Deployment-capable reaction and topology declarations must use
compiler-recognized macro syntax so payload bodies can be excluded from descriptor compilation.
The permissive builder API may remain available to owned host tests, but arbitrary closure-bearing
builder code is not automatically eligible for generated static deployment.

## Implementation binding analysis

Implementation binding occurs before global lowering.

`cargo-boomerang` first reads the named deployment and resolves all selected packages through
Cargo metadata. For the initial architecture, selected implementation packages must be members of
the same Cargo workspace and must be present in the workspace lockfile.

The tool then generates a temporary host descriptor-driver crate. That crate depends on:

- the application topology package;
- the selected implementation packages in descriptor mode; and
- the host compiler/descriptor APIs.

The driver constructs a registry that maps every logical binding site to its selected descriptor
entry point, runs the topology entry point, expands the selected descriptors, and emits one
`ApplicationTopology`. This generated driver is the only place where manifest package selection is
turned into Rust dependencies during host analysis.

Global lowering turns stable descriptor slots into `RequiredBindings` for each Federate. Once the
later direct typed bindings exist, the generated target launcher depends on only the payload
packages assigned to that Federate and binds them directly:

```text
ReactionSlotId("vehicle/sensor/sample")
    -> sensor_stm32::__boomerang::sample_reaction
```

The initial compatibility-header slice uses const assertions to verify descriptor fingerprints and
macro ABI versions. Once the later direct typed bindings exist, the Rust compiler also verifies
state, reaction-reference, payload, and codec types. Missing slots, duplicate slots, type
mismatches, or fingerprint mismatches are compile errors. There is no runtime plugin loader,
service locator, package registry, or symbol lookup.

When one implementation package is used for multiple logical instances, lowering instance-
qualifies its stable slots and the launcher allocates distinct state and action storage for each
instance while reusing function symbols where valid.

## Deployment manifest

`Boomerang.toml` is a committed build input separate from `Cargo.toml`. It may contain multiple
named deployment variants without changing source declarations or Cargo feature definitions.

The initial schema has this shape:

```toml
schema = 1

[topology]
package = "vehicle-topology"
entry = "vehicle::topology"

[deployments.production.bindings."vehicle/sensor"]
package = "sensor-stm32"
features = ["board-a"]

[deployments.production.bindings."vehicle/controller"]
package = "vehicle-control"

[deployments.production.federates.sensor-edge]
groups = ["sensor"]
target = "thumbv7em-none-eabihf"
profile = "release"
runtime = "bare-metal"
cargo-config = ".cargo/sensor-target.toml"

[deployments.production.federates.compute]
groups = ["control", "planning"]
target = "aarch64-unknown-linux-gnu"
profile = "release"
runtime = "std"

[deployments.production.coordination]
backend = "central-rti"

[deployments.production.rti]
target = "aarch64-unknown-linux-gnu"
profile = "release"

[deployments.replay.bindings."vehicle/sensor"]
package = "sensor-replay"

[deployments.replay.bindings."vehicle/controller"]
package = "vehicle-control"

[deployments.replay.federates.dev]
groups = ["sensor", "control", "planning"]
runtime = "std"
```

The `coordination` table is required for a multi-Federate deployment and absent for a
one-Federate deployment. `central-rti` is the initially supported backend. The `rti` table is valid
only with `central-rti`; it configures the additional coordinator artifact rather than defining
Federate membership. A future RTI-free deployment selects `backend = "peer-to-peer"` and has no
`rti` table.

```toml
[deployments.future-p2p.coordination]
backend = "peer-to-peer"
```

Implementation-specific Cargo features belong to binding entries. Target triple, toolchain,
profile, runtime backend, target JSON, and optional Cargo configuration belong to Federates.
Linkers, linker scripts, runners, and target-specific flags are supplied through normal Cargo
configuration selected by that entry rather than through an unstructured shell command.
Independent Cargo invocations prevent feature unification across Federates. Feature unification
within one Federate follows normal Cargo rules and is validated against component-declared
incompatibilities.

The build manifest does not contain credentials, host inventory, flash commands, runtime-assigned
addresses, or production secrets. Topology-affecting transport capabilities and delay assumptions
are compiler inputs; concrete addresses and credentials are supplied at deployment or runtime.

## Global analysis and lowering

Global analysis always sees the complete resolved application, even though target builds are
strictly sliced. It performs:

- unique stable-ID and contract validation;
- descriptor and payload requirement validation;
- placement-group coverage and constraint validation;
- Enclave partition and local dependency analysis;
- connection and reaction-level analysis;
- deployment slicing followed by deterministic image-local dense-key assignment;
- payload-schema and codec validation;
- local, cross-Enclave, and cross-Federate boundary selection;
- zero-delay distributed-cycle validation;
- direct and transitive federation dependency analysis;
- minimum-delay path analysis;
- static queue, scratch, and resource-bound validation; and
- generation of immutable global, coordination, and per-Federate images.

All derived collections are canonicalized by stable identity before deployment slicing and
image-local dense-key assignment. Input iteration order must not affect compiled images or
fingerprints.

Connections lower according to the resolved deployment:

| Relationship | Compiled delivery |
| --- | --- |
| Same Enclave | Direct local port/action binding |
| Different Enclaves, same Federate | In-process scheduler event route |
| Different Federates | Serialized endpoint plus selected coordination backend |
| Recordable boundary | Scheduler admission tap in addition to the selected route |
| Replayed implementation | Trace source feeding the same logical boundary contract |

Same-Federate cross-Enclave traffic does not require a federated codec unless the boundary is
recorded through a serialized trace contract. Cross-Federate traffic always requires a compatible
codec and transport capability.

A recordable logical boundary must expose the same canonical event observation point in every
supported deployment. If a direct local binding has no equivalent admission point, lowering
materializes a protocol-neutral boundary adapter. That adapter must preserve the declared logical
tag and ordering contract; recording is not allowed to introduce an observable scheduling change.

## Fingerprints and determinism

The architecture uses distinct fingerprints for distinct claims.

### Descriptor fingerprint

A descriptor fingerprint identifies one implementation's structural facet and macro ABI. The
payload facet must embed the same value. It prevents a target payload from binding to static tables
generated from a different descriptor.

### Deployment fingerprint

The deployment fingerprint covers the canonical resolved topology, selected package identities and
features, Cargo lock resolution, placement-group mapping, target/runtime configuration, selected
coordination backend and protocol version, compiler schema versions, descriptor fingerprints, and
generated global/per-Federate images.

All Federates and any separate coordinator artifact in one running deployment embed and present the
same deployment fingerprint. Participants reject mismatches before coordinated execution begins.

The fingerprint is a consistency and compatibility mechanism, not authentication or
authorization. A secure deployment must authenticate peers and protect configuration through a
separate security design.

### Boundary-contract fingerprint

A boundary-contract fingerprint covers stable boundary identity, direction, payload schema and
encoding, logical timing semantics, and ordering contract. It deliberately excludes Federate
mapping and alternative implementation internals. Recordings use this fingerprint to establish
compatibility across production and replay deployments with different deployment fingerprints.

### Determinism guarantees

The compiler guarantees canonical compiled images for identical locked compiler inputs. This is a
stronger and more tractable claim than promising bit-identical final binaries across arbitrary
toolchains.

Canonicalization sorts stable IDs before compiled deployment slices assign dense typed keys. Dense
keys are local to one compiled image or Federate slice and are excluded from fingerprints and
serialized interchange except where their canonical table position is explicitly part of a
compiled-image schema. Dense key types do not implement serialization traits; a schema that stores
a canonical table position represents it explicitly as an integer field.

The runtime guarantees deterministic logical scheduling for one compiled deployment given the same
initial state and input event stream, subject to documented physical-input and reaction-code
assumptions.

Alternative implementations are not assumed behaviorally equivalent. A replay implementation is
compatible when it reproduces a declared boundary event stream accepted by the boundary contract.

## Generated target artifacts

For every Federate, `cargo-boomerang` generates an ephemeral launcher crate outside the source
workspace membership. Its manifest directly depends on:

- `boomerang_runtime` with the selected static runtime backend;
- only the selected coordination-client and transport support when this Federate is in a
  multi-Federate deployment;
- shared contract/schema crates required by local payloads; and
- only the selected payload implementation packages assigned to this Federate.

It does not depend on the topology package, host compiler, graph analyzer, lowering crates,
implementations assigned only to other Federates, or the complete application package.

The generated source contains:

- immutable `FederateImage` and `EnclaveImage` tables;
- direct payload binding references;
- statically allocated or backend-provided mutable storage;
- runtime-backend initialization;
- transport and selected coordination-backend initialization when required; and
- a minimal process or firmware entry point.

The launcher workspace uses package IDs, paths, versions, and feature selections resolved through
Cargo metadata. `cargo-boomerang` creates and verifies a generated lock resolution against the
source workspace `Cargo.lock`; it does not infer dependencies from names alone.

With the initial `central-rti` backend, a multi-Federate deployment also generates one RTI launcher
containing the immutable `RtiImage`. A future `peer-to-peer` deployment generates no RTI launcher;
its Federate launchers contain the precomputed peer routes and coordination data assigned to their
slices. A one-Federate deployment generates neither an RTI binary nor distributed-coordination
dependencies.

## Coordination backend extensibility

Global host analysis is mandatory and backend-neutral. It produces the complete immutable
federation graph, including stable Federate and endpoint identities, direct routes, transitive
dependencies, minimum-delay paths, and affected downstream sets. A coordination backend adapter
then validates and projects that graph into backend-specific compiled data. `RtiImage` is the
projection for `central-rti`, not the canonical federation representation.

The initial implementation supports `central-rti`. The architecture reserves `peer-to-peer` as a
future backend, not as an initial implementation requirement. Such a backend must:

- emit no RTI artifact;
- provide each Federate slice with explicit direct-peer routes and all dependency, progress, and
  coordination data required by the selected decentralized algorithm;
- perform no runtime topology discovery, dynamic membership negotiation, or graph lowering;
- preserve the same declared logical-time, ordering, and determinism guarantees as the centralized
  backend; and
- reject at build time any federation graph or target capability it cannot support.

Backend selection cannot change the application topology, implementation bindings, Federate
boundaries, or logical boundary contracts. Backend-specific crates and transports are compiled
only for the selected backend. Adding a peer-to-peer backend therefore extends the host projection,
generated coordination tables, and target support without introducing a second graph-analysis
path.

## Compiled scheduler images

Each `EnclaveImage` contains all immutable scheduler information:

- dense reactor, action, port, reaction, mode, and scope tables;
- precomputed trigger sets, dependency levels, and modal schedule indices;
- local logical-time dependency tables;
- state and action-storage slot descriptions;
- direct initialization and reaction bindings;
- fixed queue and scratch-buffer capacities;
- boundary routing tables; and
- optional recording/replay admission hooks.

Generated Rust expresses these as `const` and `static` data. Deployable targets do not deserialize
an image at startup.

The scheduler algorithm operates on bounded interfaces for immutable image access, mutable
state/action storage, event queues, scratch buffers, clocks, wake sources, and logical-time
coordination. Platform backends supply those capabilities without changing scheduling semantics.

For allocator-free backends, state slots, event queues, payload buffers, and scheduler scratch
space have compile-time bounds. A descriptor declares resource bounds, and target compilation
verifies concrete types and layouts fit those bounds. An unbounded requirement is a build error for
a bounded backend.

Hosted targets may use heap-backed mutable storage where permitted, but they still execute
pre-lowered images and do not carry the graph compiler into production.

## Owned host reference executor

The final architecture does not preserve a first-class `DynamicStorage` backend compatible with
today's live `RuntimeAssembly`. It preserves a narrower `std`-only reference executor:

```text
ApplicationTopology + deployment
    -> canonical compiler
    -> OwnedCompiledDeployment
    -> owned host reference executor

The same canonical compiler output
    -> generated Rust image
    -> target executor
```

The reference executor uses owned heap storage for immutable images and mutable slots, but consumes
the same compiled-image schema and runs the same scheduler algorithm. It exists for fast macro,
builder, scheduler, and compiler tests; diagnostics; benchmarks; and differential validation of
generated images.

`Assembly::into_runtime_assembly` may temporarily adapt to this path during migration. It is not a
permanent alternative lowering architecture and is eventually deprecated with live
`RuntimeAssembly` and `RuntimeFederation`.

## Federate and coordination execution

Every generated application contains at least one Federate. A Federate owns:

- its stable Federate identity;
- every Enclave assigned to it;
- its in-process Enclave coordination;
- its local endpoint routes;
- one selected backend client and Federate-wide logical-time coordination service when
  distributed; and
- one process or firmware lifecycle.

Every active Enclave in a distributed Federate participates in the Federate-wide logical-time
frontier. The generated `FederateImage` contains the complete immutable participant layout. The
existing split-phase publication, acquisition, fixed-point completion, and failure supervision
semantics remain required. With `central-rti`, generating the layout does not change the RTI grant
protocol; another backend must provide equivalent logical-time behavior through its own compiled
protocol.

One-Federate execution uses local coordination only. A `central-rti` deployment uses the generated
RTI graph and one persistent ordered client connection per Federate. A future `peer-to-peer`
deployment uses only its generated direct-peer routes and backend-specific coordination tables.
Same-Federate Enclaves never communicate through a federation coordination backend.

## Recording and replay

### Recording point

Source marks a logical connection boundary as recordable. Global lowering assigns a stable
`BoundaryId` and inserts a scheduler admission hook at the receiving side:

```text
transport or local producer
    -> payload decoding
    -> physical-arrival to logical-tag normalization
    -> scheduler admission hook and optional TraceSink
    -> target action queue
```

Recording at receiver admission captures exactly what the downstream application observed and
allows recording to run on a hosted compute Federate instead of a constrained sensor MCU.

### Boundary event

A container-neutral `BoundaryEvent` contains:

- stable `BoundaryId`;
- boundary-contract fingerprint;
- complete logical tag, including microstep;
- trace-wide admission sequence;
- serialized payload; and
- optional elapsed physical time for paced replay.

It does not contain RTI grants, coordination wakes, raw transport frames, `ActionKey`,
`EnclaveKey`, or other deployment-local dense keys.

### Replay binding

A replay deployment selects a replay implementation package at the same logical component binding
site. The replay implementation satisfies the same external contract but may have different
internal structure. It resolves trace boundary IDs through its generated deployment tables and
injects ordinary scheduler events through the same admission path.

Supported policies are:

- **logical:** preserve recorded logical tags and ordering;
- **paced:** preserve logical tags while pacing injection by recorded physical intervals; and
- **fast-forward:** preserve tags and ordering without wall-clock waiting.

Unknown boundaries, contract mismatches, invalid schemas, corrupt ordering, decode failures, or
events in the scheduler's past are terminal replay errors.

### Trace container

The scheduler depends only on `TraceSink` and `TraceSource` interfaces over `BoundaryEvent`. MCAP is
the initial hosted container because it provides schemas, channels, metadata, compression,
indexing, checksums, recovery, and existing tooling. MCAP channel metadata carries stable boundary
and schema identity; a versioned Boomerang message envelope carries logical tags, sequence, and
payload.

Core replay APIs do not accept `mcap::Message` directly and do not require memory-mapping or summary
records. A bounded embedded trace container may be added later without changing scheduler or replay
semantics.

## `cargo-boomerang` commands and outputs

The initial command surface is:

```text
cargo boomerang check --deployment <name>
cargo boomerang build --deployment <name>
cargo boomerang run   --deployment <name>
```

`check` resolves packages, builds and runs the descriptor driver, validates the resolved
deployment, and performs global lowering without compiling target payloads.

`build` performs the complete pipeline and writes content-addressed output:

```text
target/boomerang/<deployment>/<fingerprint>/
|- generated/
|  |- <federate>/Cargo.toml
|  |- <federate>/src/
|  `- rti/                    # central-rti only
|- artifacts/
|  |- <federate>/<binary>
|  `- rti/<binary>           # central-rti only
|- deployment.json
`- reports/
   |- topology.json
   `- resource-usage.json
```

Generated crates are ephemeral and never committed. `deployment.json` contains artifact paths and
hashes, deployment and boundary fingerprints, target triples, runtime backends, Federate
identities, coordination backend and protocol identity, required transport capabilities, resource
requirements, and required runtime configuration fields. It contains no credentials or mutable
host assignment.

`run` supports local host-compatible execution only. It launches a one-Federate binary directly;
for `central-rti`, it launches the generated RTI followed by independent Federate processes. A
future peer-to-peer runner launches the Federates without an RTI. It supervises startup, logs,
coordinated shutdown, and exit status. It rejects non-host-runnable artifacts.

External deployment systems consume `deployment.json` to flash, provision, place, configure, and
supervise heterogeneous artifacts.

## Crate and dependency boundaries

Exact crate extraction may evolve, but dependency responsibilities are invariant:

- `boomerang_macros` generates descriptor and payload facets. It does not contain deployment
  policy, RTI analysis, or platform orchestration.
- The host compiler layer owns `ApplicationTopology`, manifest-independent graph validation,
  canonical lowering, compiled-image construction, backend projection interfaces, and structural
  inspection of the resolved logical graph. Its core graph representation is protocol-neutral.
- `boomerang_runtime` owns compiled-image views, scheduler semantics, storage/queue interfaces,
  clocks, generic logical-time coordination, and trace admission interfaces. It does not depend on
  Cargo metadata, Tokio, RTI frames, or graph construction.
- The RTI/protocol layer owns wire identities and tags, immutable RTI graph projection, RTI state,
  sessions, clients, framing, and transports. It does not own source descriptors, application
  graph lowering, or scheduler execution.
- Host-side federation backend adapters validate and project the protocol-neutral federation graph
  into backend-specific coordination images, participant routes, and generated transport bindings.
  The centralized adapter produces `RtiImage`.
- `cargo-boomerang` owns Cargo metadata resolution, generated descriptor drivers and launcher
  workspaces, target build invocation, artifact bundling, and local process supervision.

No dependency points from runtime or RTI/protocol crates back into the host compiler, macros, or
Cargo tool. The generated target dependency graph includes only runtime capabilities and selected
payload/backend packages.

## Feature model

Federate identity and compiled Federate structure are always available. There is no user-facing
feature that turns a local application from "not a Federate" into a Federate.

Distributed coordination protocols, client transports, and any coordinator executable are behind
backend-specific internal feature boundaries. `cargo-boomerang` selects only the capability named
by the deployment: initially `rti` for `central-rti`, and eventually a distinct capability for
`peer-to-peer`. It omits all distributed-coordination capabilities for one-Federate deployments.
There is no user-authored Cargo feature that selects federation placement or coordination backend.

Replay support is orthogonal. Trace interfaces may be present in the runtime core while concrete
containers such as MCAP remain optional hosted dependencies.

## Validation and failure boundaries

Validation is staged and failure-atomic:

1. Parse and schema-check `Boomerang.toml`.
2. Resolve topology and implementation packages through locked Cargo metadata.
3. Compile and execute the selected descriptor driver.
4. Validate contracts, descriptor identities, and source constraints.
5. Resolve Federate placement and validate complete topology and coordination-backend semantics.
6. Perform canonical global lowering and static resource analysis.
7. Generate and validate every Federate image and the selected coordination-backend projection.
8. Compile target payload bindings and verify descriptor fingerprints.
9. Hash artifacts and atomically publish `deployment.json`.

Errors identify the named deployment, component instance, implementation package, placement group,
Federate, stable graph ID, and source or manifest location where applicable. Unsupported topology
or resource semantics fail before target compilation. Target-binding failures fail before artifact
publication. Fingerprint and protocol mismatches fail before coordinated logical execution.

Builds use fingerprinted staging directories. A failed or interrupted build cannot leave a new
deployment bundle that appears complete.

## Verification strategy

The architecture requires the following test layers:

- macro expansion and compile-fail tests for descriptor/payload separation, target-only dependency
  leakage, missing slots, duplicate slots, type mismatch, and fingerprint mismatch;
- canonical topology and compiled-image tests under reordered input declarations;
- manifest tests for package binding, group coverage, source constraints, target compatibility,
  single-versus-multi-Federate behavior, and coordination-backend selection;
- strict-slicing tests proving an unrelated target package and its dependencies are absent from a
  Federate build;
- differential tests comparing owned-reference and generated-static execution of the same compiled
  image;
- a generated monolithic full-system integration test as the canonical CI execution path;
- local multi-process Federate and central-RTI integration tests;
- host plus bare-metal cross-compilation tests;
- bounded storage, queue-capacity, and allocator-absence tests for constrained backends;
- production boundary capture followed by monolithic replay with a replacement implementation;
- deployment-fingerprint and boundary-contract mismatch tests; and
- failure-atomic startup, scheduler failure, coordination-participant failure including RTI
  failure, and coordinated shutdown tests.

All named types and fields added for these representations carry concise Rust documentation
comments, including private compiler-model records. Public documentation is enforced with the
`missing_docs` lint where the crate boundary permits it; private-field documentation is part of the
review checklist. Documentation describes semantic responsibility or identity lifetime rather
than restating the Rust type.

The existing in-memory runner may remain as migration and protocol-test scaffolding. It is not the
final proof of deployable artifact correctness.

## Migration direction

Migration follows architecture seams rather than attempting one replacement:

1. Introduce stable source identities, `ApplicationTopology`, and dual-facet macro output while
   preserving current hosted APIs.
2. Separate canonical structural lowering from live runtime construction.
3. Introduce compiled images and the owned host reference executor; use differential tests against
   current behavior.
4. Add `Boomerang.toml`, workspace package resolution, and `cargo-boomerang check`.
5. Generate and run one-Federate hosted artifacts with no runtime lowering.
6. Add strict per-Federate package slicing, independent target builds, generated RTI artifacts, and
   local multi-process supervision for the initial `central-rti` backend, while retaining the
   backend-neutral analysis and projection seam.
7. Move replay to stable scheduler-boundary events and container-neutral trace interfaces.
8. Add bounded static storage and allocator-free runtime backends.
9. Remove the `federated` user-facing feature distinction and deprecate live `RuntimeAssembly` and
   `RuntimeFederation` construction.

This sequence is directional, not an implementation plan. Each step requires its own scoped plan,
runnable proof, and compatibility decision.

## Architectural invariants

- All implementation binding occurs before global lowering.
- Global lowering sees one complete resolved application.
- Production artifacts contain no graph compiler or startup-time lowerer.
- A Federate build compiles only its assigned payload packages and shared dependencies.
- Every generated application has at least one Federate.
- One Federate produces one binary and requires no RTI.
- Multiple Federates produce independent binaries and explicitly select a coordination backend.
- The `central-rti` backend produces one additional RTI binary; a `peer-to-peer` backend produces
  none.
- Coordination backends consume the same complete canonical federation analysis and cannot perform
  runtime topology discovery or lowering.
- Enclave, Federate, deployment unit, compute node, and execution resource remain distinct.
- Enclave boundaries are scheduler semantics, not evidence of process or safety isolation.
- `ApplicationTopology` uses canonical stable-ID keyed collections and stable-ID relationships;
  graph algorithms may use private temporary dense indexes.
- Permanent dense keys never serve as durable identity and are assigned only in compiled/runtime
  images after stable-ID canonicalization and deployment slicing.
- Structural topology inspection belongs to `ApplicationTopology`; mutable runtime diagnostics do
  not.
- Every named architecture type and field has a concise documentation comment.
- Descriptor and payload facets are tied by fingerprints and compile-time type checks.
- All compiled collections are canonicalized before dense indexing and hashing.
- The owned host executor and target executor consume the same compiled-image schema and scheduler
  algorithm.
- Recording captures scheduler-admitted logical boundary events, not RTI control traffic or runtime
  keys.
- Deployment fingerprints establish peer compatibility; boundary-contract fingerprints establish
  recording compatibility.
- Unsupported topology, resource, or target semantics are rejected during the earliest phase that
  has enough information to prove the error.
- `cargo-boomerang` does not silently broaden from build orchestration into production deployment
  orchestration.

## Acceptance criteria for the architecture

The architecture is materially realized when one workspace can demonstrate all of the following:

1. The same topology builds as a generated one-Federate host binary and as multiple independently
   compiled Federate binaries plus an RTI under the initial `central-rti` backend.
2. One Federate targets a hosted platform while another cross-compiles for a constrained target,
   and neither compiles the other's payload package.
3. Both deployments are derived from the same complete host analysis and contain consistent
   fingerprints.
4. Generated binaries start schedulers directly from immutable static images without invoking
   `Assembly` or lowering.
5. The monolithic generated deployment runs as a normal CI integration test.
6. A production sensor-boundary trace replays through a replacement sensor implementation in the
   monolithic deployment while preserving logical tags and ordering.
7. The RTI is absent from the one-Federate artifact graph and present only when the multi-Federate
   deployment selects `central-rti`; backend identity is explicit in the compiled deployment and
   deployment fingerprint.
8. A bounded runtime backend links and starts without a global allocator or startup graph
   construction.
