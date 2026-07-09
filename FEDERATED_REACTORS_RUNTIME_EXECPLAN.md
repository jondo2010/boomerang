# Implement a Live Static Federated Reactor Runtime

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository does not currently contain `PLANS.md` or `.agent/PLANS.md`, although `AGENTS.md` mentions `.agent/PLANS.md`. This document follows the bundled Codex ExecPlan rules. It is intentionally a new plan for the live runtime phase; it does not update or depend on `FEDERATED_REACTORS_EXECPLAN.md`.

## Purpose / Big Picture

After this work, Boomerang can execute a small static federation as a live distributed runtime instead of a manually staged test. A developer can build a root reactor with two child federates, run an in-memory RTI and two federate schedulers, and observe the sink federate receive the same value at the same logical tag as the existing local enclave execution. A later milestone extends the same runner shape to TCP so the RTI and federates exchange real framed protocol messages.

The important behavioral change is that `boomerang_federated::RtiState` stops being only a manually called state machine in tests. It becomes part of a federate client loop that sends `NET`, `LTC`, and `MSG`, receives `TAG` grants and routed `MSG` frames, and drives `boomerang_runtime::Scheduler::new_with_federated_time_barrier`.

The first target is deliberately static and conservative. It supports persistent federates, logical cross-federate messages routed through the RTI, `NET`, `TAG`, `LTC`, `MSG`, `Stop`, and positive-delay cycles. It continues to reject distributed zero-delay cycles until `PTAG` and `ABS` are implemented. A federate is a reactor instance placed with `ReactorPlacement::Federate`; in this first live runner, one federate maps to one runtime enclave.

Before the live runner is implemented, this plan now first aligns local enclave boundaries and federated boundaries at the builder metadata layer. A federated reactor should be treated conceptually as a remote enclave, but only for topology, boundary-edge, and validation metadata. Local enclave delivery and remote federated delivery remain separate implementations because they use different runtime mechanisms.

## Progress

- [x] (2026-07-09 08:47Z) Created a fresh runtime-phase ExecPlan that starts from the current committed milestones and does not reuse the old plan as the implementation guide.
- [x] (2026-07-09 08:47Z) Grounded the repository with CodeStory local navigation and confirmed packet/search are unavailable because the sidecar retrieval mode is degraded.
- [x] (2026-07-09 09:58Z) Revised the plan to put shared inter-partition boundary metadata before the runtime/protocol bridge milestone.
- [x] (2026-07-09 10:09Z) Extracted shared inter-partition topology and boundary-edge metadata for local enclave and federated boundaries.
- [x] (2026-07-09 10:39Z) Added checked runtime/protocol tag and delay bridge utilities in `boomerang_federated`, plus builder-owned topology and endpoint route extraction.
- [x] (2026-07-09 11:32Z) Added a live `FederatedOutboundChannel`/`FederatedOutboundReceiver` pair in `boomerang_runtime` while preserving `FederatedOutboundBuffer`.
- [x] (2026-07-09 12:33Z) Added a pure in-memory RTI session loop in `boomerang_federated` that drives real protocol `Hello`, `Start`, `NET`, `TAG`, `LTC`, `MSG`, `Stop`, and `Error` frames over the existing transport traits.
- [x] (2026-07-09 13:03Z) Refactored the Milestone 4 RTI session loop to async Tokio tasks/channels so the same session shape can be reused by TCP transport.
- [x] (2026-07-09 13:19Z) Replaced the custom federated transport traits with `futures_util::Sink` and `futures_util::Stream` bounds for the RTI session and in-memory/TCP transports.
- [x] (2026-07-09 13:39Z) Replaced manual `TcpTransport` protocol JSON encode/decode with `tokio-serde` layered over the existing `tokio-util` length-delimited TCP frames.
- [x] (2026-07-09 13:44Z) Removed the `TcpTransport` wrapper and exposed the direct `JsonProtocolFrameTransport` tokio-serde transport using `LengthDelimitedCodec::new()`.
- [x] (2026-07-09 13:55Z) Replaced the custom Tokio-backed in-memory transport wrappers with `futures-channel` MPSC endpoint types and relaxed the RTI session transport bounds to ecosystem `Sink` plus `TryStream` with errors convertible into `TransportError`.
- [ ] Implement a federate client bridge that attaches protocol sessions to scheduler barriers.
- [ ] Implement an in-memory static RTI/federate runner that uses real scheduler barriers.
- [ ] Define and test shutdown and no-future-event behavior.
- [ ] Promote TCP smoke transport into a reusable RTI/federate runtime path.
- [ ] Add broader correctness tests for multi-hop topologies, positive-delay cycles, same-tag messages, and rejected unsupported semantics.

## Surprises & Discoveries

- Observation: The current code already has the right low-level hooks, but they are not connected into a live runner.
  Evidence: `boomerang_runtime::Scheduler::new_with_federated_time_barrier` exists behind the `federated` feature, and builder tests manually drain `FederatedOutboundBuffer`, route `MSG` through `RtiState`, and schedule through `FederatedInboundEndpointRegistry`.

- Observation: The current TCP test is a transport smoke test, not a runtime proof.
  Evidence: `boomerang_federated/src/transport.rs` contains an ignored localhost test that manually accepts two clients, handles `Hello`, forwards one `MSG`, and exchanges `Stop`, but it does not construct Boomerang schedulers or use a federated time barrier.

- Observation: The outbound runtime bridge is pull-only.
  Evidence: `FederatedOutboundBuffer` stores commands in an `Arc<Mutex<Vec<_>>>` and exposes `drain`, `len`, and `is_empty`; there is no blocking receive, wakeup, bounded capacity, or backpressure.

- Observation: The builder currently rejects all cross-federate physical connections and distributed zero-delay cycles, which is the correct boundary for this plan.
  Evidence: `boomerang_builder/src/env/build.rs` rejects cross-federate physical edges and validates zero-delay federated cycles; `boomerang_builder/src/connection.rs` also rejects physical cross-federate lowering.

- Observation: The builder already has two parallel notions of crossing a runtime partition boundary.
  Evidence: `boomerang_builder/src/env/build.rs` records local cross-enclave dependencies as `EnclaveDep` values and federated dependencies as `FederationPlan` edges, while `boomerang_builder/src/connection.rs` chooses either `EnclaveSenderReactionFn` or `FederatedSenderReactionFn` when source and target partitions differ.

- Observation: CodeStory was available but could not be used for repo-specific grounding in this session.
  Evidence: `codestory://status` reported `workspace_mismatch`: active root `/Users/johhug01/Source/boomerang`, served root `/Users/johhug01/Source/arm-ppu`. Direct source reads were used after that supported MCP path was blocked.

- Observation: Milestone 1 can keep federation-specific delivery unchanged while moving validation and metadata extraction to one builder-owned source of truth.
  Evidence: `boomerang_builder/src/inter_partition.rs` now defines `PartitionRoot`, `PartitionRootKind`, `BoundaryKind`, `InterPartitionEdge`, and `InterPartitionPlan`; `boomerang_builder/src/env/build.rs` builds that plan first, derives local `EnclaveDep` values from local boundary edges, and derives `FederationPlan` from federated boundary edges. `InterPartitionEdge` keeps source and target port keys rather than caching derived port FQNs.

- Observation: In this Milestone 2 session, CodeStory was pointed at `/Users/johhug01/Source/boomerang`; local navigation was fresh, while packet/search remained blocked by degraded sidecar retrieval.
  Evidence: `codestory://status` reported `project_root` `/Users/johhug01/Source/boomerang`, `index_freshness.status` `fresh`, and `allowed_surfaces.packet/search.allowed` `false`.

- Observation: Runtime `Duration` can represent values that cannot be serialized as `WireDelay`.
  Evidence: the bridge test constructs `runtime::Duration::nanoseconds_i128(i128::from(u64::MAX) + 1)` and verifies `WireDelay::try_from` rejects it because `WireDelay` stores nanoseconds in `u64`.

- Observation: Rust's orphan rules allow the checked runtime/protocol conversions to be expressed as `TryFrom` impls in `boomerang_federated`.
  Evidence: `boomerang_federated` owns `WireTag` and `WireDelay`, so the `runtime` feature now provides `TryFrom<boomerang_runtime::Tag> for WireTag`, `TryFrom<WireTag> for boomerang_runtime::Tag`, and `TryFrom<boomerang_runtime::Duration> for WireDelay` without adding any dependency from `boomerang_runtime` to `boomerang_federated`.

- Observation: In this Milestone 3 session, CodeStory was pointed at `/Users/johhug01/Source/boomerang`; local navigation was fresh, while packet/search remained blocked by degraded sidecar retrieval.
  Evidence: `codestory://status` reported `project_root` `/Users/johhug01/Source/boomerang`, `index_freshness.status` `fresh`, and `allowed_surfaces.packet/search.allowed` `false`; `mcp__codestory.ground` returned repository coverage for the Boomerang workspace.

- Observation: `kanal` already provides the exact synchronous receive shape needed for the runtime channel wrapper.
  Evidence: `kanal::Receiver::recv` blocks until a command or close, and `kanal::Receiver::try_recv` returns `Result<Option<T>, ReceiveError>`, matching the planned `FederatedOutboundReceiver` API without adding a runtime dependency.

- Observation: In this Milestone 4 session, CodeStory local navigation was fresh for `/Users/johhug01/Source/boomerang`, but packet/search stayed blocked by stale sidecar retrieval even after the MCP auto-repair path started.
  Evidence: `codestory://status` reported `project_root` `/Users/johhug01/Source/boomerang`, `index_freshness.status` `fresh`, `allowed_surfaces.packet/search.allowed` `false`, and `status_resource_auto_repair.result.status` `started`; direct source reads were used after the allowed `ground` surface.

- Observation: The first protocol session test must make the source advertise a later `NET` before the sink's `LTC(ZERO)` can release the sink's blocked `TAG(ZERO)`.
  Evidence: `RtiState::earliest_incoming_message_tag` considers both in-transit messages and the upstream federate's `next_event`; the session test sends source `NET([0ns+1])` after source `MSG(ZERO)`, then sink `LTC(ZERO)` clears the in-transit message and triggers the pending sink grant.

- Observation: The initial synchronous Milestone 4 session used the correct protocol behavior but the wrong execution shape for the TCP goal.
  Evidence: `StaticRtiSession::run` now returns a future, session reader fan-in uses `tokio::sync::mpsc`, reader loops are spawned with `tokio::spawn`, and in-memory protocol tests use async channel-backed endpoints with `#[tokio::test]`.

- Observation: The CodeStory MCP transport closed during the follow-up transport-trait refactor.
  Evidence: `codestory://status` through MCP resources failed with `Transport closed`, and `mcp__codestory.ground` returned `tool call failed ... Transport closed`; direct source reads were used for the refactor.

- Observation: `boomerang_federated` already had enough async ecosystem surface to remove the crate-specific transport traits without adding another dependency.
  Evidence: `boomerang_federated/Cargo.toml` already depended on `futures-util` with the `sink` feature, and `cargo test -p boomerang_federated` passed after `InMemoryTransport`, split in-memory halves, and the TCP protocol transport used `futures_util::Sink` and `futures_util::Stream`.

- Observation: `tokio-serde` matches the desired TCP layering better than a hand-written TCP JSON wrapper.
  Evidence: `tokio-serde` 0.9.0 provides `SymmetricallyFramed` and `formats::SymmetricalJson`, while its documentation expects a separate framed byte transport such as a length-delimited `tokio-util` transport underneath. `JsonProtocolFrameTransport` is now a direct alias for `tokio_serde::SymmetricallyFramed<tokio_util::codec::Framed<TcpStream, LengthDelimitedCodec>, ProtocolFrame, SymmetricalJson<ProtocolFrame>>`, and `cargo test -p boomerang_federated` passed.

- Observation: The ignored localhost TCP smoke test still needs sandbox approval, but the `tokio-serde` TCP path works after that approval.
  Evidence: `cargo test -p boomerang_federated tcp_smoke -- --ignored` first failed at `TcpListener::bind("127.0.0.1:0")` with `Operation not permitted`; rerunning the same command with approval passed with `1 passed`.

- Observation: Once `tokio-serde` owned protocol JSON and `LengthDelimitedCodec::new()` owned frame sizing, the `TcpTransport` wrapper had no remaining behavior worth preserving.
  Evidence: `boomerang_federated/src/transport.rs` now exposes `json_protocol_frame_transport(TcpStream) -> JsonProtocolFrameTransport`; the TCP smoke test uses `TcpStream::connect` and the free constructor directly.

- Observation: During the `futures-channel` in-memory transport follow-up, the CodeStory MCP transport was unavailable.
  Evidence: `functions.list_mcp_resources(server="codestory")` failed with `Transport closed`; direct source reads were used after the supported MCP status path was blocked.

- Observation: `futures-channel` provides the missing direct in-memory async transport pieces that `tokio-util` and `tokio-serde` do not.
  Evidence: `futures_channel::mpsc::UnboundedSender<T>` implements `futures_util::Sink<T>` behind the `sink` feature, and `UnboundedReceiver<T>` implements `Stream`; `boomerang_federated/src/transport.rs` now exposes those endpoints through type aliases instead of local `Sink`/`Stream` impls.

- Observation: Direct ecosystem transport endpoints should not be forced to use `TransportError` as their native error type.
  Evidence: `futures_channel::mpsc::UnboundedSender<T>` uses `mpsc::SendError`, while `tokio-serde` over `LengthDelimitedCodec` uses the underlying framed transport error. `StaticRtiSession` now accepts `Sink<ProtocolFrame>` and `TryStream<Ok = ProtocolFrame>` where both error types implement `Into<TransportError>`.

## Decision Log

- Decision: Keep `boomerang_runtime` independent of `boomerang_federated`.
  Rationale: The runtime should remain protocol-free and Tokio-free. It only needs a scheduler barrier trait, endpoint registry, and outbound sink trait. Protocol state, transport framing, and RTI loops belong in `boomerang_federated` or feature-gated builder integration.
  Date/Author: 2026-07-09 / Codex

- Decision: Implement the first live runner in memory before adding TCP orchestration.
  Rationale: In-memory channels make logical-time correctness deterministic and avoid operating-system socket permission issues. TCP should reuse the same client and RTI session logic after the in-memory path proves `NET`, `TAG`, `LTC`, `MSG`, and shutdown behavior.
  Date/Author: 2026-07-09 / Codex

- Decision: Continue to route first-slice `MSG` payloads through the RTI.
  Rationale: Central routing lets the RTI observe in-transit message tags directly and keeps the first live runner small. Direct federate-to-federate data channels can be added later as an optimization after correctness is established.
  Date/Author: 2026-07-09 / Codex

- Decision: Treat no-future-local-event as `NET(FOREVER)` in the static runner.
  Rationale: A federate that has no local event must still tell the RTI that it will not spontaneously produce future messages below infinity. Without this, downstream federates can block forever waiting for information that will never arrive.
  Date/Author: 2026-07-09 / Codex

- Decision: Keep `PTAG` and `ABS` out of this plan except as explicit non-goals and rejected topologies.
  Rationale: `PTAG` means a provisional grant to process a tag while equal-tag input may still arrive. `ABS` means an upstream port is absent through a tag. Both are required for constructive zero-delay distributed cycles, but the current runtime does not emit per-port absence messages. Shipping a partial implementation would be misleading.
  Date/Author: 2026-07-09 / Codex

- Decision: Add a builder-owned inter-partition boundary metadata milestone before runtime bridge utilities.
  Rationale: Local enclave boundaries and federated boundaries should share topology and validation vocabulary before endpoint route extraction, RTI topology conversion, and runner APIs harden around federated-only names. The unification must stop at metadata: local cross-enclave delivery still uses `SendContext` and `EnclaveSenderReactionFn`, while federated delivery still uses endpoint ids, outbound sinks, inbound endpoint registries, and RTI protocol messages.
  Date/Author: 2026-07-09 / Codex

- Decision: Feature-gate `boomerang_builder/src/federation.rs` as a whole and keep common inter-partition metadata in `boomerang_builder/src/inter_partition.rs`.
  Rationale: The shared boundary vocabulary is useful for ordinary local enclave builds, while the federated plan and endpoint names should live in the federated builder module. This avoids per-item cfg noise inside `federation.rs` and keeps feature boundaries at module granularity.
  Date/Author: 2026-07-09 / Codex

- Decision: Do not store source or target port FQNs on `InterPartitionEdge`.
  Rationale: Boundary metadata should keep stable builder keys and semantic fields. FQNs are derived presentation or endpoint metadata and can be created from the builder when `FederationPlan` or debug output needs them.
  Date/Author: 2026-07-09 / Codex

- Decision: Put runtime/protocol conversion impls in `boomerang_federated` behind its `runtime` feature, while keeping builder plan-to-topology and route extraction in `boomerang_builder`.
  Rationale: Runtime `Tag`/`Duration` conversion is not builder logic. `boomerang_federated` owns the wire protocol types and can implement `TryFrom` without making `boomerang_runtime` depend on protocol code. Builder still owns conversion from `FederationPlan` metadata into protocol topology and endpoint route metadata because that mapping depends on builder-produced federate and endpoint ids.
  Date/Author: 2026-07-09 / Codex

- Decision: Represent invalid bridge conversions as `BuilderError::FederationBridgeError`.
  Rationale: Builder APIs still need to report failed protocol topology extraction through `BuilderError`, including errors propagated from `boomerang_federated::RuntimeBridgeError` and malformed federation plan metadata such as duplicate or missing endpoint routes.
  Date/Author: 2026-07-09 / Codex

- Decision: Implement the Milestone 3 live outbound sink as an unbounded `kanal` channel pair in `boomerang_runtime`.
  Rationale: The federate bridge needs wakeup and blocking/nonblocking receive semantics now, but capacity and backpressure policy belong with later client/session design. The existing `FederatedOutboundBuffer` remains the deterministic drainable test helper.
  Date/Author: 2026-07-09 / Codex

- Decision: Initially keep Milestone 4 in `boomerang_federated` as a synchronous pure protocol session over `FrameSink<ProtocolFrame>` and `FrameStream<ProtocolFrame>`, with split in-memory transport halves for deterministic tests.
  Rationale: The milestone needed real session flow and RTI routing, not scheduler orchestration. Splitting the in-memory transport let reader threads block on federate input while the central RTI loop could still send `Start`, `TAG`, routed `MSG`, `Stop`, and `Error` frames through the same transport traits. This decision was superseded later the same day by the async Tokio session-loop decision below.
  Date/Author: 2026-07-09 / Codex

- Decision: Replace the synchronous Milestone 4 session loop with an async Tokio loop before building the runtime federate client bridge.
  Rationale: The end goal is RTI sessions over TCP/Tokio. Keeping a synchronous session would force later adapter code or a second rewrite during TCP promotion. Making `StaticRtiSession::run` async now keeps Milestone 4 pure protocol while aligning it with the TCP protocol transport.
  Date/Author: 2026-07-09 / Codex

- Decision: Use `futures_util::Sink` and `futures_util::TryStream` as the transport contract for RTI sessions instead of maintaining crate-specific `FrameSink` and `FrameStream` traits.
  Rationale: The RTI session should be transport-agnostic, but `Sink` and result-bearing streams are the ecosystem-standard async contracts already used by framed TCP transports and test clients. Removing the custom traits reduces adapter code before the later TCP/Tokio runtime path is built.
  Date/Author: 2026-07-09 / Codex

- Decision: Add `tokio-serde` with its `json` feature for TCP `ProtocolFrame` serialization instead of adding `tokio-serde-json` or keeping manual serde calls in a TCP wrapper.
  Rationale: `tokio-serde` is the current Tokio 1-compatible crate and composes directly with `tokio-util` length-delimited framing. `tokio-serde-json` is an older format-specific adapter and would be a worse fit for this workspace's Tokio 1 stack. Keeping JSON serialization in `tokio-serde` lets the TCP path use standard `Sink` and `Stream` implementations from the composed transport stack.
  Date/Author: 2026-07-09 / Codex

- Decision: Remove the `TcpTransport` newtype and use `LengthDelimitedCodec::new()` for the default TCP frame length.
  Rationale: After adding `tokio-serde`, the wrapper only forwarded `Sink`/`Stream`, constructed the inner framed transport, and tracked a custom max-frame length. The session loop already accepts standard `Sink`/`Stream` endpoints. A public `JsonProtocolFrameTransport` alias plus `json_protocol_frame_transport(TcpStream)` constructor exposes the reusable transport without duplicating ecosystem behavior.
  Date/Author: 2026-07-09 / Codex

- Decision: Use `futures-channel` MPSC endpoints directly for pure in-memory protocol transports.
  Rationale: `futures-channel` supplies an async `Sink` sender and `Stream` receiver without tying the transport helper to Tokio internals. Keeping `InMemoryFrameSink` as `UnboundedSender` removes local sink forwarding code; mapping the receiver into `Result<_, TransportError>` is the only adapter needed for the session's `TryStream` contract.
  Date/Author: 2026-07-09 / Codex

## Outcomes & Retrospective

This plan starts after the builder, protocol, endpoint, scheduler hook, in-memory smoke, and TCP smoke groundwork has landed. The expected outcome is a live in-memory static federation that can be run by one test or helper and then a TCP-backed variant that uses the same protocol session logic. On 2026-07-09, the plan was revised so the first implementation milestone extracts shared inter-partition boundary metadata before adding protocol bridge utilities.

Milestone 1 is complete. `BuilderRuntimeParts` now carries an `inter_partition_plan`; local cross-enclave `EnclaveDep` values and federated `FederationPlan` endpoint/edge metadata are both derived from this shared plan. Common inter-partition metadata lives in non-gated `boomerang_builder/src/inter_partition.rs`, while federated plan metadata lives in module-gated `boomerang_builder/src/federation.rs`. Local delivery still lowers to `EnclaveSenderReactionFn`, federated delivery still lowers to `FederatedSenderReactionFn` plus outbound/inbound endpoint runtime pieces, and no RTI/session/tag bridge utilities were added. `InterPartitionEdge` records source and target port keys, not derived port FQN strings; `FederationPlan` generation computes FQNs from the builder when endpoint metadata is created. Validation passed with `cargo test -p boomerang_builder --features federated`, `cargo test -p boomerang_runtime --features federated`, and `cargo test -p boomerang_federated`; extra checks `cargo test -p boomerang_builder` and `cargo check -p boomerang --features federated` also passed.

Milestone 2 is complete. `boomerang_federated/src/runtime_bridge.rs` now provides checked `TryFrom` impls for `runtime::Tag` <-> `boomerang_federated::WireTag` and `runtime::Duration` -> `WireDelay` behind the `boomerang_federated/runtime` feature. Negative finite tags are rejected except for sentinel `Tag::NEVER`/`WireTag::NEVER`; wire offsets outside `runtime::Duration`, microsteps outside `usize`, and delays outside nonnegative `u64` nanoseconds return `boomerang_federated::RuntimeBridgeError`. `boomerang_builder/src/federation.rs` keeps builder-owned `federation_topology_from_plan` and `federated_routes_from_plan`, and maps `RuntimeBridgeError` into `BuilderError::FederationBridgeError` when extracting topology. Route extraction maps runtime `FederatedEndpointId` values to source and target protocol `FederateId` values and checks endpoint/edge consistency. The manual federated builder tests now use `TryFrom` for tag conversion and the builder route/topology helpers instead of local ad hoc conversion helpers. Validation passed with `cargo test -p boomerang_federated --features runtime`, `cargo test -p boomerang_builder --features federated`, `cargo test -p boomerang_runtime --features federated`, `cargo test -p boomerang_federated`, and `cargo check -p boomerang --features federated`.

Milestone 3 is complete. `boomerang_runtime/src/federated.rs` now defines `FederatedOutboundChannel` and `FederatedOutboundReceiver` behind the existing runtime `federated` feature. `FederatedOutboundChannel::pair()` returns a sink and receiver backed by an unbounded `kanal` channel; the sink implements `FederatedOutboundSink`, the receiver exposes blocking `recv` and nonblocking `try_recv`, and channel send/receive failures map to `FederatedEndpointError`. `FederatedOutboundBuffer` remains unchanged and available for deterministic tests. Runtime unit tests prove that `FederatedOutboundSink::send` delivers the exact command through `try_recv`, wakes a blocking receiver, and that the buffer still drains commands. No `boomerang_runtime` dependency on `boomerang_federated`, RTI/session/client bridge, TCP transport, builder-lowered outbound behavior replacement, or local/federated delivery behavior change was added. Validation passed with `cargo test -p boomerang_runtime --features federated`, `cargo test -p boomerang_builder --features federated`, `cargo test -p boomerang_federated`, and `cargo check -p boomerang --features federated`.

Milestone 4 is complete. `boomerang_federated/src/session.rs` now defines `StaticRtiSession`, `RtiSessionEndpoint`, and `SessionError`. The session owns `RtiState`, validates that keyed persistent endpoints match the static topology, receives matching `Hello` frames and neighbor structures from every federate, sends `Start`, drives `NET`/`TAG` and `LTC` through `RtiState`, validates and routes topology-backed `MSG` frames, sends `Stop` after all federates stop, and sends `RtiToFederate::Error` before returning a protocol error for unexpected frames. `StaticRtiSession::run` is async, uses Tokio reader tasks and `tokio::sync::mpsc` fan-in, and is generic over `futures_util::Sink<ProtocolFrame>` plus `futures_util::TryStream<Ok = ProtocolFrame>` where transport errors convert into `TransportError`, so direct in-memory endpoints and future TCP endpoints do not need local wrapper impls just to normalize errors. `boomerang_federated/src/transport.rs` now exposes `InMemoryFrameSink` as `futures_channel::mpsc::UnboundedSender`, `InMemoryFrameStream` as a mapped `futures_channel::mpsc::UnboundedReceiver`, and `InMemoryTransport` as a pair of those halves; the previous custom in-memory `Sink`/`Stream` structs and impls are gone. The TCP path now exposes `JsonProtocolFrameTransport` directly as a `tokio-serde` JSON transport over `tokio-util` length-delimited TCP framing, plus `json_protocol_frame_transport(TcpStream)` as the constructor. There is no `TcpTransport` wrapper and no custom TCP max-frame policy; the path uses `LengthDelimitedCodec::new()` defaults. Pure protocol tests script two federates over in-memory `ProtocolFrame` transports: the source/sink test covers `Hello`, `Start`, sink `NET(ZERO)`, source `NET(ZERO)` and source `TAG(ZERO)`, routed source `MSG(ZERO)`, source `NET([0ns+1])`, sink `LTC(ZERO)` and pending sink `TAG(ZERO)`, then `Stop`; a second test verifies protocol error delivery for an unexpected federate frame. The ignored localhost TCP smoke test also passes when rerun with sandbox approval. No Boomerang schedulers, runtime federate client bridge, `FederatedTimeBarrier` integration, TCP orchestration, builder manual test replacement, or `boomerang_runtime` dependency on `boomerang_federated` was added. Validation passed with `cargo test -p boomerang_federated`, `cargo test -p boomerang_builder --features federated`, `cargo test -p boomerang_runtime --features federated`, and `cargo check -p boomerang --features federated`.

## Context and Orientation

Boomerang is a Rust workspace rooted at `/Users/johhug01/Source/boomerang`. The important crates for this plan are `boomerang_builder`, `boomerang_runtime`, `boomerang_federated`, and the top-level `boomerang` re-export crate.

`boomerang_builder` turns reactor declarations into runtime parts. A reactor is a user-defined unit with ports, actions, and reactions. A port carries data. An action schedules data at a logical tag. A reaction is code that runs when a trigger is present. A logical tag is Boomerang's event time, represented by `boomerang_runtime::Tag` as an offset from logical start plus a microstep.

`boomerang_runtime` owns the scheduler. A scheduler executes one enclave. An enclave is a runtime partition with its own event queue and reaction graph. The current local enclave path uses `runtime::crosslink_enclaves` to connect enclave schedulers in one process. For federation, a remote federate is modeled as an enclave whose cross-federate messages go through protocol frames instead of direct local `SendContext` calls.

An inter-partition boundary is any logical connection whose source port and target port are in different enclave roots after the builder computes the `PartitionMap`. In the local enclave case, the builder records an `EnclaveDep` and lowers the sender to `runtime::EnclaveSenderReactionFn`. In the federated case, the builder records a `FederationPlan` edge and lowers the sender to `runtime::FederatedSenderReactionFn`. The first milestone makes this shared shape explicit without merging the two delivery paths.

`boomerang_federated` owns protocol and RTI primitives. The RTI is the runtime infrastructure process or loop that decides when each federate may advance logical time. It receives `NET` messages that say "my next local event tag is T", `LTC` messages that say "I completed tag T", and `MSG` messages that carry data for another federate at tag T. It sends `TAG` grants that say "it is safe to process tag T" and forwards `MSG` frames to targets. It tracks in-transit messages so it does not grant a federate past a tag where a routed message may still arrive.

The existing code already contains these foundations:

`boomerang_builder/src/reactor.rs` defines `ReactorPlacement`, including `ReactorPlacement::Federate(FederateSpec)`.

`boomerang_builder/src/macro_support.rs` defines `add_child_federate`, which creates a federate child while preserving enclave behavior.

`boomerang_builder/src/inter_partition.rs` defines `InterPartitionPlan`, `PartitionRoot`, `InterPartitionEdge`, and `BoundaryKind`. The shared plan is static builder metadata for all inter-partition boundary edges.

`boomerang_builder/src/federation.rs` defines `FederationPlan`, `FederateBuildInfo`, `FederatedEdge`, and `FederatedEndpoint`. This feature-gated plan is derived metadata describing federates and cross-federate endpoint edges.

`boomerang_builder/src/env/build.rs` builds `InterPartitionPlan`, derives local `EnclaveDep` values and federated `FederationPlan` values from it, rejects transient federates, rejects mixed local/federated boundaries, rejects cross-federate physical connections, and rejects distributed zero-delay cycles.

`boomerang_builder/src/connection.rs` lowers cross-federate logical `connect_port` calls into `FederatedSenderReactionFn` plus inbound endpoint registry entries. It keeps ordinary same-partition and local cross-enclave paths unchanged.

`boomerang_runtime/src/federated.rs` defines runtime endpoint IDs, payload encoder and decoder traits, `FederatedOutboundBuffer`, `FederatedOutboundSink`, and `FederatedInboundEndpointRegistry`.

`boomerang_runtime/src/sched/barrier.rs` defines the feature-gated `FederatedTimeBarrier` trait. A scheduler calls `acquire_tag(tag, event_rx)` before processing a logical tag and `logical_tag_complete(tag)` after processing that tag.

`boomerang_runtime/src/sched/mod.rs` defines `Scheduler::new_with_federated_time_barrier`, `Scheduler::event_loop`, and `execute_enclaves`. `execute_enclaves` currently always constructs ordinary local schedulers and does not use federated barriers.

`boomerang_federated/src/protocol.rs` defines `WireTag`, `WireDelay`, `FederateId`, `EndpointId`, `FederatedTopology`, `FederateToRti`, `RtiToFederate`, and `ProtocolFrame`.

`boomerang_federated/src/rti.rs` defines `RtiState`, a deterministic state machine for `TAG`, `NET`, `LTC`, and `MSG` decisions.

`boomerang_federated/src/transport.rs` defines in-memory transport endpoints and `JsonProtocolFrameTransport` for length-delimited JSON `ProtocolFrame` values over TCP. These transports implement the standard async `futures_util::Sink` and `futures_util::Stream` traits rather than crate-specific transport traits. TCP protocol JSON serialization uses `tokio-serde`, and byte framing uses `tokio-util::codec::LengthDelimitedCodec`.

`boomerang_builder/src/tests/federated.rs` contains the current strongest equivalence tests. They manually run source and sink enclaves separately, drain `FederatedOutboundBuffer`, route commands through `RtiState`, schedule through `FederatedInboundEndpointRegistry`, and compare values and tags against local enclave execution. The live runner must replace this manual staging with a reusable implementation.

Terms used in this plan:

`Federate` means a reactor instance placed with `ReactorPlacement::Federate`. In this plan, each federate maps to exactly one runtime enclave.

`Partition root` means the reactor key that owns an enclave in the builder's `PartitionMap`. A local enclave root is used for same-process scheduling. A federated partition root is a reactor with a `FederateSpec`.

`Boundary edge` means a connection from one partition root to another. A boundary edge can be a same-process local enclave edge or a federated edge that must cross the RTI.

`RTI` means the central logical-time coordinator.

`NET` means next event tag. A federate sends it to tell the RTI the earliest local tag it wants to process.

`TAG` means a grant from the RTI that lets a federate process that tag.

`LTC` means logical tag complete. A federate sends it after all work at a tag is done.

`MSG` means a tagged data message carrying endpoint id and payload bytes.

`In-transit MSG` means a message the RTI has accepted or forwarded but whose target federate has not yet acknowledged by sending `LTC` at or beyond the message tag.

`PTAG` means provisional tag grant. It is not implemented here.

`ABS` means absence message. It is not implemented here.

## Plan of Work

Milestone 1 extracts shared inter-partition topology and boundary-edge metadata in `boomerang_builder`. The goal is to make local enclave edges and federated edges two kinds of the same builder-level concept without changing runtime delivery. Add or refactor builder-owned data so a future reader can ask: which partition roots exist, which connections cross partitions, which boundary kind each edge has, what source and target ports participate, and what logical delay applies. A local boundary edge must still lower to `EnclaveDep` plus `runtime::EnclaveSenderReactionFn`; a federated boundary edge must still lower to `FederationPlan`, endpoint metadata, `runtime::FederatedSenderReactionFn`, and inbound endpoint registry entries. At the end of this milestone, existing local enclave tests and federated builder tests should still pass, cross-federate physical edges should still be rejected, and distributed zero-delay cycles should still be rejected.

Milestone 2 creates explicit bridge utilities between builder/runtime metadata and protocol metadata. Add checked conversion functions for `boomerang_runtime::Tag` and `boomerang_federated::WireTag`. These functions must reject negative finite tags except `Tag::NEVER`, reject offsets that do not fit in runtime `Duration`, and reject microsteps that do not fit in runtime `usize`. Add conversion from `boomerang_builder::FederationPlan` or the federated subset of the shared boundary metadata to `boomerang_federated::FederatedTopology` in a builder-owned module so `boomerang_federated` does not depend on `boomerang_builder`. Add route metadata that maps runtime endpoint ids to source and target federate ids. At the end of this milestone, existing manual tests should use the new conversion helpers instead of local ad hoc functions.

Milestone 3 adds a live outbound command sink. The current `FederatedOutboundBuffer` is useful for tests but does not wake a client. Add a feature-gated runtime type such as `FederatedOutboundChannel` in `boomerang_runtime/src/federated.rs` that implements `FederatedOutboundSink` and internally uses a bounded or unbounded channel. It should expose a blocking `recv` or timeout-aware `recv_timeout` API usable by a non-async scheduler bridge. Keep `FederatedOutboundBuffer` for deterministic tests. At the end of this milestone, `FederatedSenderReactionFn` can be constructed with either the old buffer or the new channel, and a unit test proves that a reaction-emitted outbound command wakes a receiver.

Milestone 4 implements an in-memory RTI session loop in `boomerang_federated`. Add a module such as `boomerang_federated/src/session.rs`. It should own `RtiState` plus per-federate transport endpoints. For in-memory operation, use `futures_util::Sink<ProtocolFrame>` and `futures_util::TryStream<Ok = ProtocolFrame>` so the same session contract can later run over TCP/Tokio while allowing concrete transports to keep their native error types. The RTI session receives `Hello` from all persistent federates, sends `Start`, routes `MSG`, handles `NET`, sends `TAG`, handles `LTC`, tracks in-transit messages through `RtiState`, and handles `Stop`. This milestone should not construct Boomerang schedulers yet. Its tests can be pure protocol tests with scripted federate clients that request tags and send messages.

Milestone 5 implements a federate client bridge that attaches one runtime scheduler to one federate id. This bridge lives outside `boomerang_runtime`; put it in `boomerang_federated` if it can avoid depending on `boomerang_builder`, or in `boomerang_builder` behind the `federated` feature if it needs `BuilderRuntimeParts` and `FederationPlan`. The bridge owns a `FederatedTimeBarrier` implementation. When `acquire_tag(tag, event_rx)` is called, it converts `tag` to `WireTag`, sends `NET`, waits for a matching `TAG`, and keeps accepting inbound protocol frames. If it receives a `MSG`, it schedules the decoded payload through `FederatedInboundEndpointRegistry` and returns an interrupting `AsyncEvent` or otherwise wakes the scheduler so the newly arrived event can be processed. When `logical_tag_complete(tag)` is called, it sends `LTC`. The bridge also drains or receives outbound commands from the runtime sink and sends `MSG` frames to the RTI with source federate id, target federate id, endpoint id, wire tag, and payload.

Milestone 6 adds a public in-memory static runner for tests and early users. A representative API can live behind `boomerang_builder/federated` and take `BuilderRuntimeParts`, `runtime::Config`, and a codec-enabled federation plan. It partitions the built enclaves by federate id, starts one scheduler thread per federate using `Scheduler::new_with_federated_time_barrier`, starts one in-memory RTI loop, and returns the final runtime `Env` values keyed by enclave key, like `execute_enclaves` does. The existing manual in-memory tests in `boomerang_builder/src/tests/federated.rs` should be rewritten or supplemented to call this runner directly. The observable result is that source and sink run concurrently under RTI grants and the sink records `(Tag::ZERO, 7)` for hello and `(Tag::new(Duration::milliseconds(10), 0), 7)` for delayed connections.

Milestone 7 defines shutdown semantics for the static runner. A federate with no local work should send `NET(FOREVER)`. When a scheduler reaches a terminal shutdown tag, the client should send `LTC` for that tag and then `Stop` or a final no-future indication. The RTI should not grant tags after a federate is stopped, and it should allow the federation to terminate only after all persistent federates have stopped or reached no-future state and all in-transit messages are acknowledged. Add tests for a source that stops after startup, a sink that waits for a delayed message and then stops, and a no-message topology that does not deadlock.

Milestone 8 promotes TCP from smoke coverage to a reusable runtime path. Reuse the session and client logic from the in-memory runner with `JsonProtocolFrameTransport`. Keep the test ignored if localhost binding remains sandbox-sensitive, but make the code path real: one RTI listener accepts federates, each federate sends `Hello`, the RTI sends `Start`, the source sends a Boomerang-produced `MSG`, the sink schedules it through the inbound registry, `TAG` and `LTC` flow, and both sides shut down. Do not add Tokio to `boomerang_runtime`; keep Tokio scoped to `boomerang_federated`.

Milestone 9 broadens correctness coverage. Add tests for a three-federate chain, fanout from one source to two sinks, two messages at the same tag and endpoint, same timestamp but increasing microsteps, a positive-delay distributed cycle, and continued rejection of a zero-delay distributed cycle. Add an explicit test that a target federate cannot receive a `TAG` beyond an in-transit message until it sends `LTC` at or beyond that message tag. If modal reactors or physical actions are not supported across federation, add explicit rejection tests or document the exact supported subset in API docs.

## Concrete Steps

Work from the repository root:

    cd /Users/johhug01/Source/boomerang

Before implementing each milestone, check the current state:

    git status --short --branch
    cargo test -p boomerang_federated
    cargo test -p boomerang_builder --features federated
    cargo test -p boomerang_runtime --features federated

Expected baseline at the time this plan was written:

    cargo test -p boomerang_federated
    test result: ok. 13 passed; 0 failed; 1 ignored

    cargo test -p boomerang_builder --features federated
    test result: ok. 44 passed; 0 failed

    cargo test -p boomerang_runtime --features federated
    test result: ok. 19 passed; 0 failed

For Milestone 1, extract shared boundary metadata before adding any new RTI or protocol bridge code. Start by reading `boomerang_builder/src/env/build.rs`, `boomerang_builder/src/connection.rs`, and `boomerang_builder/src/federation.rs`. The implementation should make the builder's inter-partition boundary model explicit enough to derive both the current local `EnclaveDep` list and the current `FederationPlan` federated edge list from the same source facts. Do not change how local cross-enclave delivery or federated delivery executes in this milestone. Add tests or strengthen existing tests in `boomerang_builder/src/tests/federated.rs` and nearby builder tests so they prove local cross-enclave connections still do not require a federated codec, federated logical connections still record endpoint metadata, cross-federate physical connections are still rejected, and distributed zero-delay cycles are still rejected.

For Milestone 2, add bridge functions after the boundary metadata is in place. Prefer one of these shapes:

    In boomerang_runtime/src/federated.rs:

        pub fn runtime_tag_to_wire_parts(tag: Tag) -> Result<RuntimeWireTag, FederatedEndpointError>;
        pub fn runtime_tag_from_wire_parts(tag: RuntimeWireTag) -> Result<Tag, FederatedEndpointError>;

    or, if protocol types are required, put the conversion in boomerang_builder behind the federated feature:

        pub fn runtime_tag_to_wire_tag(tag: runtime::Tag) -> Result<boomerang_federated::WireTag, BuilderError>;
        pub fn wire_tag_to_runtime_tag(tag: boomerang_federated::WireTag) -> Result<runtime::Tag, BuilderError>;

Do not make `boomerang_runtime` depend on `boomerang_federated`. If protocol types appear in a signature, the function must live outside `boomerang_runtime`.

For Milestone 3, add the live outbound sink without replacing existing tests. A minimal test should build a `FederatedOutboundChannel`, call `FederatedOutboundSink::send`, and prove a receiver obtains the exact command. If using `kanal`, it is already a runtime dependency and fits the existing scheduler stack.

For Milestone 4, add pure protocol session tests in `boomerang_federated`. The first test should create a two-federate topology, send `Hello` from both, send `NET(ZERO)` from the sink, send `NET(ZERO)` from the source, verify the RTI grants the source first, send a source `MSG(ZERO)`, verify the sink receives the routed message, send sink `LTC(ZERO)`, and then verify later grants are unblocked.

For Milestone 5 and Milestone 6, write the first end-to-end test before finalizing the API. The test should be in `boomerang_builder/src/tests/federated.rs` or a new integration test under `boomerang/tests/` if top-level APIs are used. It must build two child federates using `add_child_federate`, register `SerdeJsonCodec` for `u32`, connect source to sink with ordinary `connect_port`, and call the new live runner. The old helper functions `runtime_tag_to_wire_tag`, `wire_tag_to_runtime_tag`, and `route_outbound_commands_through_rti` in the test module should become unnecessary after the runner exists.

For Milestone 7, write shutdown tests before adding TCP. These tests should fail with a deadlock or timeout before the shutdown semantics are implemented and then pass deterministically.

For Milestone 8, keep the ignored TCP test command:

    cargo test -p boomerang_federated tcp_smoke -- --ignored

If the sandbox denies localhost binding with `Operation not permitted`, rerun the same command with approval. This should be the only milestone that needs sandbox escalation.

After each milestone, run the narrow tests first, then the broader affected checks:

    cargo test -p boomerang_runtime --features federated
    cargo test -p boomerang_federated
    cargo test -p boomerang_builder --features federated
    cargo check -p boomerang --features federated --benches

At the end of this plan, also run:

    cargo test

If full `cargo test` fails for an unrelated pre-existing reason, record the command, failing output, and why it is unrelated in `Surprises & Discoveries`.

## Validation and Acceptance

The plan is accepted when a human can run a live static federation test and see the same behavior as local enclave execution without manually routing commands in the test.

The hello acceptance test builds a root reactor with a source federate and a sink federate. The source emits `7u32` during startup. The sink records every received value with `ctx.get_tag()`. Running the in-memory federated runner should produce:

    recorded values: [(Tag::ZERO, 7)]

The delayed acceptance test connects the source output to the sink input with `after = Some(Duration::milliseconds(10))`. Running the in-memory federated runner should produce:

    recorded values: [(Tag::new(Duration::milliseconds(10), 0), 7)]

The live-time-coordination acceptance test must prove that the target federate is not granted a tag beyond an in-transit `MSG` until it reports `LTC` for the message tag. The expected assertion is not just that a message is delivered; it must inspect the order of protocol decisions or a recording RTI log:

    target NET(10ms) -> blocked by in-transit MSG(5ms)
    target LTC(5ms) -> target TAG(10ms)

The shutdown acceptance test must show that two persistent federates terminate without a timeout once all messages are processed and all federates send `Stop` or no-future state. A test that only passes because `Config::with_timeout` forces shutdown is not sufficient.

The zero-delay-cycle acceptance test remains a rejection test:

    building A -> B and B -> A with no positive after delay returns BuilderError::UnsupportedFederationTopology containing "distributed zero-delay cycle"

The TCP acceptance test can remain ignored if localhost binding is not always available in CI or the sandbox, but when run manually it must use the same RTI/federate session logic as the in-memory runner rather than a bespoke one-message smoke harness.

## Idempotence and Recovery

All milestones should be additive. Keep the existing `FederatedOutboundBuffer` and manual tests until the live runner tests pass, then either remove redundant helpers or leave them as narrow unit tests if they still prove useful behavior.

Do not change default local execution. `runtime::execute_enclaves` and `Scheduler::new` must continue to behave as local-only APIs. The federated runner should be a new opt-in API behind the `federated` feature.

If a milestone introduces a deadlock, first reduce it to an in-memory test with small timeouts and protocol logging. Do not debug it first through TCP. TCP adds scheduling, OS sockets, and async task ordering that can hide the logical-time bug.

If tag conversion overflows or encounters a negative finite offset, return a typed error instead of panicking. Existing tests currently use `unwrap` in helper conversion functions; the runtime bridge must be stricter.

If the RTI loop receives an unexpected frame, return or send `RtiToFederate::Error { message }` where possible and terminate that session cleanly. Avoid silent drops for control-plane messages.

The worktree may contain unrelated untracked files. Do not remove them as part of this plan. Before each implementation session, use `git status --short --branch` and avoid reverting changes that are not part of this task.

## Artifacts and Notes

Current baseline evidence from 2026-07-09:

    git log --oneline -n 6
    5bffc5a feat: add federated tcp transport smoke
    c0a45a2 refactor: infer federated builder connections
    a3eabea feat: lower federated endpoints
    5a25647 feat: add federated scheduler time barrier hook
    268489f feat: add federated protocol crate
    5d2a1ed feat: add federated builder topology groundwork

The key tests that currently pass are:

    cargo test -p boomerang_federated
    cargo test -p boomerang_builder --features federated
    cargo test -p boomerang_runtime --features federated

The current manual in-memory federated test flow is in `boomerang_builder/src/tests/federated.rs`. It should be treated as a specification for values and tags, not as the final runtime structure. The final live runner should make the manual function that drains `FederatedOutboundBuffer` and calls `RtiState::handle(FederateToRti::Msg { ... })` unnecessary for end-to-end tests.

The current ignored TCP smoke command is:

    cargo test -p boomerang_federated tcp_smoke -- --ignored

The managed sandbox may reject localhost bind. That is an environment limitation, not proof that the TCP code path is wrong. Keep non-network in-memory tests as the primary CI correctness proof.

## Interfaces and Dependencies

Do not add a dependency from `boomerang_runtime` to `boomerang_federated`. If a type mentions `WireTag`, `ProtocolFrame`, `FederateId`, `EndpointId`, or `RtiState`, it must live outside `boomerang_runtime`.

Keep Tokio dependencies scoped to `boomerang_federated`. The reaction scheduler remains synchronous and thread-based.

Use existing `kanal` channels where a synchronous bridge is needed inside runtime-adjacent code. Use `futures_util::Sink` and `futures_util::TryStream` for transport-agnostic protocol sessions, with `TransportError` as the common session-facing error. Use `futures-channel` for pure in-memory protocol endpoint pairs. Use `tokio-serde` for TCP protocol serialization and `tokio-util` for byte framing. Use `JsonProtocolFrameTransport` only after the in-memory session is correct.

The final public or crate-public shape should include equivalents of these interfaces. Exact names may change if the implementation finds a better local convention, but the behavior must remain.

In `boomerang_builder/src/inter_partition.rs`, define shared inter-partition metadata without protocol types:

    pub enum BoundaryKind {
        LocalEnclave,
        Federated {
            source_federate: String,
            target_federate: String,
        },
    }

    pub struct InterPartitionEdge {
        pub kind: BoundaryKind,
        pub source_partition: BuilderReactorKey,
        pub target_partition: BuilderReactorKey,
        pub source_port: BuilderPortKey,
        pub target_port: BuilderPortKey,
        pub delay: Option<runtime::Duration>,
        pub physical: bool,
    }

    pub struct InterPartitionPlan {
        pub partition_roots: Vec<PartitionRoot>,
        pub edges: Vec<InterPartitionEdge>,
    }

The exact type names may change, but Milestone 1 must leave a single builder-owned representation from which local `EnclaveDep` values and federated `FederationPlan` edges can be derived. This module must not mention `boomerang_federated::WireTag`, `ProtocolFrame`, `FederateId`, `EndpointId`, or `RtiState`.

In `boomerang_federated` behind the `runtime` feature, define checked runtime/protocol conversion impls:

    pub enum RuntimeBridgeError {
        ...
    }

    impl TryFrom<boomerang_runtime::Tag> for WireTag {
        type Error = RuntimeBridgeError;
        ...
    }

    impl TryFrom<WireTag> for boomerang_runtime::Tag {
        type Error = RuntimeBridgeError;
        ...
    }

    impl TryFrom<boomerang_runtime::Duration> for WireDelay {
        type Error = RuntimeBridgeError;
        ...
    }

In a builder-owned federated bridge module, define topology and route extraction:

    pub struct FederatedRoute {
        pub endpoint: runtime::FederatedEndpointId,
        pub source: boomerang_federated::FederateId,
        pub target: boomerang_federated::FederateId,
    }

    pub fn federation_topology_from_plan(
        plan: &FederationPlan,
    ) -> Result<boomerang_federated::FederatedTopology, BuilderError>;

    pub fn federated_routes_from_plan(
        plan: &FederationPlan,
    ) -> Result<Vec<FederatedRoute>, BuilderError>;

In `boomerang_runtime/src/federated.rs`, add a live outbound sink while preserving the existing buffer:

    pub struct FederatedOutboundChannel {
        ...
    }

    impl FederatedOutboundChannel {
        pub fn pair() -> (Self, FederatedOutboundReceiver);
    }

    pub struct FederatedOutboundReceiver {
        ...
    }

    impl FederatedOutboundReceiver {
        pub fn recv(&self) -> Result<FederatedOutboundCommand, FederatedEndpointError>;
        pub fn try_recv(&self) -> Result<Option<FederatedOutboundCommand>, FederatedEndpointError>;
    }

In `boomerang_federated/src/session.rs`, define transport-agnostic RTI session machinery:

    pub struct StaticRtiSession {
        ...
    }

    impl StaticRtiSession {
        pub async fn run(self) -> Result<(), SessionError>;
    }

    pub enum SessionError {
        Transport(String),
        Rti(String),
        Protocol(String),
        Shutdown(String),
    }

If the final implementation keeps the in-memory runner synchronous, `StaticRtiSession::run` can have a synchronous equivalent. The important point is that the logic is reusable between in-memory and TCP tests.

In the federate client bridge, implement `runtime::FederatedTimeBarrier`:

    pub struct RtiFederatedTimeBarrier {
        ...
    }

    impl runtime::FederatedTimeBarrier for RtiFederatedTimeBarrier {
        fn acquire_tag(
            &mut self,
            tag: runtime::Tag,
            event_rx: &runtime::Receiver<runtime::AsyncEvent>,
        ) -> Option<runtime::AsyncEvent>;

        fn logical_tag_complete(&mut self, tag: runtime::Tag);
    }

In a builder-facing runner module, expose an opt-in execution API:

    pub fn execute_federation_in_memory(
        parts: BuilderRuntimeParts,
        config: runtime::Config,
    ) -> Result<tinymap::TinySecondaryMap<runtime::EnclaveKey, runtime::Env>, BuilderError>;

This function must not replace `runtime::execute_enclaves`; it is the explicit federated execution path. It should fail clearly if `parts.federation_plan` is empty, if an enclave cannot be mapped to exactly one federate, if an endpoint route is missing, or if a cross-federate edge uses unsupported physical or zero-delay-cycle semantics.

## Non-Goals For This Plan

This plan does not implement transient federates, dynamic join, leave, or rejoin behavior.

This plan does not implement `PTAG` or `ABS`.

This plan does not support distributed zero-delay cycles; it preserves the current build-time rejection.

This plan does not support cross-federate physical connections.

This plan does not add authentication, reconnect behavior, fault tolerance, hot swap, direct federate-to-federate payload channels, or optimized coordination traffic.

This plan does not require multiple local enclaves inside one federate process. The API should avoid blocking that future support, but the first live runner may enforce one federate per runtime enclave.

Change note: This file was created on 2026-07-09 as a fresh, self-contained ExecPlan for the next phase: turning the existing federated scaffolding into a live static federated runtime. It intentionally leaves `FEDERATED_REACTORS_EXECPLAN.md` untouched.

Change note: Revised on 2026-07-09 to insert shared inter-partition topology and boundary-edge metadata as the new first milestone. This change captures the decision to unify local enclave and federated reactor planning at the builder metadata layer before adding checked protocol bridge utilities, while keeping local and federated delivery implementations separate.

Change note: Revised on 2026-07-09 to replace the Milestone 4 custom federated transport traits with the ecosystem-standard `futures_util::Sink` and `futures_util::Stream` contract. This keeps the pure in-memory RTI session milestone aligned with the later TCP/Tokio goal while reducing crate-specific async abstraction.

Change note: Revised on 2026-07-09 to replace manual `TcpTransport` JSON encode/decode logic with `tokio-serde`'s JSON transport combinator over the existing `tokio-util` length-delimited frames. This further reduces custom TCP transport code while preserving the Milestone 4 scope.

Change note: Revised on 2026-07-09 to remove the `TcpTransport` wrapper entirely. The TCP protocol path is now the direct `JsonProtocolFrameTransport` alias plus `json_protocol_frame_transport(TcpStream)`, using the default `LengthDelimitedCodec` frame length.

Change note: Revised on 2026-07-09 to replace the Tokio-backed custom in-memory transport structs with direct `futures-channel` MPSC endpoint types. The RTI session now accepts `Sink`/`TryStream` endpoints whose native errors convert into `TransportError`.
