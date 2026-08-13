# Federated RTI Phase Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make assembly lowering the only producer of an immutable RTI graph, give each runtime Federate only its local data, and separate that graph from dense mutable RTI coordination state.

**Architecture:** Builder-owned federation graph analysis derives final RTI graph parts and per-Federate bridges from `PartitionAnalysis`. `boomerang_federated` mechanically interns those final parts into `RtiGraph`; sessions own the graph and a separate `RtiRuntimeState`, while clients consume only `RuntimeFederate` values. One Federate coordination service projects RTI grants across every active Enclave scheduler, and runner supervision stops blocked peers before joining after failure. Raw topology manifests, topology-bearing `Hello`, and compatibility constructors are removed end to end.

**Tech Stack:** Rust, `petgraph`, `slotmap`, `tinymap`, Tokio, Cargo tests, Clippy, rustfmt.

---

## File Structure

- Create `boomerang_builder/src/federated/graph.rs`: builder-only weighted federation graph analysis, deterministic reachability, minimum-delay paths, and zero-delay cycle validation.
- Modify `boomerang_builder/src/federated/lowering.rs`: project `PartitionAnalysis` boundaries into analyzed graph inputs, final `RtiGraph`, per-Federate routes, and connection-boundary metadata.
- Modify `boomerang_builder/src/federated/mod.rs`: expose the builder-internal graph analysis module.
- Modify `boomerang_builder/src/inter_partition.rs`: keep partition ownership and boundaries as the single input to federated graph analysis.
- Modify `boomerang_builder/src/assembly/build.rs`: remove the separately reconstructed zero-delay graph and pass final RTI/connection artifacts through runtime lowering.
- Modify `boomerang_builder/src/lib.rs`: replace the RTI-owned topology error wrapper with focused assembly graph errors.
- Create `boomerang_federated/src/rti/graph.rs`: immutable dense `RtiGraph` and the doc-hidden final-parts handoff used by builder lowering.
- Modify `boomerang_federated/src/rti/index.rs`: retain only immutable RTI records used at runtime; remove neighbor/source-manifest caches.
- Modify `boomerang_federated/src/rti/mod.rs`: isolate `RtiRuntimeState`, make `RtiState` consume `RtiGraph`, and retain only runtime protocol/state errors.
- Modify `boomerang_federated/src/protocol.rs`: reduce `Hello` to identity and remove declarative topology types.
- Modify `boomerang_federated/src/client/mod.rs` and `client/tests.rs`: connect without a topology argument.
- Modify `boomerang_federated/src/session.rs`: consume `RtiGraph`, resolve members from it, and trust identity-only `Hello`.
- Modify `boomerang_federated/src/hierarchy.rs`: make `RuntimeFederation` own `RtiGraph` and independent `RuntimeFederate` values.
- Modify `boomerang_federated/src/runtime_bridge.rs`: construct Federate-local bridges directly from lowered routes; remove topology-derived construction.
- Modify `boomerang_federated/src/static_runner.rs`: split the graph from Federates once, move it into the session, and never consult it while starting clients.
- Modify `boomerang_federated/src/static_runner.rs`: replace the single-gateway heuristic with a
  Federate-wide coordination service, and supervise scheduler failures before joining blocked
  peers.
- Modify `boomerang_federated/src/transport.rs`: accept only final `RtiGraph` for RTI TCP startup.
- Modify `boomerang_federated/src/lib.rs`: export `RtiGraph`; remove manifest topology exports.
- Modify `boomerang_federated/src/rti/tests.rs`, session/client/transport/static-runner tests, and `boomerang_builder/src/tests/federated.rs`: relocate tests to their owning phase.
- Modify `docs/federated-runtime.md`: document the final assembly-to-runtime ownership graph.

### Task 1: Make the Trusted Handshake Identity-Only

**Files:**
- Modify: `boomerang_federated/src/protocol.rs:298-324`
- Modify: `boomerang_federated/src/client/mod.rs:190-239`
- Modify: `boomerang_federated/src/client/tests.rs`
- Modify: `boomerang_federated/src/session.rs:182-238`
- Modify: `boomerang_federated/src/rti/mod.rs:604-684`
- Modify: `boomerang_federated/src/session.rs` test module
- Modify: `boomerang_federated/src/transport.rs` test module

- [ ] **Step 1: Write a failing identity-only client handshake test**

In `boomerang_federated/src/client/tests.rs`, change the first Hello assertion to require the final enum shape:

```rust
assert_eq!(
    rti_stream.next().await,
    Some(ProtocolFrame::FederateToRti(FederateToRti::Hello {
        federate_id: fed("source"),
    })),
);
```

Also change one session fixture to construct `FederateToRti::Hello { federate_id }` without a
topology field. This makes the desired protocol API explicit before production code changes.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p boomerang_federated client::tests
```

Expected: compilation fails because `FederateToRti::Hello` still requires `topology` and client
connection still emits a neighbor structure.

- [ ] **Step 3: Remove topology from the handshake path**

Change the protocol variant:

```rust
pub enum FederateToRti {
    Hello {
        federate_id: FederateId,
    },
    // existing Net, Ltc, Msg, and Stop variants remain unchanged
}
```

Change both client constructors to remove `NeighborStructure`:

```rust
pub async fn connect<S, R>(
    federate_id: FederateId,
    sink: S,
    stream: R,
) -> Result<Self, FederateClientError>
```

```rust
pub async fn connect_with_mailbox<S, R>(
    federate_id: FederateId,
    mut sink: S,
    mut stream: R,
    mailbox: FederateClientMailbox,
) -> Result<Self, FederateClientError>
```

Send only identity:

```rust
sink.send(ProtocolFrame::FederateToRti(FederateToRti::Hello {
    federate_id,
}))
.await
.map_err(|error| FederateClientError::Transport(error.into()))?;
```

In `StaticRtiSession::receive_hellos`, match only `federate_id`, retain duplicate and identity
checks, remove `neighbors_for` comparison, and forward the same identity-only message to RTI state.
Update RTI validation matches from `{ federate_id, .. }` to `{ federate_id }`.

- [ ] **Step 4: Update all handshake fixtures and verify GREEN**

Remove topology arguments from `FederateProtocolClient::connect*` call sites and replace all Hello
fixtures with:

```rust
FederateToRti::Hello {
    federate_id: federate_id.clone(),
}
```

Run:

```bash
cargo test -p boomerang_federated
```

Expected: all `boomerang_federated` tests pass.

- [ ] **Step 5: Commit the trusted handshake slice**

```bash
git add boomerang_federated/src/protocol.rs boomerang_federated/src/client/mod.rs boomerang_federated/src/client/tests.rs boomerang_federated/src/session.rs boomerang_federated/src/rti/mod.rs boomerang_federated/src/transport.rs
git commit -m "refactor(federated): trust identity-only hello"
```

### Task 2: Move Federation Graph Analysis into the Builder

**Files:**
- Create: `boomerang_builder/src/federated/graph.rs`
- Modify: `boomerang_builder/src/federated/mod.rs`
- Modify: `boomerang_builder/src/federated/lowering.rs`
- Modify: `boomerang_builder/src/assembly/build.rs:527-730`
- Modify: `boomerang_builder/src/lib.rs:110-120`
- Test: `boomerang_builder/src/federated/graph.rs`
- Test: `boomerang_builder/src/tests/federated.rs`

- [ ] **Step 1: Write failing builder graph-analysis tests**

Add unit tests beside the new graph module for deterministic paths and zero-delay cycles. The
tests exercise stable IDs rather than RTI dense keys:

```rust
#[test]
fn analysis_computes_minimum_nonempty_paths_and_affected_sets() {
    let analysis = analyze_federation_graph(
        vec![fed("isolated"), fed("b"), fed("a"), fed("c")],
        vec![
            edge("a", "b", "a-direct-b", 5),
            edge("a", "c", "a-c", 1),
            edge("c", "b", "c-b", 1),
            edge("b", "a", "b-a", 10),
        ],
    )
    .unwrap();

    assert_eq!(analysis.minimum_delay("a", "b"), Some(2));
    assert_eq!(analysis.minimum_delay("a", "a"), Some(12));
    assert_eq!(analysis.minimum_delay("isolated", "a"), None);
    assert_eq!(analysis.affected_downstream("a"), [fed("b"), fed("c")]);
}

#[test]
fn analysis_rejects_zero_delay_cycles() {
    let error = analyze_federation_graph(
        vec![fed("a"), fed("b")],
        vec![edge("a", "b", "a-b", 0), edge("b", "a", "b-a", 0)],
    )
    .unwrap_err();

    assert!(matches!(error, AssemblyError::FederationZeroDelayCycle { .. }));
}
```

Add cases for parallel endpoints, deterministic input reordering, duplicate endpoints, positive
delay cycles, disconnected members, and a path whose accumulated delay exceeds `u64::MAX`.

- [ ] **Step 2: Run the graph tests and verify RED**

Run:

```bash
cargo test -p boomerang_builder --features federated federated::graph::tests
```

Expected: compilation fails because `federated::graph` and the focused assembly error variants do
not exist.

- [ ] **Step 3: Implement the builder-owned analyzed graph**

Create these builder-internal records in `federated/graph.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FederationEndpoint {
    pub(crate) source: FederateId,
    pub(crate) target: FederateId,
    pub(crate) endpoint: EndpointId,
    pub(crate) delay: WireDelay,
}

#[derive(Debug)]
pub(crate) struct AnalyzedFederationGraph {
    pub(crate) federates: Vec<FederateId>,
    pub(crate) endpoints: Vec<FederationEndpoint>,
    pub(crate) transitive_incoming:
        BTreeMap<FederateId, Vec<(FederateId, WireDelay)>>,
    pub(crate) affected_downstream: BTreeMap<FederateId, Vec<FederateId>>,
}
```

Implement `analyze_federation_graph` by sorting/deduplicating stable IDs, allocating a
`petgraph::stable_graph::StableDiGraph<FederateId, u128>`, preserving every endpoint separately,
and using `u128` weights during path analysis. For each source, seed path search with every
outgoing edge before applying Dijkstra; this preserves the minimum nonempty cycle distance from a
node to itself. Convert final distances to `WireDelay` only after checking `u64::try_from`.

Build a zero-delay-only graph and run `petgraph::algo::toposort`; translate a cycle into:

```rust
AssemblyError::FederationZeroDelayCycle {
    federates: cycle_ids,
}
```

Add focused errors in `boomerang_builder/src/lib.rs`:

```rust
#[error("duplicate Federate id `{federate_id}`")]
DuplicateFederateId { federate_id: String },

#[error("duplicate federated endpoint `{endpoint}`")]
DuplicateFederatedEndpoint { endpoint: String },

#[error("distributed zero-delay cycle: {federates:?}")]
FederationZeroDelayCycle { federates: Vec<String> },

#[error("minimum path delay from `{source}` to `{target}` exceeds u64 nanoseconds")]
FederationPathDelayOverflow { source: String, target: String },
```

- [ ] **Step 4: Route existing assembly validation through the analyzed graph**

In `lower_federation`, first project each `PartitionAnalysis::federated_boundaries()` item into a
`FederationEndpoint`, then call `analyze_federation_graph`. Delete
`Assembly::validate_federation_zero_delay_cycles` and its call from `build_partition_analysis` so
only the builder graph module performs graph validation.

Keep `PartitionAnalysis` as the sole owner of Federate placement and cross-partition boundary
inputs; do not add a second topology collection to `Assembly`.

- [ ] **Step 5: Verify builder graph behavior is GREEN**

Run:

```bash
cargo test -p boomerang_builder --features federated federated::graph::tests
cargo test -p boomerang_builder --features federated tests::federated
```

Expected: graph-analysis and existing federated builder tests pass.

- [ ] **Step 6: Commit builder-owned graph analysis**

```bash
git add boomerang_builder/src/federated/graph.rs boomerang_builder/src/federated/mod.rs boomerang_builder/src/federated/lowering.rs boomerang_builder/src/assembly/build.rs boomerang_builder/src/lib.rs boomerang_builder/src/tests/federated.rs
git commit -m "refactor(builder): analyze federation graph during lowering"
```

### Task 3: Define the Final Immutable `RtiGraph` Contract

**Files:**
- Create: `boomerang_federated/src/rti/graph.rs`
- Modify: `boomerang_federated/src/rti/mod.rs`
- Test: `boomerang_federated/src/rti/graph.rs`

- [ ] **Step 1: Write failing tests against final graph parts**

Add tests in `rti/graph.rs` against the intended final-parts API:

```rust
#[test]
fn rti_graph_interns_final_parts_without_retaining_source_data() {
    let graph = RtiGraph::from_lowered(RtiGraphParts {
        federates: vec![
            federate_parts("a", vec![], vec!["b", "c"]),
            federate_parts("b", vec![("a", 0)], vec!["c"]),
            federate_parts("c", vec![("a", 1), ("b", 2)], vec![]),
        ],
        endpoints: vec![
            endpoint_parts("a", "b", "a-b", 0),
            endpoint_parts("a", "c", "a-c", 1),
            endpoint_parts("b", "c", "b-c", 2),
        ],
    });

    let a = graph.federate_key(&fed("a")).unwrap();
    let b = graph.federate_key(&fed("b")).unwrap();
    let c = graph.federate_key(&fed("c")).unwrap();
    assert_eq!(graph.federate_id(a), &fed("a"));
    assert_eq!(graph.affected_downstream(a), &[b, c]);
    assert!(graph.contains_route(&fed("a"), &fed("c"), &endpoint("a-c")));
}
```

- [ ] **Step 2: Run focused RTI tests and verify RED**

Run:

```bash
cargo test -p boomerang_federated rti::graph::tests
```

Expected: compilation fails because `RtiGraph`, `RtiGraphParts`, and final part records do not
exist.

- [ ] **Step 3: Implement immutable graph records and mechanical interning**

Create `rti/graph.rs` with a public immutable graph and doc-hidden builder handoff:

```rust
pub struct RtiGraph {
    federates: TinyMap<FederateKey, RtiFederate>,
    federate_keys: BTreeMap<FederateId, FederateKey>,
    endpoints: TinyMap<EndpointKey, RtiEndpoint>,
    endpoint_keys: BTreeMap<EndpointId, EndpointKey>,
}

#[doc(hidden)]
pub struct RtiGraphParts {
    pub federates: Vec<RtiFederateParts>,
    pub endpoints: Vec<RtiEndpointParts>,
}

#[doc(hidden)]
pub struct RtiFederateParts {
    pub id: FederateId,
    pub transitive_incoming: Vec<(FederateId, WireDelay)>,
    pub affected_downstream: Vec<FederateId>,
}

#[doc(hidden)]
pub struct RtiEndpointParts {
    pub id: EndpointId,
    pub source: FederateId,
    pub target: FederateId,
    pub delay: WireDelay,
}
```

`RtiGraph::from_lowered` sorts stable IDs, interns them once, mechanically translates final paths,
and derives direct incoming dependencies from endpoint parts. It performs no cycle, reachability,
or shortest-path analysis. It has no `Clone` implementation and stores no `FederatedTopology`,
neighbor views, direct-downstream test cache, or all-pairs minimum-delay map.

Keep `graph` private inside the `rti` module during this task. This is an internal final-shape
contract used to drive the end-to-end migration in Task 4, not a public compatibility API.

- [ ] **Step 4: Verify the graph contract is GREEN**

Run:

```bash
cargo test -p boomerang_federated rti::graph::tests
```

Expected: final-parts interning, dense identities, route lookup, and affected-set tests pass while
the new module remains private.

- [ ] **Step 5: Commit the private final graph contract**

```bash
git add boomerang_federated/src/rti/graph.rs boomerang_federated/src/rti/mod.rs
git commit -m "refactor(federated): define final RTI graph contract"
```

### Task 4: Plumb `RtiGraph` and Star-Shaped Ownership End to End

**Files:**
- Modify: `boomerang_federated/src/runtime_bridge.rs:181-244`
- Modify: `boomerang_federated/src/rti/index.rs`
- Modify: `boomerang_federated/src/rti/mod.rs`
- Modify: `boomerang_federated/src/rti/tests.rs`
- Modify: `boomerang_federated/src/lib.rs`
- Modify: `boomerang_federated/src/hierarchy.rs`
- Modify: `boomerang_federated/src/static_runner.rs`
- Modify: `boomerang_federated/src/session.rs`
- Modify: `boomerang_federated/src/transport.rs`
- Modify: `boomerang_builder/src/federated/lowering.rs`
- Modify: `boomerang_builder/src/federated/graph.rs`
- Modify: `boomerang_builder/src/assembly/build.rs`
- Test: `boomerang_federated/src/static_runner.rs` test module
- Test: `boomerang/tests/federated_static.rs`

- [ ] **Step 1: Write a failing runner ownership test**

Add a test that consumes `RuntimeFederation` into one graph and independent Federates, then starts
client preparation using only a selected `RuntimeFederate`:

```rust
#[test]
fn runtime_federate_is_complete_without_rti_graph_access() {
    let runtime = lowered_test_federation();
    let (graph, mut federates) = runtime.into_parts();
    let source = federates.remove(&fed("source")).unwrap();
    let (id, enclaves, bridge) = source.into_parts();

    assert_eq!(id, fed("source"));
    assert!(!enclaves.is_empty());
    assert!(bridge.routes().all(|route| route.target == id));
    assert_eq!(graph.federate_count(), 2);
}
```

Add an integration assertion that the existing static federation still runs after the graph is
moved into the RTI task.

- [ ] **Step 2: Run runner tests and verify RED**

Run:

```bash
cargo test -p boomerang_federated --features runtime static_runner::tests::runtime_federate_is_complete
```

Expected: compilation fails because the hierarchy and runner still expose/consume
`CompiledTopology` and derive client configuration from it.

- [ ] **Step 3: Replace compiled topology with graph plus dense runtime state**

Promote the private graph contract as `boomerang_federated::RtiGraph`. Change `rti/index.rs` to
contain only `RtiFederate`, `RtiEndpoint`, `IncomingDependency`, and `IncomingPath`; remove neighbor
views, source-manifest data, direct-downstream test caches, and the retained all-pairs delay map.
Re-export only `RtiGraph` as normal public API. Re-export `RtiGraphParts`, `RtiFederateParts`, and
`RtiEndpointParts` with `#[doc(hidden)]` for the cross-crate builder handoff; give them no serde
implementation and no raw-topology convenience constructor.

Use the final state split in `rti/mod.rs`:

```rust
#[derive(Debug, Clone)]
struct RtiRuntimeState {
    federates: TinySecondaryMap<FederateKey, FederateCoordination>,
}

#[derive(Debug)]
pub struct RtiState {
    graph: RtiGraph,
    runtime: RtiRuntimeState,
}

impl RtiState {
    pub fn from_graph(graph: RtiGraph) -> Self {
        let federates = graph
            .federates()
            .map(|(key, _)| (key, FederateCoordination::default()))
            .collect();
        Self {
            graph,
            runtime: RtiRuntimeState { federates },
        }
    }
}
```

Rename immutable accesses from `self.topology` to `self.graph` and mutable coordination accesses
from `self.federates` to `self.runtime.federates`. Remove public raw-topology construction and
`from_compiled`. Convert RTI behavioral fixtures to explicit `RtiGraphParts` through one
crate-private test helper. Delete the old topology-compilation tests only after confirming their
duplicate, cycle, path, and overflow cases are covered by Task 2's builder graph tests.

- [ ] **Step 4: Project analyzed builder data directly into graph parts**

Add a borrowing conversion that performs no graph algorithms and leaves endpoint data available
for Federate-local route construction:

```rust
pub(crate) fn to_rti_graph(&self) -> boomerang_federated::RtiGraph {
    boomerang_federated::RtiGraph::from_lowered(RtiGraphParts {
        federates: self
            .federates
            .iter()
            .cloned()
            .map(|id| RtiFederateParts {
                transitive_incoming: self.transitive_incoming[&id].clone(),
                affected_downstream: self.affected_downstream[&id].clone(),
                id,
            })
            .collect(),
        endpoints: self.endpoints.iter().cloned().map(Into::into).collect(),
    })
}
```

Change `FederationLoweringArtifacts.topology` to `rti_graph: RtiGraph` and pass that graph through
`Assembly::into_runtime_assembly` without compiling or validating it again.

- [ ] **Step 5: Build Federate-local bridges during lowering**

Remove `FederatedRuntimeConnections::from_topology`. In builder lowering, create routes from the
same analyzed endpoint records used for `RtiGraph`:

```rust
let connections = FederatedRuntimeConnections::new(
    analyzed.federates.iter().cloned(),
    analyzed.endpoints.iter().map(|edge| {
        FederateClientRoute::new(
            edge.endpoint.clone(),
            edge.source.clone(),
            edge.target.clone(),
        )
    }),
)?;
```

Make `FederationLoweringArtifacts` carry both `rti_graph` and `connections`. Change
`StaticFederationRuntime::new` to accept both and perform no derivation:

```rust
pub fn new(graph: RtiGraph, connections: FederatedRuntimeConnections) -> Self {
    Self { graph, connections }
}
```

- [ ] **Step 6: Make the runtime hierarchy own final artifacts**

Change `RuntimeFederation` to:

```rust
pub struct RuntimeFederation {
    graph: RtiGraph,
    federates: BTreeMap<FederateId, RuntimeFederate>,
}

pub fn into_parts(self) -> (RtiGraph, BTreeMap<FederateId, RuntimeFederate>) {
    (self.graph, self.federates)
}
```

`RuntimeFederation::from_lowered` pairs the already-complete Enclave maps and bridges by iterating
the lowering-owned Federate IDs. Remove graph-based client configuration and second-pass topology
validation.

- [ ] **Step 7: Move the graph once and start clients from Federates only**

In `prepare_static_federation`, consume the hierarchy into:

```rust
struct PreparedStaticFederation {
    graph: RtiGraph,
    federates: BTreeMap<FederateId, RuntimeFederate>,
}
```

Create transports by iterating `federates.keys()`. Move `graph` into `StaticRtiSession::new` or the
TCP RTI task. Change `connect_clients` to accept only Federate-local bridges and transports:

```rust
fn connect_clients<S, R>(
    tokio_runtime: &tokio::runtime::Runtime,
    federates: BTreeMap<FederateId, RuntimeFederate>,
    transports: BTreeMap<FederateId, (S, R)>,
) -> Result<
    (
        BTreeMap<FederateId, TinyMap<EnclaveKey, Enclave>>,
        BTreeMap<FederateId, ConnectedFederate>,
    ),
    StaticFederationRunnerError,
>
```

Call `FederateProtocolClient::connect_with_mailbox(id, sink, stream, mailbox)` with no RTI graph
or neighbor data. Split each consumed `RuntimeFederate` with `into_parts`, retain its Enclave map
for scheduler execution, and consume its bridge into mailbox, routes, and fault state.

Before moving each route list into `RtiLogicalTimeCoordinator`, record `has_inbound_routes` for
that Federate. Change `federate_has_no_initial_work` to accept this local boolean instead of the
global topology:

```rust
fn federate_has_no_initial_work(
    enclave: &boomerang_runtime::Enclave,
    has_inbound_routes: bool,
) -> bool {
    enclave.env.reactions.is_empty()
        || (enclave.graph.startup_actions.is_empty()
            && enclave.upstream_enclaves.is_empty()
            && !has_inbound_routes)
}
```

- [ ] **Step 8: Remove runner topology validation and raw TCP construction**

Delete `validate_static_runner_runtime`, `UnsupportedTopology`, and checks that compare global
topology edges with Federate bridges. These are lowering invariants.

Replace the two TCP RTI entry points with one final-artifact function:

```rust
pub(crate) async fn run_tcp_static_rti_session(
    listener: TcpListener,
    graph: RtiGraph,
) -> Result<(), SessionError>
```

Use `graph.federate_ids()` to validate accepted Hello identities, then move the graph into
`StaticRtiSession::new(graph, endpoints)`.

- [ ] **Step 9: Verify star-shaped runtime behavior is GREEN**

Run:

```bash
cargo test -p boomerang_federated --all-features static_runner
cargo test -p boomerang_federated --all-features rti::tests
cargo test -p boomerang_builder --features federated tests::federated
cargo test -p boomerang --all-features federated_static
```

Expected: unit and end-to-end in-memory/TCP federation tests pass.

- [ ] **Step 10: Commit the graph/state/runtime ownership slice**

```bash
git add boomerang_federated/src/rti boomerang_federated/src/lib.rs boomerang_federated/src/runtime_bridge.rs boomerang_federated/src/hierarchy.rs boomerang_federated/src/static_runner.rs boomerang_federated/src/session.rs boomerang_federated/src/transport.rs boomerang_builder/src/federated/graph.rs boomerang_builder/src/federated/lowering.rs boomerang_builder/src/assembly/build.rs boomerang/tests/federated_static.rs
git commit -m "refactor(federated): plumb final RTI graph ownership"
```

### Task 5: Coordinate Every Enclave and Make Scheduler Failure Fail Fast

**Files:**
- Modify: `boomerang_federated/src/static_runner.rs`
- Modify: `boomerang_federated/src/client/mod.rs` only if the protocol coordinator needs a
  non-blocking polling boundary
- Test: `boomerang_federated/src/static_runner.rs` test module
- Test: `boomerang/tests/federated_static.rs`

- [ ] **Step 1: Write a failing multi-Enclave frontier test**

Build a Federate with at least two independently runnable Enclaves where the Enclave that does not
match the old gateway heuristic has an inbound federated route. Hold back the RTI grant for that
Federate and assert through a bounded observation channel that neither Enclave advances beyond the
withheld tag. Then deliver a message at that tag, grant the tag, and assert that the receiving
Enclave observes the message before executing later-tag work.

Name the focused test
`static_runner::tests::all_enclaves_participate_in_federate_rti_frontier`. It must fail against the
single-gateway implementation because the non-gateway scheduler uses only local coordination.

- [ ] **Step 2: Write a failing bounded panic-supervision test**

Create a static federation in which one scheduler reaction panics while a peer scheduler is
waiting for an RTI grant. Run the federation behind a test-only bounded completion channel and
assert that it returns `StaticFederationRunnerError::SchedulerThreadPanic` within the timeout.
The test must fail against the current join-first implementation by timing out. Do not add a
runtime timeout as the fix.

Name the focused test `static_runner::tests::scheduler_panic_stops_waiting_peers`.

- [ ] **Step 3: Replace the gateway with a Federate coordination service**

In `boomerang_federated/src/static_runner.rs`, create one coordination service per Federate. The
service owns its `RtiLogicalTimeCoordinator` and runs a dedicated request loop. Give every active
Enclave a participant proxy implementing `boomerang_runtime::LogicalTimeCoordinator`; the proxy
contains its `EnclaveKey` and request/reply channels, but no RTI graph or protocol transport.

Track each active participant's requested tag, completed tag, and terminal state. Request the
Federate's minimum safe next tag only after the participant frontier is known, release only the
participants covered by the returned grant, and report LTC only when every active participant has
completed that tag or moved beyond it. Remove a normally finished participant from subsequent
frontier calculations. Serialize protocol access in the service loop; do not hold a shared mutex
across a blocking RTI wait that prevents participant progress from reaching the service.

Delete `gateway_enclaves` and the participant count of one. Keep the generic coordinator interface
in `boomerang_runtime`; all Federate identity, RTI protocol, transport, and Tokio knowledge remains
in `boomerang_federated`.

- [ ] **Step 4: Supervise completion before ordered joins**

Have every scheduler thread report its terminal result, including a caught panic payload, over a
completion channel. The runner waits for completion reports rather than blocking on handles in
spawn order. On the first panic or scheduler error, force-stop every Federate coordination
service and abort or close the RTI session before joining remaining scheduler threads. Preserve
the first terminal error as the returned error; retain cleanup failures only as secondary
diagnostics.

On normal completion, remove each Enclave participant from its Federate frontier. Send exactly one
Stop after the last participant finishes, then join all scheduler threads and await the RTI
session.

- [ ] **Step 5: Run focused and end-to-end tests**

Run:

```bash
cargo test -p boomerang_federated --all-features static_runner::tests::all_enclaves_participate_in_federate_rti_frontier
cargo test -p boomerang_federated --all-features static_runner::tests::scheduler_panic_stops_waiting_peers
cargo test -p boomerang_federated --all-features static_runner
cargo test -p boomerang --all-features federated_static
```

Expected: the multi-Enclave frontier test proves that no sibling scheduler bypasses RTI grants;
the panic test returns the expected error within its bound; all static federation tests pass.

- [ ] **Step 6: Commit Federate-wide coordination and supervision**

```bash
git add boomerang_federated/src/static_runner.rs boomerang_federated/src/client/mod.rs boomerang/tests/federated_static.rs
git commit -m "fix(federated): coordinate all federate enclaves"
```

### Task 6: Remove Raw Topology APIs and Separate Error Phases

**Files:**
- Modify: `boomerang_federated/src/protocol.rs`
- Modify: `boomerang_federated/src/lib.rs`
- Modify: `boomerang_federated/src/rti/mod.rs`
- Modify: `boomerang_federated/src/session.rs`
- Modify: `boomerang_federated/src/transport.rs`
- Modify: `boomerang_federated/src/client/tests.rs`
- Modify: `boomerang_federated/src/rti/tests.rs`
- Modify: `boomerang_federated/src/session.rs` test module
- Modify: `boomerang_federated/src/transport.rs` test module
- Modify: `boomerang_builder/src/lib.rs`
- Modify: `boomerang_builder/src/tests/federated.rs`

- [ ] **Step 1: Move topology-failure assertions to builder tests**

Add builder tests that assert the focused `AssemblyError` variants for duplicate Federate IDs,
duplicate endpoints, zero-delay cycles, and accumulated-delay overflow. Use real `Assembly`
declarations for duplicate IDs and zero-delay cycles; use the builder graph module's unit fixture
for the `u64::MAX + 1` path that normal runtime durations cannot express.

Delete the corresponding `CompiledTopology`/`RtiError` assertions from `rti/tests.rs` only after
the builder tests are in place.

- [ ] **Step 2: Run ownership tests and verify RED where responsibility moved**

Run:

```bash
cargo test -p boomerang_builder --features federated tests::federated
```

Expected before completing the move: new assertions fail or do not compile because some graph
failures are still represented as `RtiError`/generic federation errors.

- [ ] **Step 3: Delete declarative topology and compile-time RTI errors**

Remove these types and all exports:

```text
FederatedTopology
TopologyEdge
NeighborStructure
CompiledTopology
```

Remove these `RtiError` variants because the builder now owns them:

```text
DuplicateFederate
UndeclaredEdgeFederate
MissingRouteEndpoint
DuplicateRoute
ConflictingRoute
PathDelayOverflow
```

Remove `AssemblyError::FederationTopology(#[from] RtiError)`. Keep only live RTI errors for
identity, route messages, tags, lifecycle, regressions, and runtime tag arithmetic.

- [ ] **Step 4: Remove compatibility constructors and update test fixtures**

Delete raw-topology constructors from `RtiState`, `StaticRtiSession`, TCP transport helpers,
`StaticFederationRuntime`, and runner tests. Update remaining tests to use explicit final graph
parts or real builder lowering.

Verify absence with:

```bash
rg -n "FederatedTopology|TopologyEdge|NeighborStructure|CompiledTopology|from_compiled|from_topology" \
  boomerang_federated boomerang_builder boomerang
```

Expected: no matches, except historical design/plan documentation outside those source roots.

- [ ] **Step 5: Run crate tests and verify GREEN**

Run:

```bash
cargo test -p boomerang_federated --all-features
cargo test -p boomerang_builder --all-features
```

Expected: both crates pass with topology failures owned exclusively by builder tests.

- [ ] **Step 6: Commit API and error cleanup**

```bash
git add boomerang_federated/src boomerang_builder/src
git commit -m "refactor(federated): remove raw RTI topology APIs"
```

### Task 7: Document and Verify the Complete Boundary Refactor

**Files:**
- Modify: `docs/federated-runtime.md`
- Modify: `docs/superpowers/plans/2026-07-19-federated-rti-boundaries.md` (checkboxes only during execution)
- Verify: all workspace crates and tests

- [ ] **Step 1: Update architecture documentation**

Replace the `CompiledTopology` diagram and narrative with:

```text
RuntimeFederation
├── RtiGraph -> consumed by StaticRtiSession/RtiState
└── FederateId -> RuntimeFederate
    ├── TinyMap<EnclaveKey, Enclave>
    └── FederateRuntimeBridge -> consumed by FederateProtocolClient
```

Document that `PartitionAnalysis` and builder graph analysis compute all reachability and delays,
that Hello carries only identity, and that runtime graph/state use separate immutable/dense types.

- [ ] **Step 2: Format and verify formatting**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
```

Expected: the check exits successfully with no output.

- [ ] **Step 3: Run the full workspace test suite**

Run:

```bash
cargo test --workspace --all-features
```

Expected: all workspace tests pass with zero failures.

- [ ] **Step 4: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: Clippy exits successfully with zero warnings.

- [ ] **Step 5: Verify architectural removals and review the diff**

Run:

```bash
rg -n "FederatedTopology|TopologyEdge|NeighborStructure|CompiledTopology|from_compiled|from_topology" \
  boomerang_federated boomerang_builder boomerang
git diff --check
git status --short
```

Expected: the search has no source matches, `git diff --check` succeeds, and status contains only
the intended implementation/documentation changes plus pre-existing user-owned files.

- [ ] **Step 6: Commit documentation and final mechanical cleanup**

```bash
git add docs/federated-runtime.md docs/superpowers/plans/2026-07-19-federated-rti-boundaries.md
git commit -m "docs: describe federated RTI phase boundaries"
```

- [ ] **Step 7: Perform a final requirement audit**

Re-read `docs/superpowers/specs/2026-07-19-federated-rti-boundaries-design.md` and confirm:

```text
[ ] Assembly lowering is the only RtiGraph producer.
[ ] RtiGraph retains no declarative source topology or client neighbor structures.
[ ] Runtime RTI state is a separate dense secondary map.
[ ] RuntimeFederate startup does not receive or query RtiGraph.
[ ] Hello contains only FederateId.
[ ] Graph validation and path computation live in boomerang_builder.
[ ] Every active Enclave participates in its Federate's RTI logical-time frontier.
[ ] A scheduler panic stops waiting peers before joins and returns SchedulerThreadPanic.
[ ] No compatibility topology constructors or aliases remain.
[ ] In-memory and TCP end-to-end tests pass.
```

If any box cannot be checked from source and fresh command output, do not report completion; fix
the uncovered requirement and repeat Steps 2-5.

## Plan Revision Note

2026-08-13: Reconciled the architecture review with the approved design. Added Task 5 because the
existing single-gateway heuristic does not coordinate every Enclave in a Federate and the current
join-before-force-stop ordering can hang after a scheduler panic. The task preserves the existing
wire protocol and RTI grant state machine while making the static runner project those semantics
across all Federate-owned schedulers and fail fast as one supervised unit.
