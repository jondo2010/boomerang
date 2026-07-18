# Compile Federated Protocol Identities into Dense Runtime Keys

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `.agent/PLANS.md`. It is a focused federated-runtime improvement under Stage 6 of `.agent/PROJECT_ROADMAP_EXECPLAN.md`; it does not replace the broader stable logical identity work for graphs, reactors, ports, actions, connections, partitions, schemas, or recordings.

## Purpose / Big Picture

Boomerang uses `FederateId` and `EndpointId` as stable string identities in topology manifests and serialized RTI protocol frames. Those values must remain readable, durable, and independent of allocation order. After a topology has been validated, however, the RTI repeatedly uses those strings as keys for coordination state, dependency indexes, transitive work sets, and route validation. This duplicates string-keyed maps and performs ordered string lookup in the hottest control-plane paths.

After this work, stable IDs remain unchanged at every public, manifest, diagnostic, and wire boundary, while `CompiledTopology` translates them into process-local `FederateKey` and `EndpointKey` values. Immutable compiled records use `TinyMap`; RTI coordination state uses `TinySecondaryMap` keyed by the topology-owned `FederateKey`. Protocol behavior, serialized frames, grant order, failure atomicity, and public stable identities remain unchanged. A smaller related cleanup removes the linear scan used to find an outbound endpoint during lowering by supplying the already-known target Federate.

The change is observable through tests that prove identical JSON protocol frames, deterministic stable-ID-to-dense-key resolution, unchanged RTI delivery traces, failure-atomic rejection of invalid identities and routes, and direct target-scoped outbound route lookup.

## Progress

- [x] (2026-07-18) Investigated every production `FederateId` and `EndpointId` collection and separated protocol/deployment boundaries from compiled RTI state.
- [x] (2026-07-18) Chose a two-level identity model: stable IDs at boundaries and topology-owned dense keys internally.
- [x] (2026-07-18) Milestone 1: characterized stable wire identity and current deterministic RTI behavior; both focused tests and the feature-minimal check pass.
- [x] (2026-07-18) Milestone 2: compiled topology members and endpoints into lexical dense records; focused tests and both feature checks pass.
- [x] (2026-07-18) Milestone 3: migrated RTI coordination, validated events, grant work sets, overrides, and transitions to dense keys; all focused gates pass.
- [x] (2026-07-18) Milestone 4: cached each authenticated session participant's dense key once after endpoint validation; session and runner gates pass.
- [x] (2026-07-18) Milestone 5: replaced the outbound route scan with target-Federate and endpoint lookup; bridge and builder gates pass.
- [x] (2026-07-18) Milestone 6: documented the stable/dense identity boundary, completed the focused validation matrix, and recorded the delayed local scheduler-shutdown results separately.

## Surprises & Discoveries

- Observation: `FederateId` and `EndpointId` cross Federate/RTI protocol and process boundaries, not ordinary same-Federate scheduler boundaries.
  Evidence: `boomerang_federated/src/protocol.rs` includes them in `FederateToRti`, `RtiToFederate`, `FederatedTopology`, and `TopologyEdge`; local cross-Enclave delivery instead uses `EnclaveKey`, runtime actions, and in-process channels.

- Observation: the RTI duplicates the same Federate domain across many ordered maps.
  Evidence: `boomerang_federated/src/rti/mod.rs::CompiledTopology` stores five `BTreeMap` indexes keyed by `FederateId` or pairs of `FederateId`, and `RtiState` owns another `BTreeMap<FederateId, FederateCoordination>`.

- Observation: endpoint identity is already globally unique within one compiled topology.
  Evidence: `CompiledTopology::new` rejects duplicate or conflicting uses of one `EndpointId` before it builds the current three-field `RouteKey`. Consequently one compiled endpoint record can own source, target, and delay, and route validation needs only one endpoint lookup followed by source/target comparison.

- Observation: replacing every endpoint map with `TinyMap` would add indirection without removing the required stable lookup.
  Evidence: inbound `RtiToFederate::Msg` frames carry `EndpointId`; `RtiLogicalTimeCoordinator::route_for` must translate that wire value before reaching a route handler. The per-Federate stable route map is therefore an appropriate boundary index.

- Observation: `FederatedRuntimeConnections::outbound_endpoint` scans every target-owned route table even though builder lowering already knows the target Federate.
  Evidence: `boomerang_federated/src/runtime_bridge.rs::outbound_endpoint` uses `values().find_map`, while `boomerang_builder/src/federated/lowering.rs::FederatedBoundary` already retains `target_federate`.

- Observation: RTI code is compiled without the crate's `runtime` feature, but `tinymap` is currently optional under that feature.
  Evidence: `boomerang_federated/Cargo.toml` places `dep:tinymap` in `runtime`, while `rti` and `session` are unconditional modules.

- Observation: CodeStory's local graph is current, but broad packet/search evidence was blocked at implementation start by a stale sidecar manifest caused by the unrelated modified `boomerang/benches/physical_actions.rs` input.
  Evidence: project-scoped status allowed local grounding and reported 145 indexed files with no graph errors; balanced grounding resolved `CompiledTopology`, `RtiState::handle_from`, and `FederatedRuntimeConnections::outbound_endpoint` before source inspection.

- Observation: manifest insertion order does not affect the current stable neighbor views, transitive downstream work set, minimum delay, or route acceptance.
  Evidence: `compiled_topology_indexes_dependencies_and_routes_deterministically` passes with a second topology whose member and edge insertion orders are reversed.

- Observation: `TinyMap` deliberately owns dense allocation but does not implement `Clone`, `PartialEq`, or `Eq`, and its opaque public iterators do not advertise `ExactSizeIterator`.
  Evidence: the first Milestone 2 compile failed on derived `CompiledTopology` traits and the requested exact-size iterator. `CompiledTopology` now implements structural clone/equality locally and exposes a zero-allocation range-backed exact-size iterator without changing `boomerang_tinymap`.

- Observation: the existing RTI failure-atomic suite is strong enough to validate the dense transition boundary without changing expected deliveries or errors.
  Evidence: after `ResolvedRtiEvent` and `TinySecondaryMap` migration, all 42 `rti::tests` pass unchanged except representation-only helpers and assertions; authenticated mismatch, unknown member, invalid route, lifecycle, overflow, and ordering cases retain their stable results.

- Observation: session spoofing protection was already observable before dense transport identity caching.
  Evidence: `session_cached_identity_rejects_mismatched_claim` passed before the session representation changed and still returns the exact `NET identified federate ... authenticated endpoint ...` protocol error afterward.

- Observation: target-owned route storage makes wrong-target lookup failure directly testable even when the endpoint exists for another Federate.
  Evidence: `outbound_endpoint_uses_declared_target_route` emits the unchanged source, target, endpoint, tag, and payload through the declared target, then returns `UnknownRoute` for the same endpoint through a different target without falling back to any other route table.

- Observation: the guarded delayed local cross-Enclave test passed on this run, but the broader boundary-equivalence test exposed the same intermittent local delayed scheduler-shutdown behavior during the workspace suite.
  Evidence: `timeout 30 cargo test -p boomerang_builder --all-features test_in_memory_distributed_delayed_connection_matches_local_tag -- --nocapture` passed 1/1. The prescribed workspace command timed out twice at `local delayed boundary equivalence`, while that exact equivalence test passed 1/1 in isolation in 0.02 seconds. A workspace run excluding both delayed local cases then passed with no other failures. The failing local path predates and does not exercise the federated dense-key changes, so scheduler shutdown remains outside this plan.

- Observation: CodeStory refreshed its project-scoped index after the implementation files changed and grounded the final tree without graph errors.
  Evidence: final strict grounding represented all 146 indexed files and 7,886 symbols, with 0 index errors and 0 fatal errors.

## Decision Log

- Decision: Keep `FederateId` and `EndpointId` as the only serialized and deployment-facing identities.
  Rationale: dense keys are allocation-local indexes and cannot be durable protocol or recording identities.
  Date/Author: 2026-07-18 / John Hughes and Codex

- Decision: Define crate-private `FederateKey` and `EndpointKey` in the RTI compilation layer.
  Rationale: these keys describe one `CompiledTopology` instance and must not become public alternatives to stable IDs.
  Date/Author: 2026-07-18 / Codex

- Decision: Allocate dense keys in lexical stable-ID order, independent of manifest insertion order.
  Rationale: existing BTree-based RTI work sets and delivery batches are deterministic in stable-ID order. Lexical allocation preserves that behavior while making dense-key order deterministic.
  Date/Author: 2026-07-18 / Codex

- Decision: Let `CompiledTopology` own dense identity records and let `RtiState` use a secondary map.
  Rationale: the topology defines which Federates exist; coordination state is mutable data attached to those identities and is therefore correctly represented by `TinySecondaryMap<FederateKey, FederateCoordination>` rather than another owner map.
  Date/Author: 2026-07-18 / Codex

- Decision: Retain stable-ID maps in builder lowering, `RuntimeFederation`, transports, public session constructors, errors, and tracing.
  Rationale: these are construction, deployment, or protocol boundaries where names are meaningful and lookup volume is low. Mechanically replacing them would obscure ownership and still require reverse translation.
  Date/Author: 2026-07-18 / Codex

- Decision: Retain the stable `EndpointId` route map in each `FederateRuntimeBridge` and `RtiLogicalTimeCoordinator`.
  Rationale: incoming wire messages arrive with an `EndpointId`; one direct stable lookup is simpler than a stable-to-dense lookup followed by a TinyMap lookup when no later processing benefits from `EndpointKey`.
  Date/Author: 2026-07-18 / Codex

- Decision: Make `tinymap` an unconditional local dependency of `boomerang_federated`.
  Rationale: the unconditional RTI module will own dense keys. This adds no external package and does not introduce a dependency on `boomerang_runtime`.
  Date/Author: 2026-07-18 / Codex

- Decision: Treat Milestone 1 as characterization rather than a production TDD cycle.
  Rationale: the new tests intentionally pass against the pre-refactor representation and establish the invariant that later red/green representation changes must preserve.
  Date/Author: 2026-07-18 / Codex

- Decision: Implement `CompiledTopology` clone/equality and exact-size iteration locally instead of expanding `TinyMap`'s workspace-wide API.
  Rationale: only the compiled topology currently needs these traits, and keeping the adaptation local preserves the focused milestone file set and avoids a broader container-library change.
  Date/Author: 2026-07-18 / Codex

- Decision: Make validation consume the incoming protocol message and return an owned `ResolvedRtiEvent`.
  Rationale: all stable identities are resolved and all errors are produced before mutation, while the payload can move directly through the validated event without cloning. `handle_validated` can therefore operate exclusively on dense keys.
  Date/Author: 2026-07-18 / Codex

- Decision: Carry one private `SessionParticipant { id, key }` through initial frames, reader frames, closure, and transport errors.
  Rationale: every transport event retains the stable identity needed for errors and sink addressing while the RTI receives the already-resolved immutable key; no public constructor or session error shape changes.
  Date/Author: 2026-07-18 / Codex

- Decision: Clone the lowered target Federate once so inbound binding and deferred outbound construction consume the same stable identity.
  Rationale: the target is already established by `FederatedBoundary`; threading that value prevents a later global scan and keeps route ownership consistent without adding a duplicate route index.
  Date/Author: 2026-07-18 / Codex

- Decision: Record, but do not patch, the additional delayed local boundary-equivalence timeout found by the final workspace run.
  Rationale: the failure is isolated to the unchanged local cross-Enclave scheduler path, passes when run alone, and is another manifestation of the pre-existing intermittent shutdown race that this plan explicitly excludes.
  Date/Author: 2026-07-18 / Codex

## Outcomes & Retrospective

All six milestones are implemented. Stable `FederateId` and `EndpointId` strings remain the public, manifest, diagnostic, and serialized identities. `CompiledTopology` now owns lexically allocated crate-private dense Federate and endpoint records, `RtiState` attaches coordination with `TinySecondaryMap`, sessions cache the authenticated dense Federate key, and outbound sender construction performs direct target-scoped route lookup. The protocol, RTI, session, bridge, and builder regression suites preserve stable errors, delivery order, wire frames, and failure atomicity.

Final verification passed formatting, all-target workspace checking, all 85 non-ignored `boomerang_federated` tests, all 58 selected builder tests, mdBook generation, identity-boundary audit, and diff hygiene. The guarded known delayed local test passed 1/1. The prescribed workspace test command encountered the pre-existing delayed local scheduler-shutdown race twice in the broader boundary-equivalence test; that test passed in isolation, and the workspace suite passed when both delayed local cases were excluded. No scheduler change was made because shutdown work is outside this plan.

## Context and Orientation

`boomerang_federated/src/protocol.rs` defines stable wire data. `FederateId(String)` identifies one protocol participant. `EndpointId(String)` identifies one globally unique serialized logical route. `FederatedTopology` is the stable manifest containing a `Vec<FederateId>` and `Vec<TopologyEdge>`. `FederateToRti` and `RtiToFederate` serialize these identities. None of those types should contain `FederateKey` or `EndpointKey` after this work.

`boomerang_federated/src/rti/mod.rs` validates `FederatedTopology` into `CompiledTopology`. Today the compiled value retains the original topology plus string-keyed incoming, downstream, transitive, minimum-delay, route, and neighbor indexes. `RtiState` then creates another string-keyed map for mutable `FederateCoordination`. NET, LTC, MSG, and Stop transitions repeatedly resolve and clone stable IDs while evaluating grants.

`boomerang_federated/src/rti/tests.rs` is the principal semantic regression suite. It protects topology validation, competing paths, positive-delay cycles, exact grant decisions, deterministic sender-first work sets, authenticated identity checks, lifecycle transitions, in-transit messages, overflow handling, and failure atomicity. Dense-key work must preserve all these tests rather than rewrite expected protocol behavior.

`boomerang_federated/src/session.rs` owns the transport boundary. Public constructors accept `BTreeMap<FederateId, RtiSessionEndpoint<...>>`; errors and sinks are addressed by stable IDs. Each stream reader repeatedly sends its authenticated stable identity with frames. This plan caches the corresponding `FederateKey` once after endpoint validation but keeps public constructors, errors, and sink addressing stable.

`boomerang_federated/src/runtime_bridge.rs` owns prepared per-Federate mailboxes and stable endpoint routes. Routes are stored on their target Federate because the target owns the inbound decode-and-schedule handler. `outbound_endpoint` currently scans every target route table to recover the route, then selects the source mailbox. Builder lowering already knows the target Federate and can request that route directly.

`boomerang_builder/src/federated/lowering.rs` derives stable endpoint and Federate identities from assembly partitions and port fully-qualified names. `boomerang_builder/src/connection.rs` consumes those artifacts while constructing serialized senders and target handlers. These remain stable-ID construction paths; only the target Federate is threaded into the deferred outbound lookup.

`boomerang_tinymap` provides `tinymap::key_type!`, `TinyMap`, and `TinySecondaryMap`. A `TinyMap<K, V>` creates and owns contiguous keys as values are inserted. A `TinySecondaryMap<K, V>` attaches optional values to keys owned elsewhere. This ownership distinction is mandatory in the new representation.

The working tree may contain unrelated unstaged documentation and untracked `.agent`, `docs/execplans`, coverage, or log files. Implementation must stage only files named by the current milestone and must not discard unrelated changes.

## Plan of Work

### Milestone 1: Protect stable wire identity and deterministic behavior

Begin with characterization, not representation changes. In `boomerang_federated/src/protocol.rs`, add a serde-json test named `stable_protocol_identities_round_trip_without_dense_keys`. Construct a `FederateToRti::Msg` using nontrivial IDs, serialize it, deserialize it, and assert exact equality. Also assert that the JSON contains the stable strings and does not contain `FederateKey` or `EndpointKey`. This proves that the later private keys cannot leak into frames.

In `boomerang_federated/src/rti/tests.rs`, extend `compiled_topology_indexes_dependencies_and_routes_deterministically` with a second topology containing the same stable members and edges in a different insertion order. Assert that public neighbor views, minimum delays, route acceptance, and the ordered affected Federate results remain identical. Do not assert a dense type yet; this test must pass before production changes and protect the ordering that lexical dense-key allocation must preserve.

Run from the repository root:

    cargo test -p boomerang_federated --features serde-json-codec stable_protocol_identities_round_trip_without_dense_keys
    cargo test -p boomerang_federated compiled_topology_indexes_dependencies_and_routes_deterministically
    cargo check -p boomerang_federated --no-default-features

Expect both tests and the feature-minimal check to pass. Commit only the characterization changes:

    git add boomerang_federated/src/protocol.rs boomerang_federated/src/rti/tests.rs
    git commit -m "test(federated): characterize stable protocol identities"

### Milestone 2: Compile stable identities into dense topology records

Make `tinymap` unconditional in `boomerang_federated/Cargo.toml`: remove `dep:tinymap` from the `runtime` feature and change the dependency from optional to `tinymap.workspace = true`. Keep `boomerang_runtime` and `tracing` optional.

Create `boomerang_federated/src/rti/index.rs`. Define crate-private keys with one-line rustdoc:

    tinymap::key_type! {
        /// Dense identity of one Federate within a compiled topology.
        pub(crate) FederateKey
    }

    tinymap::key_type! {
        /// Dense identity of one endpoint within a compiled topology.
        pub(crate) EndpointKey
    }

Define `CompiledFederate`, `CompiledEndpoint`, `IncomingDependency`, and `IncomingPath` there. Every struct and field must have rustdoc. `CompiledFederate` owns its stable `FederateId`, direct incoming dependencies, direct downstream keys, transitive incoming paths, transitive downstream keys, and cached stable `NeighborStructure`. `CompiledEndpoint` owns its stable `EndpointId`, source and target `FederateKey`, and `WireDelay`. Dependencies and paths refer to dense keys; they do not clone stable IDs.

Refactor `CompiledTopology` in `boomerang_federated/src/rti/mod.rs` to own:

    original: FederatedTopology,
    federates: tinymap::TinyMap<FederateKey, CompiledFederate>,
    federate_keys: BTreeMap<FederateId, FederateKey>,
    endpoints: tinymap::TinyMap<EndpointKey, CompiledEndpoint>,
    endpoint_keys: BTreeMap<EndpointId, EndpointKey>,
    minimum_delays: BTreeMap<(FederateKey, FederateKey), WireDelay>,

The sparse all-pairs delay table may remain a `BTreeMap`, but its keys must be cheap dense keys rather than strings. Remove `RouteKey` and the parallel incoming, downstream, transitive, route-set, and neighbor maps. Allocate `FederateKey` values by sorting unique `FederateId` values lexically, and allocate `EndpointKey` values by sorting validated edges by `EndpointId`. Preserve `original` byte-for-byte and in original order for public manifest access.

Provide crate-private resolution methods and retain stable public accessors:

    pub(crate) fn federate_key(&self, id: &FederateId) -> Option<FederateKey>;
    pub(crate) fn federate_id(&self, key: FederateKey) -> &FederateId;
    pub(crate) fn federates(
        &self,
    ) -> impl ExactSizeIterator<Item = (FederateKey, &CompiledFederate)>;
    pub(crate) fn endpoint_key(&self, id: &EndpointId) -> Option<EndpointKey>;
    pub(crate) fn endpoint(&self, key: EndpointKey) -> &CompiledEndpoint;
    pub fn neighbors_for(&self, id: &FederateId) -> Option<&NeighborStructure>;

Change route validation to resolve one `EndpointId`, then compare the compiled endpoint's source and target keys. Preserve `RtiError` payloads in stable IDs. Add private RTI tests proving lexical keys are identical across reordered manifests, every key round-trips to its stable ID, endpoints round-trip, and an `EndpointId` cannot validate with the wrong source or target.

Run:

    cargo test -p boomerang_federated compiled_topology
    cargo test -p boomerang_federated state_handler_rejects_route_absent_from_topology_without_mutation
    cargo check -p boomerang_federated --no-default-features
    cargo check -p boomerang_federated --all-features

Expect all selected tests and both feature configurations to pass. Commit:

    git add boomerang_federated/Cargo.toml boomerang_federated/src/rti/index.rs \
      boomerang_federated/src/rti/mod.rs boomerang_federated/src/rti/tests.rs
    git commit -m "refactor(federated): compile dense topology identities"

### Milestone 3: Store RTI coordination by dense Federate key

First add a private representation test in `boomerang_federated/src/rti/tests.rs` named `rti_coordination_is_indexed_by_compiled_federate_key`. It should resolve `a` and `b` through `CompiledTopology`, assert distinct dense keys, and retrieve both coordination records through those keys. This test should fail to compile while `RtiState` remains string-keyed.

Replace `RtiState::federates` with:

    federates: tinymap::TinySecondaryMap<FederateKey, FederateCoordination>,

Populate it from `CompiledTopology::federates()` so the topology remains the sole key owner. Introduce a private `ResolvedRtiEvent` whose variants contain `FederateKey` and `EndpointKey` instead of stable IDs. Change validation to resolve and validate all stable IDs before mutation and return this resolved event. `handle_validated` must accept only `ResolvedRtiEvent`; it must not perform another stable-ID map lookup.

Convert `IncomingDependency`, `IncomingPath`, affected work sets, grant evaluation, override state, in-transit recording, and commit transitions to dense keys. Translate keys back through `CompiledTopology` only when creating `RtiDelivery`, `RtiToFederate::Msg`, tracing fields, or `RtiError`. Preserve exact sender-first then lexical-downstream delivery order by relying on the lexical `FederateKey` allocation from Milestone 2.

Keep the public boundary:

    pub fn handle_from(
        &mut self,
        authenticated_federate: &FederateId,
        message: FederateToRti,
    ) -> Result<Vec<RtiDelivery>, RtiError>;

Add a crate-private variant for callers that already resolved the authenticated participant:

    pub(crate) fn handle_from_key(
        &mut self,
        authenticated_federate: FederateKey,
        message: FederateToRti,
    ) -> Result<Vec<RtiDelivery>, RtiError>;

The stable method resolves once and delegates. Both methods must retain identity-mismatch, unknown-member, invalid-route, lifecycle, tag, overflow, and failure-atomic behavior.

Run:

    cargo test -p boomerang_federated rti::tests
    cargo test -p boomerang_federated --all-features static_runner
    cargo check -p boomerang_federated --no-default-features

Expect the complete RTI suite to pass without changed delivery expectations. Commit:

    git add boomerang_federated/src/rti/index.rs boomerang_federated/src/rti/mod.rs \
      boomerang_federated/src/rti/tests.rs
    git commit -m "refactor(federated): index RTI state by dense keys"

### Milestone 4: Resolve authenticated session participants once

In `boomerang_federated/src/session.rs`, retain the public `BTreeMap<FederateId, RtiSessionEndpoint<...>>` constructor and stable-ID sink maps. Add `FederateKey` to the private `SessionInput` variants alongside `FederateId`, or define a documented private `SessionParticipant { id, key }` used by all variants. Resolve keys after `validate_endpoint_set` and before spawning stream readers. A stream reader must carry its immutable participant value rather than requiring `RtiState` to resolve its authenticated ID for every frame.

Call `RtiState::handle_from_key` in the protocol loop. Claimed IDs inside frames must still be resolved and compared against the authenticated key by `RtiState`; caching the transport identity must not weaken spoofing protection. Keep `SessionError`, protocol errors, sink selection, Hello validation, and `RtiDelivery` addressed by stable IDs.

Add a session test named `session_cached_identity_rejects_mismatched_claim` that connects as one stable Federate and sends a NET or MSG claiming another. Assert the same protocol error frame and terminal outcome as the existing identity-mismatch contract. Retain all existing session order and positive-cycle tests.

Run:

    cargo test -p boomerang_federated session::tests
    cargo test -p boomerang_federated --all-features static_runner

Expect all session and runner tests to pass. Commit:

    git add boomerang_federated/src/session.rs boomerang_federated/src/rti/mod.rs
    git commit -m "refactor(federated): cache session federate keys"

### Milestone 5: Remove the outbound endpoint scan

Add a runtime-bridge test named `outbound_endpoint_uses_declared_target_route`. Construct at least three Federates and routes, request an outbound endpoint using its target Federate and stable endpoint, and assert that the resulting sink emits the unchanged source, target, endpoint, tag, and payload. Also request the endpoint through the wrong target and assert `FederateClientError::UnknownRoute` without searching another Federate's table.

Change `FederatedRuntimeConnections::outbound_endpoint` to accept the target Federate:

    pub fn outbound_endpoint(
        &self,
        target_federate: &FederateId,
        endpoint: &EndpointId,
    ) -> Result<(Box<dyn FederatedOutboundSink>, FederatedFaultState), FederateClientError>;

Perform one `self.federates.get(target_federate)` lookup and one target route lookup. Remove `values().find_map`. Continue selecting the source mailbox from the validated route's stable source ID.

In `boomerang_builder/src/connection.rs`, extend `InterPartitionSourceBackend::Serialized` with `target_federate`. Clone `FederatedBoundary::target_federate` once so inbound binding and deferred outbound construction receive the same identity. Pass it to `outbound_endpoint`. Update direct tests in `boomerang_federated/src/client/tests.rs` and `boomerang_federated/src/runtime_bridge.rs`.

Run:

    cargo test -p boomerang_federated --all-features outbound_endpoint
    cargo check -p boomerang_builder --all-features --tests
    cargo test -p boomerang_builder --all-features test_federated_sender_emits_serialized_msg_command

Expect direct lookup and builder sender tests to pass with unchanged wire messages. Commit:

    git add boomerang_federated/src/runtime_bridge.rs boomerang_federated/src/client/tests.rs \
      boomerang_builder/src/connection.rs
    git commit -m "refactor(federated): resolve outbound routes directly"

### Milestone 6: Document the identity boundary and verify the workspace

Update `docs/federated-protocol.md` to distinguish stable serialized `FederateId` and `EndpointId` values from crate-private compiled keys. Update `docs/federated-runtime.md` to show that stable IDs are resolved once into dense RTI topology/state indexes and translated back only at protocol, deployment, tracing, and error boundaries. Update `boomerang_federated/README.md` with one concise sentence; do not expose `FederateKey` or `EndpointKey` as public API.

Run the stale-terminology and boundary audit:

    rg -n "FederateKey|EndpointKey" boomerang_federated/src docs boomerang_federated/README.md

Expect production occurrences only in crate-private RTI/session implementation and maintained explanatory documentation. There must be no occurrence in `ProtocolFrame`, `FederateToRti`, `RtiToFederate`, `FederatedTopology`, `TopologyEdge`, `RuntimeFederation`, or builder public APIs.

Run final verification from the repository root:

    cargo fmt --all -- --check
    cargo check --workspace --all-features --all-targets
    cargo test -p boomerang_federated --all-features
    cargo test -p boomerang_builder --all-features -- --skip test_in_memory_distributed_delayed_connection_matches_local_tag
    cargo test --workspace --all-features -- --skip test_in_memory_distributed_delayed_connection_matches_local_tag
    mdbook build book -d /tmp/boomerang-mdbook-dense-federated-identities
    git diff --check

The skipped builder case has a documented pre-existing intermittent local cross-Enclave shutdown hang on both the pre-refactor and current trees. Run it once separately under a finite guard and record the actual result rather than claiming it is fixed by this work:

    timeout 30 cargo test -p boomerang_builder --all-features \
      test_in_memory_distributed_delayed_connection_matches_local_tag -- --nocapture

Expect every non-flaky test, formatting, compile, mdBook, and diff check to pass. If the guarded test times out, record that known result in `Surprises & Discoveries` and do not alter scheduler shutdown in this plan.

Commit documentation only after verification:

    git add docs/federated-protocol.md docs/federated-runtime.md boomerang_federated/README.md \
      .agent/FEDERATED_DENSE_RUNTIME_KEYS_EXECPLAN.md
    git commit -m "docs: explain dense federated runtime identities"

## Concrete Steps

Work from `/Users/johhug01/Source/boomerang` on the current feature branch. Before each milestone, run `git status --short` and inspect `git diff` so user-owned changes are not folded into milestone commits. Add the named test first, observe its expected baseline or compile failure, make only the milestone's implementation changes, run the focused commands, update this plan's `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective`, then commit the exact named files.

Do not use `git reset --hard`, `git checkout --`, or broad `git add .`. If a focused verification exposes an unrelated failure, preserve the evidence and diagnose it separately; do not broaden this identity/indexing plan into scheduler, wire-versioning, replay, deployment, or dynamic-membership work.

## Validation and Acceptance

Acceptance requires all of the following observable results:

Stable JSON and in-memory protocol frames still contain exactly `FederateId` and `EndpointId` strings. Reordering a valid manifest does not change dense key assignment, neighbor ordering, grant delivery order, or externally visible protocol traces. Duplicate Federates, missing members, missing endpoints, duplicate/conflicting routes, spoofed identities, invalid lifecycle transitions, regressing tags, and delay overflow return the same stable-ID errors without partial mutation.

`CompiledTopology` has one dense owner map for Federates and one for endpoints, plus stable-to-dense boundary indexes. It no longer owns parallel string-keyed incoming, downstream, transitive, route-set, and neighbor maps. `RtiState` stores coordination in `TinySecondaryMap<FederateKey, FederateCoordination>` and its grant/effect algorithms operate on dense keys. No dense key is serialized or exposed by the public facade.

The static session resolves each authenticated transport participant once and retains all existing spoofing checks. Runtime bridge outbound lookup uses the known target Federate and no longer scans all Federates. Existing in-memory and TCP protocol behavior, RTI grants, endpoint payloads, and federate-local Enclave ownership remain unchanged.

The prescribed feature-minimal and all-feature checks, focused tests, non-flaky workspace suite, formatting, mdBook, and diff hygiene pass. The known delayed local test is reported separately under its finite guard.

## Idempotence and Recovery

Topology compilation is deterministic and contains no external side effects. Re-running tests or compilation is safe. Each milestone is independently committed so a failed later milestone can be investigated without discarding earlier verified work.

If dense compilation changes delivery order, stop and compare key allocation against the existing lexical `BTreeMap`/`BTreeSet` order; do not update expected deliveries merely to accept a new order. If stable IDs appear in different serialized form, revert the protocol-facing change and keep translation behind `CompiledTopology`. If `--no-default-features` fails because `tinymap` remains optional, correct `boomerang_federated/Cargo.toml` rather than feature-gating the RTI keys.

## Artifacts and Notes

The intended ownership relationship is:

    FederatedTopology / ProtocolFrame
        stable FederateId and EndpointId
                    |
                    | validate and resolve once
                    v
    CompiledTopology
        TinyMap<FederateKey, CompiledFederate>
        TinyMap<EndpointKey, CompiledEndpoint>
        BTreeMap<FederateId, FederateKey>
        BTreeMap<EndpointId, EndpointKey>
                    |
                    | attach mutable state
                    v
    RtiState
        TinySecondaryMap<FederateKey, FederateCoordination>

Stable IDs are translated back for frames, errors, tracing, public topology views, deployment maps, and transport sink selection. Dense keys never leave `boomerang_federated` internals.

## Interfaces and Dependencies

`boomerang_federated` gains no new external dependency. Its existing local `tinymap` dependency becomes unconditional so unconditional RTI code can use it. `boomerang_runtime` remains optional and enabled only by the existing `runtime` feature. `boomerang_runtime` must not learn about Federates, endpoints, RTI state, protocol messages, Tokio, or transports.

The final internal interfaces are `FederateKey`, `EndpointKey`, `CompiledFederate`, `CompiledEndpoint`, stable-to-dense accessors on `CompiledTopology`, `RtiState::handle_from_key`, and target-scoped `FederatedRuntimeConnections::outbound_endpoint`. Public wire and manifest interfaces remain `FederateId`, `EndpointId`, `TopologyEdge`, `FederatedTopology`, `FederateToRti`, `RtiToFederate`, and `ProtocolFrame`.

Revision note (2026-07-18): Initial plan created from the approved investigation. It deliberately limits dense keys to compiled RTI/session state, retains stable maps at protocol and deployment boundaries, and includes the outbound-route scan removal because it uses already-lowered target ownership without adding a duplicate global route index.

Revision note (2026-07-18): Milestone 1 added stable-wire and reordered-manifest characterization evidence, recorded CodeStory readiness and the characterization-specific TDD decision, and left production representation unchanged for Milestone 2.

Revision note (2026-07-18): Milestone 2 introduced crate-private lexical dense keys and compiled records, replaced parallel string-keyed topology indexes, recorded the local TinyMap trait adaptation, and retained stable RTI coordination for the next milestone.

Revision note (2026-07-18): Milestone 3 moved mutable coordination and the complete validated-event/grant path to dense keys, preserved stable boundary errors and deliveries, and added the crate-private pre-resolved handler required by session caching.

Revision note (2026-07-18): Milestone 4 added one private stable/dense session participant value, resolved keys after endpoint-set validation, routed frames through `handle_from_key`, and proved spoofing behavior unchanged.

Revision note (2026-07-18): Milestone 5 threaded the lowered target Federate into deferred serialized senders, replaced the global outbound route scan with two direct lookups, and preserved exact wire output.

Revision note (2026-07-18): Milestone 6 documented the stable/dense boundary, completed the final audit and verification matrix, recorded the guarded delayed-local pass plus the separate intermittent boundary-equivalence timeout, and left scheduler shutdown unchanged as required.
