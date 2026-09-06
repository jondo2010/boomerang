# Federated Deployment Roadmap Design

**Status:** Approved design awaiting roadmap implementation
**Date:** 2026-09-05

## Purpose

This document revises the direction of phases 5 onward in Boomerang's static deployment roadmap.
It extends [Static Federate Deployment Architecture](deployment-architecture.md) from a
static central-RTI deployment into a closed-world, heterogeneous, real-time-capable federation
architecture.

The roadmap is for one final product architecture. Its milestones are implementation and evidence
checkpoints, not promises that Boomerang is shippable or generally deployable at every phase.

## Context and correction to the current roadmap

The existing roadmap correctly establishes stable topology identities, host-side global analysis,
compiled deployment images, and generated artifacts. Its remaining phases currently build static
federation beside the live `RuntimeAssembly` path, add recording/replay, add bounded execution, and
only then remove the live path.

That migration order is unnecessarily conservative for a pre-launch project with no compatibility
obligation to deployed users. From phase 5 onward, Boomerang shall build directly toward the
compiled architecture:

- remove the legacy federated construction and execution path during phase 5;
- retain ordinary non-federated live `Assembly` execution only until compiled execution is ready to
  replace it in phase 8;
- preserve protocol knowledge and valid semantic tests by moving them to the new coordination
  core, not by preserving the legacy runner;
- move bounded and constrained execution ahead of hosted recording/replay; and
- extend the roadmap to cover strong zero-delay coordination, deterministic lifecycle changes,
  recovery, physical-time contracts, mixed-criticality policies, and security.

The earlier umbrella direction in GitHub issue #91 is superseded where it calls for one application
binary that selects all runtime roles. The deployment architecture produces one statically
specialized binary per Federate and, for the initial centralized projection, a separate RTI
artifact.

## Architectural principles

### Closed-world federation

Every Federate identity, connection, legal replacement, resource bound, codec, transport, and
recovery policy is known at compile time. Runtime join and leave change which members of that known
set are active; they never introduce an unknown participant or mutate the application graph.

This is a defining distinction from ROS-style runtime discovery. Unknown binaries, undeclared
connections, and arbitrary topology changes require a newly compiled deployment.

Closed-world structure does not freeze exact binary contents. A compatible implementation may be
rebuilt and installed offline without rebuilding unrelated Federate artifacts.

### Backend-neutral coordination semantics

Logical-time, membership, recovery, and failure semantics are independent of the coordination
backend. The first implementation is a `central-rti` projection. Later peer-to-peer or hierarchical
projections consume the same canonical analysis and must satisfy the same conformance suite.

`RtiImage` is a backend projection, not the authoritative federation model.

### Explicit policies

Every Federate has an explicit compiled recovery policy. Every cross-Federate boundary has explicit
failure, timing where applicable, codec, transport, and security policies. Deployable compilation
does not silently invent defaults for safety-relevant behavior.

### Bounded target work

Host compilation performs graph analysis, constructiveness checks, dependency closure, resource
analysis, dense indexing, and backend projection. Target runtimes consume immutable dense tables
and bounded buffers. Constrained runtimes do not discover topology, reconstruct graphs, negotiate
schemas, or allocate storage based on runtime membership.

## Recovery model

Transparent process checkpoint/replay is not the baseline recovery mechanism. It is unsuitable as
a general requirement for heterogeneous constrained targets and cannot reverse physical side
effects.

Each Federate selects one of these compiled capabilities:

- `fail-stop`: isolate the Federate and execute the declared downstream failure behavior;
- `restart-reset`: restart the already selected artifact from its compiled initial image;
- `transient-rejoin`: retain local state across a transport interruption and rejoin with a new
  incarnation;
- `redundant-failover`: activate a predefined hot or warm standby;
- `application-state-transfer`: transfer only explicitly declared, bounded semantic state; or
- `checkpoint-restore`: optional hosted capability for deployments that can afford it.

Local platform supervisors detect process failures, watchdog expiry, and platform deadline
violations. The coordination service detects protocol and link failures and turns all accepted
reports into deterministic membership transitions. The RTI is not the platform safety monitor.

The compiler computes failure-impact domains. Each affected boundary declares whether source loss
propagates stop, produces absence or a bounded safe value, enters degraded mode, or switches to a
standby. Analytically independent domains may continue.

Hard-real-time actuator continuity belongs inside a Federate, platform containment domain, or
redundant deployment unit. Federation recovery does not promise to preserve an actuator loop by
rolling back distributed state.

## Membership and lifecycle semantics

Join, planned leave, restart, and failover are membership-epoch transitions with an effective
superdense tag. A transition occurs only at a quiescent boundary:

- no participant changes membership midway through a tag or constructive fixed point;
- no message from the prior epoch may be admitted after the transition;
- zero-delay strongly connected components change membership atomically as coordination domains;
- old participants are fenced by coordination fingerprint, membership epoch, and Federate
  incarnation; and
- an unexpected failure closes the current epoch according to compiled failure policy before
  unaffected domains resume.

Transport acknowledgement, retry, drain, and failure bounds must be sufficient to establish the
transition boundary conservatively. A joining transient Federate receives an effective start tag;
its timers are relative to that start tag.

Lifecycle recovery never changes the selected implementation or artifact version. Membership
epochs coordinate failure recovery, not software deployment.

## Strong logical-time coordination

The full architecture supports every constructive distributed program, including constructive
zero-delay cycles. Non-constructive cycles remain compile-time errors.

The compiler identifies zero-delay strongly connected components and produces the topology and
causality metadata needed by the coordination projection. Ordinary paths use `NET`, `LTC`, and
final `TAG` grants. `PTAG` and port-level `ABS` are used selectively where a final grant would
deadlock a constructive zero-delay component. Earliest incoming message tag calculations include
minimum-delay paths and bounded in-transit-message state.

The implementation shall preserve optimization seams described by the efficient-coordination
research, but correctness comes first. Protocol compression, batching, and reduced control traffic
are later measured optimizations.

## Physical time and mixed criticality

Purely logical fast-forward deployments may omit physical-time contracts. Every physical or
real-time boundary declares the applicable bounds and violation policy, including:

- clock synchronization error;
- transport latency and jitter;
- execution or response budgets;
- deadlines; and
- late-event behavior.

The compiler checks feasibility from supplied platform evidence. The runtime monitors the contract
and invokes the compiled failure or degradation policy when it is violated. Boomerang guarantees
logical determinism and faithful enforcement of declared contracts; a hard-real-time claim remains
conditional on the scheduler, hardware, transport, WCET evidence, clock service, isolation, and
qualification argument selected by the deployment.

Physical effects are irreversible. A late or failed physical action triggers policy; it is never
silently replayed as though the earlier effect had not occurred.

## Transport contract

Coordination consumes a reliable, ordered, framed channel within a membership epoch. Each selected
transport projection either provides those properties directly or implements a bounded shim with:

- framing and integrity checks;
- sequence numbering and duplicate suppression;
- acknowledgement and bounded retry;
- fragmentation and reassembly when required;
- explicit MTU, frame-size, queue, latency, and retry bounds; and
- a bounded priority class for coordination traffic over payload traffic.

Retry exhaustion becomes a link failure and enters the compiled recovery policy. The coordination
state machine does not duplicate transport-specific reliability logic.

This contract permits TCP, shared memory, SPI, and suitable datagram or field-bus projections. A
master-polled SPI implementation may multiplex coordination and payload frames without changing
logical-time semantics.

## Heterogeneous wire and compatibility model

Stable textual identities remain authoritative in source models, manifests, durable configuration,
and diagnostics. Compilation assigns dense deployment-local indices used by steady-state frames.

Compatibility and artifact identity are layered:

- the **coordination fingerprint** covers the topology, boundary contracts, protocol and codec
  versions, dense wire mappings, coordination projection, and policies that participants must share;
- each **Federate-image fingerprint** covers that Federate's compiled scheduler image, local
  bindings, and bounded storage; and
- each **artifact digest** covers the exact produced binary bytes.

Before accepting dense indices, peers verify the coordination fingerprint, membership epoch, and
authenticated Federate identity when the selected security profile requires authentication. A
mismatch fails closed. Participants do not require identical Federate-image fingerprints or
artifact digests because those values describe different deployment slices.

Every boundary selects a codec and maximum encoded size. Generated codecs use a canonical,
architecture-independent representation with specified endianness and field widths; native struct
layout is never transferred. Encoding and decoding use bounded caller-provided storage.

### Offline artifact replacement

A payload implementation may be rebuilt independently when its declared contract remains
compatible. The build reruns descriptor and deployment validation, rebuilds only affected Federate
artifacts, and atomically publishes a new bundle-manifest revision containing their new artifact
digests. Unchanged artifacts are reused.

If only reaction bodies or other implementation internals change, the coordination and
Federate-image fingerprints may remain unchanged. A local structural change may change only the
affected Federate-image fingerprint when it preserves every global boundary and policy. A change to
topology, wire mapping, boundary schema or semantics, protocol, codec, timing or recovery contract,
or another shared policy changes the coordination fingerprint and requires all participants to be
regenerated for the new deployment.

Artifact replacement is initially an offline operation: stop the federation, install the new
bundle revision, and start a new coordinated session. Live code replacement, rolling update,
runtime implementation selection, and mixed-version session negotiation are outside this roadmap.

## Security profiles

Security is explicit per boundary or transport domain:

- `none`, only when accepted by the deployment threat model;
- `integrity-only` for suitably protected on-board links;
- `authenticated`; or
- `authenticated-encrypted`.

A coordination fingerprint proves compatibility, not identity. Authentication binds Federate
identity, coordination fingerprint, membership epoch, and incarnation to the session. Secret key
material is provisioned by the platform and referenced through generated binding slots; it is not
embedded in deployment images. Authentication and integrity failures are link failures.

## Pi-Pico reference deployment

The first heterogeneous hardware proof uses a Raspberry Pi 4B and a Pico-class microcontroller:

- the Pi runs the initial central RTI projection and a hosted Federate;
- the Pico runs an allocator-free Federate from immutable compiled images;
- the link uses the reliable ordered channel contract over SPI;
- the Pico initially uses `restart-reset`, optionally with a small application-owned bounded state
  record;
- the Pi may use `transient-rejoin` or bounded application-state transfer;
- any hard-real-time local control loop remains wholly on the Pico or within its platform recovery
  domain; and
- later phases extend the same deployment with zero-delay coordination and recovery instead of
  replacing it with unrelated demonstrations.

The early proof may deliberately reject zero-delay distributed cycles, lifecycle changes, and
advanced security profiles. Such rejection is an intermediate implementation constraint, not a
different architectural contract.

## Verification and observability

### Final-shaped test pyramid

Tests protect enduring product invariants, not historical milestone implementations:

1. Pure unit and property tests cover canonical lowering, graph analysis, constructiveness,
   coordination state transitions, codecs, bounds, and recovery policy.
2. Shared protocol conformance vectors cover `NET`, `LTC`, `TAG`, `PTAG`, `ABS`, membership epochs,
   incarnation fencing, and failure transitions. Every coordination projection runs the relevant
   vectors.
3. A small component layer checks actual compiler/runtime, codec/transport, and generated-artifact
   boundaries.
4. A minimal system layer proves a hosted reference deployment, Pi-Pico SPI deployment,
   constructive zero-delay case, and lifecycle/recovery/failover case.
5. Hardware-in-the-loop testing is used only for properties that simulation and cross-compilation
   cannot establish.

Each invariant has one authoritative lowest-layer test plus only materially distinct boundary
coverage. A milestone RED test should enter an existing final-shaped suite whenever possible.
Temporary harnesses, duplicated differential tests, and legacy comparisons are removed when their
migration purpose ends. No legacy implementation survives solely as a test oracle.

Phase gates run the evolving canonical suite; they do not accumulate a new integration suite for
each phase.

### Reference model and fault injection

A pure deterministic coordination state machine is the protocol oracle. A deterministic simulator
uses compiled topology and resource bounds to explore reordering, loss, duplication, delayed
failure reports, membership transitions, zero-delay SCCs, and deadline violations. Centralized and
later decentralized projections must make equivalent permitted decisions under the shared model.

### Observability

Tracing is compiled as `off`, bounded ring-buffer, or hosted streaming. Embedded tracing never
blocks coordination. Ring-buffer overflow follows an explicit drop or overwrite policy and records
a loss counter. Hosted MCAP recording is a diagnostic and replay capability, not a recovery
dependency.

## Milestone semantics

A milestone is complete when its planned architectural seam is implemented, its retained tests
pass, and its evidence is sufficient for the next phase. Completion does not imply that the product
is deployable, production-ready, backwards compatible, safety qualified, secure for every threat
model, or shippable.

Intermediate milestones may intentionally support a restricted topology or backend, reject future
capabilities, use temporary internal scaffolding, or lack the final operational profile. Such
limitations must be explicit and fail closed.

Release readiness is a separate cross-cutting gate after the baseline feature phases. It evaluates
the complete supported profile and its evidence; it is not inferred from closing any individual
GitHub milestone.

## Revised roadmap

### Phase 5 - Compiled federation baseline and legacy federation cut

- Project the canonical federation analysis into backend-neutral coordination data and an immutable
  central `RtiImage`.
- Generate and execute strictly sliced Federate launchers plus the central RTI artifact.
- Add the explicit recovery, boundary-failure, transport, codec, timing, and security policy schema;
  unsupported behaviors may remain compile-time errors.
- Preserve the federated-reactors research brief in the repository as provenance and translate its
  normative concepts into protocol conformance requirements.
- Remove legacy `RuntimeAssembly` federation construction, `PendingFederation`, the runtime bridge,
  static runner, and public `execute_federation_*` entry points.
- Make Federate structure unconditional.

This revises existing issues #128-#132. It extracts the federated deletion portion of #139 into
phase 5 while leaving ordinary live `Assembly` migration for phase 8.

### Phase 6 - Bounded wire protocol and transport foundation

- Define the canonical compact wire protocol, dense index handshake, codecs, layered coordination
  and Federate-image fingerprints, and per-artifact digests.
- Define the reliable ordered channel contract and implement the TCP reference projection.
- Add bounded protocol state, queues, serialization storage, priority handling, and failure
  conversion.
- Establish the pure coordination reference model, shared conformance vectors, deterministic fault
  injection, and bounded trace interface.
- Reserve membership epochs, incarnations, `PTAG`, and `ABS` in schemas without claiming their
  behavior complete.

The stable scheduler-admission event part of existing issue #133 moves here. MCAP-specific work from
#134 moves to phase 12.

### Phase 7 - Constrained heterogeneous deployment

- Isolate the `no_std` compiled runtime and coordination-client core.
- Generate concrete static storage and prove allocator-free startup and steady-state execution.
- Implement the SPI transport projection and framing/reliability shim.
- Deliver the Pi-Pico vertical proof from independently built Federate artifacts.
- Verify strict package slicing, bounded queues, fingerprint rejection, transport failure, and
  non-blocking tracing.

This incorporates and extends existing issues #135-#137. The result is an architectural proof, not
a product release.

### Phase 8 - Compiled execution migration completion

- Move remaining repository callers to compiled execution.
- Remove live lowering and the non-federated `RuntimeAssembly` construction path.
- Remove transitional differential tests and scaffolding after their unique coverage is represented
  in canonical tests.
- Make generated compiled artifacts the only deployable execution architecture.

This revises issues #138-#139 after the phase-5 federated cut.

### Phase 9 - Strong logical-time coordination

- Add constructive zero-delay SCC analysis and reject non-constructive programs.
- Lower TPO/MLAA-style dependency metadata and bounded absence state.
- Implement EIMT with bounded in-transit tracking.
- Implement selective `PTAG` and port-level `ABS` behavior in the reference model and central RTI
  projection.
- Extend the Pi-Pico proof with one constructive distributed zero-delay case.

### Phase 10 - Closed-world lifecycle and recovery

- Implement membership epochs and tagged planned join/leave.
- Add transient rejoin and incarnation fencing.
- Integrate hybrid platform/coordination failure detection.
- Implement compiled failure-impact domains and boundary policies.
- Implement `restart-reset`, bounded application-state transfer, and predefined redundant failover.
- Exercise lifecycle and recovery under deterministic fault injection and on the Pi-Pico deployment.

General checkpoint/replay and deployment of a new artifact into a running federation are not
phase-10 requirements.

### Phase 11 - Physical time, mixed criticality, and security

- Implement coordinated physical start and clock-domain conversion.
- Compile timing contracts and runtime violation policies.
- Validate supplied clock, latency, queue, and execution bounds.
- Integrate platform supervisor and safe/degraded-mode bindings.
- Implement tiered security profiles with platform-provisioned key bindings.
- Produce the explicit evidence boundary for hard-real-time, isolation, and qualification claims.

### Release-readiness gate

After phase 11, evaluate one declared baseline product profile end to end. The gate includes final
resource bounds, supported topology classes, transport and security profiles, failure semantics,
timing evidence, documentation, canonical test pyramid, Pi-Pico evidence, and removal of
transitional paths. Passing earlier milestones does not imply passing this gate.

### Phase 12 - Optional projections, hosted services, and optimization

- Add container-neutral hosted MCAP recording and replacement-implementation replay.
- Offer checkpoint restore only for hosted profiles that explicitly select it.
- Add peer-to-peer or hierarchical coordination projections against the shared conformance model.
- Measure and implement control-message compression, batching, reduced `PTAG`/`ABS` traffic, and
  other protocol optimizations without changing semantics.

Phase 12 capabilities are independently selectable extensions and are not silently included in the
baseline release profile.

## Roadmap maintenance actions

After this design is accepted for execution:

1. Update `docs/deployment-architecture.md` to incorporate the closed-world lifecycle, recovery,
   timing, transport, security, milestone, and test-lifecycle invariants.
2. Preserve the recovered federated-reactors research brief under `docs/research/` with its primary
   paper links and clearly label it non-normative provenance.
3. Revise GitHub milestones 5-8, add milestones 9-12 and the separate release-readiness gate, and
   preserve ordering without inventing calendar dates.
4. Update or replace issues #128-#139 so each issue owns one final architectural seam and one
   authoritative proof rather than a milestone-specific test stack.
5. Supersede or close issue #91 with links to the current architecture and roadmap.
6. Keep the GitHub Project private and verify issue membership, phase fields, sequence ordering,
   dependencies, and milestone assignments after the update.

## Research provenance

The protocol and lifecycle direction is anchored in:

- Peter Donovan et al., *Strongly-Consistent Distributed Discrete-event Systems*, arXiv:2405.12117;
- Byeonggil Jun et al., *Efficient Coordination for Distributed Discrete-Event Systems*,
  arXiv:2410.06454; and
- Chadlia Jerad and Edward A. Lee, *Toward Dynamism in Distributed Lingua Franca Programs*, IEEE
  Embedded Systems Letters 17(2), DOI 10.1109/LES.2024.3465408.

These sources inform Boomerang's semantics. They do not override the explicit closed-world,
bounded-resource, mixed-criticality, recovery, and backend-neutral decisions in this design.
