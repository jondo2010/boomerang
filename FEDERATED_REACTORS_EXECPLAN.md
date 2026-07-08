# Implement Static Federated Enclaves

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository does not currently contain `.agent/PLANS.md` or `PLANS.md`, although `AGENTS.md` refers to `.agent/PLANS.md`. This plan was written using the bundled ExecPlan rules from the Codex `execplan-authoring` skill. If a repository-local `PLANS.md` is added later, update this file to match it.

## Purpose / Big Picture

After this change, Boomerang can run a statically declared set of remote enclaves as a federation: a source reactor in one process can send a tagged value to a destination reactor in another process, and an RTI process can grant logical-time advancement so the result matches current in-process enclave execution for supported topologies. In this plan, a "federate" means a remote enclave: the same partitioning concept already used by Boomerang, but executed outside the current process and coordinated over a reliable ordered transport.

The first working behavior is deliberately narrow. It supports static persistent federates, serialized logical data messages, and centralized RTI coordination using MSG, NET, LTC, and TAG. It rejects distributed zero-delay cycles with a builder error instead of silently running an incomplete protocol. PTAG and ABS support, transient federate join/leave behavior, physical remote connections, authentication, and optimized coordination are later milestones.

## Progress

- [x] (2026-07-08 19:10Z) Read `FEDERATED_REACTORS_RESEARCH.md` and grounded the design in current runtime and builder files.
- [x] (2026-07-08 19:10Z) Identified the local data-plane and control-plane hooks: `boomerang_builder/src/connection.rs`, `boomerang_runtime/src/reaction.rs`, `boomerang_runtime/src/sched/mod.rs`, `boomerang_runtime/src/sched/barrier.rs`, and `boomerang_runtime/src/env/mod.rs`.
- [x] (2026-07-08 19:10Z) Decided the first implementation is a static-federation MVP with explicit zero-delay-cycle rejection, not a partial PTAG/ABS implementation.
- [x] (2026-07-08 19:22Z) Incorporated refinement answers: use an explicit `ReactorPlacement` enum, design for async I/O inside `boomerang_federated`, keep multiple local enclaves inside one federate as future support, and make payload serialization codec-agnostic rather than serde-only.
- [x] (2026-07-08 19:32Z) Recorded user decisions that async I/O is firm for the first slice and serde remains the first payload codec, then added RTI-routed versus direct federate data routing analysis.
- [x] (2026-07-08 19:37Z) Recorded user decision that first-slice MSG data frames route through the RTI.
- [x] (2026-07-08 19:40Z) Started Milestone 1 implementation, limited to builder/API placement metadata, federation topology extraction, and unsupported distributed zero-delay cycle validation.
- [x] (2026-07-08 19:53Z) Added `ReactorPlacement`, feature-gated `FederationPlan` metadata, cross-federate topology extraction, and explicit distributed zero-delay-cycle rejection with focused builder tests.
- [ ] Add wire tag, protocol message, RTI state, and in-memory transport tests in a new `boomerang_federated` crate.
- [ ] Add runtime scheduler hooks for an optional federated time barrier and transport-backed inbound events.
- [ ] Lower cross-federate connections into serialized sender reactions and inbound endpoint registry entries.
- [ ] Add equivalence tests for distributed hello, delayed connection, and zero-delay-cycle rejection.

## Surprises & Discoveries

- Observation: The repo points to `.agent/PLANS.md`, but neither `.agent/` nor `.agents/` exists in the working tree.
  Evidence: `find . -maxdepth 2 -name PLANS.md -print` returned no files.

- Observation: The existing local enclave model is already close to the desired first federated model.
  Evidence: `EnvBuilder::build_partition_map` partitions reactors on `ReactorBuilder::is_enclave`, `ConnectionBuilder::build` emits `EnclaveDep` for cross-partition edges, and `runtime::crosslink_enclaves` wires upstream/downstream scheduler references.

- Observation: Cross-enclave payload forwarding currently captures concrete local runtime handles.
  Evidence: `build_enclave_connection_source` creates a target `SendContext` and `AsyncActionRef`, then constructs `runtime::EnclaveSenderReactionFn<T>`.

- Observation: The `wincode` crate is not a serde format. It is a separate schema-based binary encoding API, so supporting it cleanly requires a codec abstraction instead of a blanket `T: serde::Serialize + serde::de::DeserializeOwned` assumption.
  Evidence: The published crate documentation describes `SchemaWrite` and `SchemaRead` traits and encode/decode helper functions.

- Observation: Moving generated reactor construction to `ReactorPlacement` still needed to preserve the old direct `Reactor::build(..., is_enclave: bool, ...)` surface.
  Evidence: `cargo test -p boomerang --features federated hello_world` initially exposed direct boolean build callers outside generated reactor code; the final implementation keeps `build` boolean-compatible and adds an internal placement-aware `build_with_placement` path.

## Decision Log

- Decision: Treat the first federate model as "one federate equals one enclave root".
  Rationale: Current Boomerang already has explicit enclave partitioning. Reusing it avoids resurrecting the old top-level-child transform from PR #17 and makes current in-process enclave execution the test oracle.
  Date/Author: 2026-07-08 / Codex

- Decision: Add an instance-level builder API for federates rather than a type-level `#[reactor(federate)]` annotation as the first public surface.
  Rationale: Whether a reactor instance is remote is deployment placement, not an inherent property of the reactor type. Current generated reactor builders already accept an `is_enclave` boolean at instantiation time, so an instance-level federate method fits the existing shape.
  Date/Author: 2026-07-08 / Codex

- Decision: Model placement with an explicit `ReactorPlacement` enum instead of adding more boolean flags.
  Rationale: The existing `is_enclave` boolean is already doing placement work. Federation introduces at least three cases: local reactor, local enclave root, and remote federate root. An enum is harder to misuse and can grow later to represent one federate containing multiple local enclaves.
  Date/Author: 2026-07-08 / Codex

- Decision: Put RTI, client, protocol, and transports in a new optional `boomerang_federated` crate, with small feature-gated hooks in `boomerang_runtime`, `boomerang_builder`, and the top-level `boomerang` crate.
  Rationale: `boomerang_runtime` is currently synchronous and dependency-light. Network framing, serialization, RTI state, and executable helpers should not become mandatory runtime dependencies.
  Date/Author: 2026-07-08 / Codex

- Decision: Use async I/O inside `boomerang_federated` from the first implementation slice, while keeping the Boomerang reaction scheduler synchronous.
  Rationale: The RTI and federate client need to multiplex reliable ordered streams, timeouts, shutdown, backpressure, and control/data frames. Async I/O handles those concerns more naturally than a thread-per-connection design. The boundary to `boomerang_runtime` should remain a small bridge that injects `AsyncEvent`s through the existing scheduler channel. The user confirmed this is a firm first-slice decision.
  Date/Author: 2026-07-08 / Codex

- Decision: The first protocol slice uses static topology with MSG, NET, LTC, TAG, topology delays, and in-transit message queues. It rejects distributed zero-delay cycles.
  Rationale: Full constructive zero-delay-cycle support requires PTAG and ABS plus explicit absence semantics. Current Boomerang ports are represented as `Option<T>` at a tag, but the runtime does not yet produce network absence messages for every upstream port/tag. A rejection is correct for an MVP, while a partial PTAG/ABS implementation would be misleading.
  Date/Author: 2026-07-08 / Codex

- Decision: Represent wire tags independently of `std::time::Instant`.
  Rationale: Current `Tag` stores a `time::Duration` offset plus a `usize` microstep. A remote wire format must not serialize a process-local `Instant` or architecture-sized `usize`.
  Date/Author: 2026-07-08 / Codex

- Decision: Design for transient federates in state and wire vocabulary, but do not implement join/leave behavior in the static MVP.
  Rationale: The transient paper requires effective start tags, absent intervals, timer alignment to effective start, and delayed/cancelable downstream grants. Those rules should shape identifiers and RTI state now, but implementing them before static TAG/LTC/NET works would expand the first slice too much.
  Date/Author: 2026-07-08 / Codex

- Decision: Make federation payload serialization codec-agnostic, but use serde as the first supported and default payload codec.
  Rationale: Existing Boomerang serde support is useful and should remain the initial developer experience. The protocol design should still avoid requiring serde forever. A codec abstraction allows a later targeted binary crate such as `wincode` without changing scheduler or builder topology semantics. The user confirmed serde is acceptable to start.
  Date/Author: 2026-07-08 / Codex

- Decision: Route first-slice federated MSG data frames through the RTI, not directly between federates.
  Rationale: RTI-routed MSGs keep the MVP centralized, easier to test, and aligned with the RTI's in-transit message queues for TAG decisions. The protocol will still carry source and target federate ids so direct federate-to-federate data channels can be added later as an optimization without redesigning endpoint identity or payload codecs. The user confirmed RTI routing for the first slice.
  Date/Author: 2026-07-08 / Codex

## Outcomes & Retrospective

Milestone 1 builder/API groundwork is implemented. The builder now records enum-based reactor placement, preserves the existing `is_enclave` compatibility path, emits feature-gated static `FederationPlan` metadata for cross-federate connections, records connection delays, and rejects distributed zero-delay cycles with an explicit builder error. No RTI, transport, protocol loop, scheduler semantic change, or distributed execution path has been added.

## Context and Orientation

Boomerang is a Rust workspace. `boomerang_runtime` owns the scheduler, tags, events, actions, reactions, runtime environment, and local enclave wiring. `boomerang_builder` turns user reactor declarations into runtime parts. `boomerang_macros` generates builder-facing code for the `#[reactor]` macro. `boomerang` re-exports the public API.

The current local enclave path is the foundation. In `boomerang_builder/src/reactor.rs`, `ReactorBuilder` has an `is_enclave` boolean. In `boomerang_builder/src/env/build.rs`, `EnvBuilder::build_partition_map` starts a new partition when it discovers a reactor whose builder has `is_enclave` set. `EnvBuilder::build_connections` asks each `ConnectionBuilder` to lower port connections. When a connection crosses partitions, `boomerang_builder/src/connection.rs` creates a source bridge reactor, a target bridge reactor, and an `EnclaveDep`.

At runtime, `boomerang_runtime/src/env/mod.rs` defines `Enclave`, `UpstreamRef`, and `DownstreamRef`. `runtime::crosslink_enclaves` installs local `SendContext` references into upstream and downstream enclaves. `boomerang_runtime/src/sched/mod.rs` constructs a `Scheduler` with `upstream_enclaves` and `downstream_enclaves`. Before processing the next tag, `Scheduler::next` waits for all upstream `LogicalTimeBarrier`s. After processing a tag, `Scheduler::release_tag_downstream` sends tag-release events downstream. This is the local control-plane analogue of a federated TAG/LTC/NET protocol.

The local data plane is in `boomerang_builder/src/connection.rs` and `boomerang_runtime/src/reaction.rs`. `build_enclave_connection_source` resolves the target runtime action and target `SendContext`, then creates `EnclaveSenderReactionFn<T>`. `EnclaveSenderReactionFn<T>::trigger` reads the source port value, computes the target tag, and schedules an `AsyncEvent::Logical` or physical async action directly into the target enclave. A federated sender reaction should preserve the same trigger shape, but replace the target `SendContext` and `AsyncActionRef<T>` with a transport-backed endpoint id and payload codec.

Current `Tag` is in `boomerang_runtime/src/time.rs`. It contains an offset `Duration` and a superdense `microstep`, with sentinel constants `NEVER`, `ZERO`, and `FOREVER`. `Tag::from_physical_time` and `Tag::to_logical_time` convert between offsets and a process-local `std::time::Instant` origin. Federated protocol messages must carry offset/microstep wire tags and separately negotiate physical start time.

## Architecture Proposal

The public model should be instance placement through a concrete `ReactorPlacement` enum. Add a new builder-facing placement type, conceptually `ReactorPlacement::Local`, `ReactorPlacement::Enclave`, and `ReactorPlacement::Federate(FederateSpec)`, while preserving the existing `is_enclave` APIs for compatibility. Add a convenience method on `ReactorBuilderState` such as `add_child_federate(reactor, name, state, spec)` that calls the enum-based placement path. The exact method name can change, but the important choice is that federation is attached to a reactor instance, not to the reactor type generated by `#[reactor]`.

The builder should emit a `FederationPlan` only when the federated feature is enabled. This plan contains stable federate ids, the runtime enclave key for each federate, static upstream/downstream topology, minimum logical delay for each cross-federate edge, and endpoint ids for each serialized connection. A stable endpoint id should be based on builder metadata such as source and target fully-qualified port names plus a deterministic integer assigned during lowering, not on process-local `ActionKey` values alone. The first implementation may require exactly one runtime enclave per federate; the plan shape should not preclude a future federate process from containing multiple local enclaves.

The runtime should keep local `SendContext` and `AsyncActionRef` for in-process scheduling. Federated data forwarding should add separate abstractions: an outbound connection endpoint that serializes a typed value into bytes, an inbound endpoint registry that maps a wire endpoint id to a local `ActionKey` and decoder, and a network client that turns received MSG frames into `AsyncEvent::Logical` events for the scheduler. This avoids pretending a remote action is an `AsyncActionRef<T>`.

The scheduler should gain an optional federated time barrier rather than replacing `LogicalTimeBarrier`. Local upstream barriers still handle same-process enclave dependencies. A federated barrier sends NET requests to the RTI for the scheduler's next local event tag, waits for TAG grants, and reports LTC after a tag is processed. While waiting, it must keep accepting inbound network events from the same `event_rx` path so an arriving MSG can create an earlier event and wake the scheduler, matching the existing `LogicalTimeBarrier::acquire_tag` pattern.

The `boomerang_federated` crate should own the RTI and client protocol. The first RTI is centralized: federates connect, send static neighbor structure, receive a common start time, and use NET/LTC/TAG to advance logical time. RTI state tracks each federate's last completed tag, last granted tag, next event tag, topology delays, and in-transit message tag queues. It grants a TAG only when its earliest future incoming message tag calculation is strictly greater than the federate's requested tag. Because the MVP rejects distributed zero-delay cycles, it does not issue PTAG or ABS.

Async I/O should be considered part of `boomerang_federated`, not part of the reaction scheduler. The RTI can run an async event loop that owns all TCP listeners and client streams. Each federate client can run async reader and writer tasks and bridge into `boomerang_runtime` with bounded synchronous channels, injecting decoded MSG frames as `AsyncEvent::Logical` events. This has practical advantages for the first real transport: one RTI can multiplex many federate sockets without a thread per connection, frame reads and writes can apply backpressure, timeouts and shutdown can be represented as cancelable tasks, and tests can use the same async transport trait with an in-memory implementation. The tradeoff is a Tokio dependency and a sync/async bridge, so default local execution must stay dependency-free and unchanged.

The wire tag should be an enum, not a direct serde derive of `Tag`: `Never`, `Forever`, or `Finite { offset_ns: i128, microstep: u64 }`. Convert to and from `Tag` at the runtime boundary. `offset_ns` is relative to the federation logical start, not to an `Instant`. Static federations start at logical tag zero. The RTI also sends a physical start time as signed Unix epoch nanoseconds so each federate can derive its local scheduler `Instant` origin for wall-clock synchronization. Future transient federates will also receive an effective start tag and will align timers relative to that tag.

Serialization constraints should be explicit at the remote connection boundary. Keep `ReactorData` unchanged for local use. Add a federated connection API that uses serde by default or accepts an explicit typed codec. The first slice should require `T: serde::Serialize + serde::de::DeserializeOwned + Clone + Send + Sync + 'static` for federated payloads. A future wincode adapter can require the corresponding wincode schema read/write traits. If a normal `connect_port` crosses federate placement without a registered codec, `EnvBuilder::into_federated_parts` should fail with a clear `BuilderError`, for example "cross-federate connection source.out -> sink.in requires a federated codec; use connect_federated_port or register a codec for T". This gives a compile-time bound for the explicit federated API and a useful build-time error for legacy code.

Federated MSG data frames can be routed through the RTI or directly between federates. RTI-routed data means every federate opens one logical connection to the RTI, and the RTI forwards both control messages and payload messages. This is simpler to implement, easier to test, better aligned with centralized transient-federate semantics, and gives the RTI direct visibility into in-transit message tags for TAG decisions. It also avoids distributed connection setup, NAT/firewall issues, peer authentication, duplicate reconnect logic, and separate data/control failure modes. Its cost is that the RTI becomes a bandwidth bottleneck and a single data-plane failure point; every data message takes an extra network hop and the RTI must buffer or apply backpressure for payload traffic.

Direct federate-to-federate data channels mean the RTI handles coordination while payload MSG frames go straight from source federate to destination federate. This reduces RTI bandwidth, removes one hop from data latency, and scales better for large payload fanout. Its cost is a more complex first implementation: each federate needs multiple peer connections, endpoint discovery, reconnect behavior, peer authentication later, backpressure per peer, and an explicit way for the RTI to learn about in-transit message tags before granting time. Direct data channels also complicate transient federates because joins, leaves, and absent intervals must be reflected in peer channels as well as RTI state.

The first-slice decision is RTI-routed MSG data. It matches the centralized coordination model, lets the first RTI state machine own in-transit queues without extra acknowledgements from peer channels, and keeps the first distributed hello/delayed tests small. The protocol should still encode source and target federate ids in MSG frames so direct data channels can be added later as an optimization without changing endpoint identity or payload codecs.

## Plan of Work

Milestone 1 adds placement and topology without running a network. Modify `boomerang_builder/src/reactor.rs` so `ReactorBuilder` records `ReactorPlacement` metadata while preserving compatibility with `is_enclave`. Extend `boomerang_builder/src/macro_support.rs` with a placement-aware child builder path and a convenience method for adding federate children. Add builder tests that a child federate becomes an enclave root, gets a stable federate id, and appears in a new `FederationPlan`. Add a distributed topology validation pass that rejects any distributed cycle with no positive logical delay. At the end of this milestone, no scheduler behavior changes, but `cargo test -p boomerang_builder federated` should prove the builder can describe static federations.

Milestone 2 creates `boomerang_federated` with pure protocol, async transport traits, serde payload framing, and RTI state tests. Add the crate to the workspace and keep it optional from the top-level `boomerang` crate. Define `WireTag`, `FederateId`, `EndpointId`, protocol message enums, neighbor/topology data, codec traits, and an RTI state machine that can be tested without sockets. Include in-transit message queues and the static TAG/NET/LTC grant calculation. Add an in-memory async transport implementation before TCP so protocol tests do not depend on the operating system network stack. At the end of this milestone, `cargo test -p boomerang_federated` should pass without starting a scheduler.

Milestone 3 adds runtime hooks but keeps local behavior unchanged by default. In `boomerang_runtime/src/sched/mod.rs`, add a constructor or configuration path that accepts optional federated hooks. In `Scheduler::next`, call the federated barrier before processing a tag and call its LTC method after processing. In `boomerang_runtime/src/event.rs`, add any feature-gated event variants needed for grants or use a small internal adapter that converts grants to existing events. Existing `execute_enclaves` and all non-federated tests must continue to pass without enabling the feature.

Milestone 4 lowers cross-federate connections into serialized endpoints. In `boomerang_builder/src/connection.rs`, branch separately for same-partition, cross-local-enclave, and cross-federate connections. Keep the current `EnclaveSenderReactionFn<T>` for local cross-enclave edges. Add a federated sender reaction that reads the source port, computes the same target tag currently computed for local logical actions, serializes the value, and sends an MSG frame containing endpoint id, tag, and payload bytes. Add an inbound endpoint registry so received MSG frames schedule the target bridge action in the destination enclave.

Milestone 5 adds in-memory distributed execution tests. Use an in-memory reliable ordered transport first so tests are deterministic and do not require binding TCP ports. Route MSG frames through the RTI. Add a source/destination distributed hello test, a delayed connection test that asserts the receiver observes the exact delayed logical tag, and a zero-delay distributed cycle test that asserts the build fails with the explicit unsupported-topology error. Compare these tests to current local enclave behavior where applicable.

Milestone 6 adds TCP smoke coverage. Implement a length-delimited TCP transport using async I/O inside `boomerang_federated`, likely with Tokio plus a small framing utility. Do not introduce a Tokio scheduler path for Boomerang reactions in this slice. Add one ignored or opt-in smoke test that starts an RTI and two federates on localhost, sends one value, and shuts down cleanly.

## Concrete Steps

Work from the repository root `/Users/johhug01/Source/boomerang`.

First, run existing tests before implementation to establish the baseline:

    cargo test

Expect the existing workspace tests to pass. If this fails before any edits, record the failing command and output in `Surprises & Discoveries` before continuing.

For Milestone 1, add feature flags and builder-only types first. Edit `boomerang_builder/Cargo.toml` to add a `federated` feature. Edit `boomerang_builder/src/reactor.rs`, `boomerang_builder/src/macro_support.rs`, and `boomerang_builder/src/env/build.rs` to record placement and emit a `FederationPlan`. Add tests near the existing enclave tests in `boomerang_builder/src/tests.rs`.

For Milestone 2, add a workspace member `boomerang_federated`, its `Cargo.toml`, and `src/lib.rs`. Keep this crate focused on wire types, serde-backed codec traits, async transport traits, and RTI state. Add protocol tests in that crate.

For Milestone 3 and later, make each runtime change behind a feature or optional constructor so existing calls such as `runtime::execute_enclaves(enclaves.into_iter(), config)` keep compiling and behaving the same.

After each milestone, run the narrow tests for the changed crate, then run the affected integration tests. After the MVP is complete, run:

    cargo test
    cargo test -p boomerang_federated
    cargo test -p boomerang --features federated --test federated_equivalence

The exact final integration test file name may differ, but it must include the hello, delayed connection, and zero-delay-cycle decision tests described below.

## Validation and Acceptance

The source/destination distributed hello test builds a root reactor with `Source` and `Destination` as separate federates. `Source` sends a small serde payload such as `String` or `u32` at startup. `Destination` records the received value and tag in state. The federated run must produce the same value and logical tag as the equivalent in-process enclave run.

The delayed connection test connects a federated source output to a federated destination input with `after = Some(Duration::milliseconds(10))`. The destination reaction must observe the payload at `Tag::new(Duration::milliseconds(10), 0)` or the exact tag produced by the current local bridge semantics if microstep handling changes. This test proves topology delays and wire tags are not being treated as wall-clock timestamps.

The zero-delay-cycle decision test builds two federates with A output to B input and B output to A input, both with no positive `after` delay. In the static MVP, building federated runtime parts must fail with a specific unsupported-topology error. The local in-process enclave counterpart, such as `boomerang/tests/enclave_cycle.rs` when it includes a positive delay, remains the behavior oracle for supported cycles. When PTAG/ABS is implemented later, replace or split this test so one case proves the old rejection and another proves constructive zero-delay execution.

Protocol tests must prove `WireTag` round-trips `NEVER`, `ZERO`, a finite offset with microstep, and `FOREVER`. RTI state tests must prove a TAG is granted only when no upstream NET or in-transit message can produce a message at or before the requested tag. A test must show that an in-transit MSG tag blocks a TAG grant until the target federate reports LTC for that tag.

Acceptance for the MVP is not just compilation. A human should be able to run the federated hello test and observe that the destination state contains the sent value at the same logical tag as local enclave execution. A human should also be able to run the zero-delay-cycle test and observe the explicit builder error.

## Idempotence and Recovery

All milestones are additive and can be retried. Feature-gated additions should leave default `cargo test` behavior unchanged. If a new crate or feature causes unrelated tests to fail, first disable the feature and rerun `cargo test` to confirm the regression is confined to federated code.

Do not remove or rewrite the existing local enclave path while implementing this plan. If a runtime hook becomes awkward, keep `Scheduler::new` and `execute_enclaves` intact and add a separate constructor or runner for federated execution. This preserves the local oracle and gives a safe rollback path.

If the protocol design changes, update `Decision Log`, `Plan of Work`, `Interfaces and Dependencies`, and the tests in the same commit or work session. Do not leave the ExecPlan describing stale wire messages or stale public APIs.

## Non-Goals For The Static MVP

The static MVP does not implement transient federate join, leave, or rejoin behavior. It should reserve stable ids and state fields for later absent intervals and effective start tags, but it should reject or ignore transient configuration with an explicit unsupported error.

The static MVP does not implement PTAG or ABS. It must reject unsupported zero-delay distributed cycles at build time.

The static MVP does not support remote physical connections. It should reject cross-federate physical connections until wall-clock and clock-synchronization semantics are designed.

The static MVP does not support multiple local enclaves inside one federate process. It should keep `FederationPlan` and `ReactorPlacement` flexible enough to add that support later, but the first slice may enforce one federate root maps to one runtime enclave.

The static MVP does not implement HMAC authentication, fault tolerance, hot swap, optimized coordination from arXiv 2410.06454, or arbitrary runtime graph mutation.

## Interfaces and Dependencies

In `boomerang_builder`, define placement and build-plan types behind the federated feature. The exact paths can change during implementation, but the resulting public shape should be close to:

    pub struct FederateSpec {
        pub id: String,
        pub transient: bool,
    }

    pub enum ReactorPlacement {
        Local,
        Enclave,
        Federate(FederateSpec),
    }

    pub struct FederationPlan {
        pub federates: Vec<FederateBuildInfo>,
        pub edges: Vec<FederatedEdge>,
        pub endpoints: Vec<FederatedEndpoint>,
    }

In `boomerang_federated`, define wire and protocol types independent of local `Instant`:

    pub enum WireTag {
        Never,
        Finite { offset_ns: i128, microstep: u64 },
        Forever,
    }

    pub enum FederateToRti {
        Hello { federate_id: FederateId, topology: NeighborStructure },
        Net { federate_id: FederateId, tag: WireTag },
        Ltc { federate_id: FederateId, tag: WireTag },
        Msg { source: FederateId, target: FederateId, endpoint: EndpointId, tag: WireTag, payload: Vec<u8> },
        Stop { federate_id: FederateId },
    }

    pub enum RtiToFederate {
        Start { start_unix_epoch_ns: i128 },
        Tag { tag: WireTag },
        Msg { endpoint: EndpointId, tag: WireTag, payload: Vec<u8> },
        Stop,
        Error { message: String },
    }

Define codec traits so the transport and protocol do not commit to serde-only payloads:

    pub trait PayloadEncoder<T>: Send + Sync + 'static {
        fn encode(&self, value: &T) -> Result<Vec<u8>, CodecError>;
    }

    pub trait PayloadDecoder<T>: Send + Sync + 'static {
        fn decode(&self, bytes: &[u8]) -> Result<T, CodecError>;
    }

Provide a serde adapter behind a `serde-codec` feature and a wincode adapter behind a `wincode-codec` feature. The exact trait bounds for the wincode adapter should be taken from the selected wincode version during implementation, not guessed in this plan.

In `boomerang_runtime`, add only the minimal hook needed by the scheduler. A representative shape is:

    pub trait FederatedTimeBarrier: Send {
        fn acquire_tag(
            &mut self,
            tag: Tag,
            event_rx: &crate::Receiver<AsyncEvent>,
        ) -> Option<AsyncEvent>;

        fn logical_tag_complete(&mut self, tag: Tag);
    }

This interface mirrors `LogicalTimeBarrier::acquire_tag`: it can return an inbound async event instead of a grant so the scheduler can handle newly arrived messages before advancing.

For serialization, provide optional default codec adapters for explicit federated connections. The serde adapter can look like:

    pub trait FederatedPayload:
        boomerang_runtime::ReactorData + Clone + serde::Serialize + serde::de::DeserializeOwned
    {
    }

    impl<T> FederatedPayload for T
    where
        T: boomerang_runtime::ReactorData + Clone + serde::Serialize + serde::de::DeserializeOwned
    {
    }

The first transport can be in-memory for tests. TCP should be reliable and ordered. Use a length-delimited framing format and keep it inside `boomerang_federated` so normal `boomerang_runtime` users do not inherit network dependencies. Async I/O is a first-slice decision, so keep Tokio and framing dependencies scoped to `boomerang_federated`.

## Open Questions Requiring User Choice

The first-slice choices from this design round are resolved: async I/O is firm, serde is the initial payload codec, and MSG data frames route through the RTI. Direct federate-to-federate data channels remain a later optimization after TAG/LTC/NET correctness, in-transit queues, and transient-federate state are stable.

## Artifacts and Notes

Important existing files and functions:

    boomerang_builder/src/env/build.rs
      EnvBuilder::build_partition_map
      EnvBuilder::build_connections
      BuilderRuntimeParts::new
      EnvBuilder::into_runtime_parts

    boomerang_builder/src/connection.rs
      ConnectionBuilder::build
      build_enclave_connection_source
      build_enclave_connection_target

    boomerang_runtime/src/reaction.rs
      EnclaveSenderReactionFn<T>
      ConnectionSenderReactionFn<T>
      ConnectionReceiverReactionFn<T>

    boomerang_runtime/src/sched/mod.rs
      Scheduler::new
      Scheduler::next
      Scheduler::release_tag_downstream
      execute_enclaves

    boomerang_runtime/src/sched/barrier.rs
      LogicalTimeBarrier::acquire_tag

    boomerang_runtime/src/time.rs
      Tag
      Tag::delay
      Tag::pre
      Tag::from_physical_time
      Tag::to_logical_time

    boomerang/tests/enclave_cycle.rs
      Existing cross-enclave cycle test with a positive logical delay.

Change note: This file was created as the design ExecPlan for the first implementable Federated Reactors slice. It records the decision to implement a static TAG/LTC/NET federation MVP first, while reserving PTAG/ABS and transient federate behavior for later slices.

Change note: The plan was refined on 2026-07-08 to prefer a `ReactorPlacement` enum, consider async I/O as the transport foundation inside `boomerang_federated`, keep multiple local enclaves per federate as future support, and make federation payload serialization codec-agnostic with serde and wincode adapters.

Change note: The plan was refined again on 2026-07-08 to make async I/O a firm first-slice decision, make serde the first supported payload codec, and document the costs and benefits of RTI-routed versus direct federate-to-federate MSG data routing.

Change note: The plan was refined again on 2026-07-08 to make RTI-routed MSG data frames a firm first-slice decision.
