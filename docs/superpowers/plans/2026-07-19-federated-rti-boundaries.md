# Federated RTI Phase Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make assembly lowering the only producer of an immutable RTI graph, give each runtime Federate only its local data, and separate that graph from dense mutable RTI coordination state.

**Architecture:** Builder-owned federation graph analysis derives final RTI graph parts and per-Federate bridges from `PartitionAnalysis`. `boomerang_federated` mechanically interns those final parts into `RtiGraph`; sessions own the graph and a separate `RtiRuntimeState`, while clients consume only `RuntimeFederate` values. At runtime, every Enclave publishes its frontier before local blocking, one isolated Federate service aggregates those publications and projects cached RTI grants through participant proxies, and runner supervision stops blocked peers before joining after failure. Raw topology manifests, topology-bearing `Hello`, and compatibility constructors are removed end to end.

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
- Modify `boomerang_runtime/src/event.rs` and `sched/`: add only generic split-phase frontier
  publication, coordination wake, blocking acquisition, and completion concepts.
- Create `boomerang_federated/src/federate_coordination/`: isolate the pure participant-frontier
  state machine from its threaded service and tests.
- Create `boomerang_federated/src/client/coordination/mod.rs` and `tests.rs`: separate
  Federate-to-RTI logical-time protocol progress and its tests from transport/client connection
  lifecycle.
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
- Modify: `boomerang_runtime/src/event.rs`
- Modify: `boomerang_runtime/src/sched/barrier.rs`
- Modify: `boomerang_runtime/src/sched/mod.rs`
- Create: `boomerang_federated/src/federate_coordination/mod.rs`
- Create: `boomerang_federated/src/federate_coordination/layout.rs`
- Create: `boomerang_federated/src/federate_coordination/state.rs`
- Create: `boomerang_federated/src/federate_coordination/service.rs`
- Create: `boomerang_federated/src/federate_coordination/tests.rs`
- Modify: `boomerang_federated/src/lib.rs`
- Modify: `boomerang_federated/src/client/mod.rs`
- Create: `boomerang_federated/src/client/coordination/mod.rs`
- Create: `boomerang_federated/src/client/coordination/tests.rs`
- Modify: `boomerang_federated/src/static_runner.rs`
- Test: `boomerang/tests/federated_static.rs`

This task changes runtime behavior but does not change build or lowering data. The phase boundary
is mandatory:

```text
build
  Assembly declarations and placement
    -> unchanged

lowering
  PartitionAnalysis -> RtiGraph + RuntimeFederate bridges/routes/Enclaves
    -> unchanged; no participant frontier or service state is stored here

runtime
  RuntimeFederate consumption
    -> immutable participant layout and route ownership
    -> per-run coordination service, channels, frontier versions, grants, rounds, and supervision
```

Do not add local Enclave dependencies to `RtiGraph`, do not give the Federate service access to
`RtiGraph`, and do not put mutable participant state into `RuntimeFederation`,
`RuntimeFederate`, builder analysis, or lowering artifacts. If implementation appears to require
any of those changes, stop for design review.

The code ownership boundary is also mandatory:

- `boomerang_runtime` defines only generic scheduler coordination concepts: published frontier,
  acquisition, completion, and a generic coordination wake. It contains no Federate identity,
  RTI message, route, endpoint, transport, or Tokio type.
- `boomerang_runtime/src/event.rs` owns only the generic wake payload and `AsyncEvent` variant;
  `sched/barrier.rs` owns the generic frontier/publication contract and consumed-wake tracking;
  `sched/mod.rs` owns scheduler ordering and publishes from queue/idle/shutdown transitions.
- `client/coordination/mod.rs` owns the single
  Federate-to-RTI protocol state machine: NET submission, TAG/MSG polling, LTC, Stop, route
  admission, and terminal protocol failure. It does not aggregate Enclaves or decide whether a
  returned TAG covers a participant request.
- `federate_coordination/state.rs` is a pure, synchronous transition model for dynamic
  participant publications, acquire supersession, grant coverage, observation epochs,
  completions, and terminal state. It performs no I/O, threading, transport, or scheduler
  construction.
- `federate_coordination/layout.rs` owns the immutable, sorted participant-key layout created only
  after `RuntimeFederate` consumption. It contains no channels, protocol client, mutable frontier,
  grant, epoch, or completion state.
- `federate_coordination/service.rs` owns channels and the dedicated service thread, applies the
  pure state transitions, and drives the protocol coordinator. It contains no builder/lowering
  logic and never queries the global RTI graph.
- `static_runner.rs` constructs services and participant proxies, starts schedulers, observes
  terminal results, orders shutdown, and joins resources. It does not contain the frontier state
  machine or protocol polling algorithm.

Immutable runtime layout, per-run resources, and mutable per-run state must remain distinct:

```text
Immutable runtime layout          Per-run resources/capabilities       Mutable service/protocol state
-------------------------------   ----------------------------------   ----------------------------------
FederateId                        participant request/reply channels   publication sequence and frontier
sorted owned EnclaveKey set       participant wake SendContext         logical pending acquire ids
Federate-local routes             protocol client/mailbox ownership    advertised NET and grant coverage
                                  service/scheduler thread handles     round tag and observation epoch
                                  completion/session handles           epoch acknowledgements/certificates
                                                                       completed tags and first failure
                                                                       protocol lifecycle and Stop state
```

Here, "immutable runtime layout" means fixed for the lifetime of one execution after
`RuntimeFederate` is consumed; it does not mean stored in builder declarations or lowering output.
The participant set is exactly every Enclave owned by the consumed `RuntimeFederate`. Federated
execution does not prune an Enclave merely because it has no startup work: such an Enclave may be
an upstream local-barrier participant. The pure state machine records logical pending requests by
participant key and request id, never channel senders, wake handles, thread handles, or protocol
clients. Those capabilities and resources belong to the service layer.

- [ ] **Step 0: Quarantine the failed prototype and re-establish the Task 4 baseline**

The working tree contains an uncommitted Task 5 prototype in `client/mod.rs` and
`static_runner.rs`. Inspect its diff before editing. Preserve the two tests that proved the
single-gateway bypass and join-first hang, but do not retain or commit its production service
implementation wholesale. Rework the production code into the modules above.

Before new production work, run only named Task 4 tests from committed `HEAD` behavior after
removing prototype production hunks. Do not use a broad `static_runner` filter because it also
selects the two preserved RED Task 5 tests:

```bash
cargo test -p boomerang_federated --all-features static_runner::tests::federate_maps_allocate_independent_dense_keys
cargo test -p boomerang_federated --all-features static_runner::tests::preparation_preserves_dense_enclave_keys
cargo test -p boomerang_federated --all-features static_runner::tests::runtime_federate_is_complete_without_rti_graph_access
cargo test -p boomerang_federated --all-features rti::tests
cargo test -p boomerang --all-features --test federated_static public_api_runs_static_in_memory_federation
```

Expected: the committed Task 4 behavior passes; the preserved Task 5 tests remain RED for their
original reasons. Do not use destructive Git commands or disturb unrelated user-owned changes.

Then run both preserved tests explicitly behind their existing test-only bounds:

```bash
cargo test -p boomerang_federated --all-features static_runner::tests::all_enclaves_participate_in_federate_rti_frontier
cargo test -p boomerang_federated --all-features static_runner::tests::scheduler_panic_stops_waiting_peers
```

Expected RED: the frontier test reports that the non-gateway Enclave advanced beyond the withheld
frontier, and the panic test reports the bounded join-first timeout. Record both intended failure
modes; an unrelated compile error or different failure is not valid RED evidence.

- [ ] **Step 1: Define the generic split-phase scheduler contract with failing runtime tests**

Add runtime tests that characterize the required ordering and interruption behavior:

```text
candidate publication
  -> local barrier acquire
  -> blocking external grant acquire
  -> reaction processing
  -> local downstream release
  -> external completion
```

Required focused tests:

- `scheduler_publishes_candidate_before_local_acquire`;
- `scheduler_publishes_idle_before_waiting_for_async_event`;
- `local_acquire_still_precedes_external_grant_wait`;
- `earlier_interruption_republishes_candidate_before_retry`;
- `coordination_wake_rechecks_an_idle_frontier`;
- `unrelated_async_event_does_not_acknowledge_sent_coordination_wake`;
- publication or wake failure prevents reaction execution and retains the concrete source error.

Run the focused tests and verify RED because the trait has no publication phase and idle schedulers
cannot be woken for a grant round.

Introduce only generic runtime concepts:

```rust
pub enum LogicalTimeFrontier {
    Candidate(Tag),
    Idle,
    Finished,
}

pub struct CoordinationWake {
    pub tag: Tag,
    pub observation_epoch: u64,
}

pub struct FrontierPublication {
    pub frontier: LogicalTimeFrontier,
    pub consumed_wake: Option<CoordinationWake>,
}

pub trait LogicalTimeCoordinator: Send {
    fn publish_frontier(
        &mut self,
        publication: FrontierPublication,
    ) -> Result<(), CoordinationError>;

    fn acquire(
        &mut self,
        tag: Tag,
        event_rx: &Receiver<AsyncEvent>,
    ) -> Result<CoordinationOutcome, CoordinationError>;

    fn complete(&mut self, tag: Tag) -> Result<(), CoordinationError>;
}
```

Add a mandatory generic `AsyncEvent::CoordinationWake(CoordinationWake)` variant. The event carries
the exact `(Tag, observation_epoch)` pair, not a Federate id, route, protocol frame, or RTI grant
type. Scheduler coordination records a wake only when that exact event is consumed, and the next
publication reports only that consumed pair. It must not sample a shared latest-epoch value, and an
unrelated async event must not acknowledge a wake that was merely sent. The scheduler publishes
`Candidate` immediately after selecting its queue head and before local acquisition; it publishes
`Idle` before blocking for asynchronous input; normal scheduler shutdown is the sole publisher of
`Finished`. An interruption restarts queue selection and replaces the previous candidate before
another local or external acquire.

Run:

```bash
cargo test -p boomerang_runtime sched::tests
cargo test -p boomerang_runtime sched::barrier::tests
```

Expected: all generic scheduler coordination tests pass, including the existing ordering and
failure-atomicity characterization tests. `boomerang_runtime` remains free of federated knowledge.

- [ ] **Step 2: Implement the generic contract and mechanically keep all implementations compiling**

Implement the generic runtime contract and scheduler behavior. Mechanically update every existing
trait implementation so the focused runtime crate can compile. Until the runner is migrated in
Step 6, the old direct `RtiLogicalTimeCoordinator` implementation may contain a clearly marked,
uncommitted no-op publication shim solely to keep the old runner buildable; do not add a public
adapter or commit that state. Step 6 must remove the direct implementation and the shim when the
participant proxy becomes the only federated trait implementation.

Run the focused runtime tests from Step 1. Do not run or require a workspace check yet, and do not
commit. Tasks 5 Steps 1-7 are one uncommitted TDD sequence because the trait, participant proxy,
service, and runner migration form one compile boundary.

- [ ] **Step 3: Write failing pure state-machine tests for Federate aggregation**

Create `federate_coordination/layout.rs` with an immutable `FederateCoordinationLayout` containing
the sorted set of every Enclave key owned by the consumed `RuntimeFederate`. "Active" means an
owned participant that has not authoritatively published `Finished`; it never means "has startup
work." Create `federate_coordination/state.rs` as a pure transition model before adding service
threads or protocol I/O. Its immutable construction input is that layout.
Its private mutable state includes, per participant, the latest publication sequence, frontier,
pending `(request_id, publication_sequence, tag)` acquire, completed tag, last consumed wake pair,
current-epoch certificate, and terminal result. Federate-wide dynamic state includes advertised
NET, grant coverage, current round tag and observation epoch, Stop state, and first failure. This
aggregate state is the single mutable authority for advertised NET, grant coverage, round state,
and participant supersession.

Drive it with explicit inputs and returned actions rather than callbacks. Inputs identify every
participant, tag, publication sequence, consumed wake pair, and acquire/completion request id.
Actions likewise carry exact identities, for example `RequestNet { tag }`,
`WakeParticipant { participant, tag, observation_epoch }`,
`ReleaseAcquire { participant, request_id, publication_sequence, tag }`, `ReportLtc { tag }`,
`SendStop`, and `FailRequest { participant, request_id, kind, reason }`; the service layer performs
those effects. Never retain channel senders or handles in the pure model.

Add table-driven tests for:

- deterministic minimum candidate across input reordering;
- `Idle` contributes no finite candidate but satisfies initial-frontier discovery;
- all-idle state emits no finite NET and remains live;
- a lower candidate replaces a higher publication before grant;
- stale or non-monotonic publication sequences are rejected while repeated `Finished` after that
  participant is terminal is idempotent and emits no additional action;
- a newer publication cancels the older acquire/reply and a late release for its request id is
  ignored;
- an RTI grant higher than the minimum opens a round only for the current minimum tag;
- cached grants cover later acquire requests without another blocking RTI wait;
- a stale pre-round `Idle` publication cannot certify quiescence;
- a wake is acknowledged only by a publication carrying the exact consumed `(tag, epoch)` pair;
- every participant must complete, advance, or publish post-round quiescence before LTC;
- if upstream A completes or advances after downstream B certified quiescence, all certificates
  are invalidated, the epoch increments, every live participant is woken again, and LTC waits for
  a complete transition-free epoch;
- local zero-delay and positive-delay producer/consumer sequences do not deadlock;
- Finished participants leave later frontier calculations;
- the final Finished participant emits one Stop and repeated terminal input emits no second Stop;
- force-stop fails every pending acquire/completion and preserves the first error.

Run and verify RED before implementing transitions:

```bash
cargo test -p boomerang_federated --all-features federate_coordination::state::tests
```

Then implement the minimum pure state machine and verify GREEN. The state module must not import
`FederateToRti`, `RtiToFederate`, `FederateProtocolClient`, `RtiGraph`, Tokio, sockets, or builder
types.

- [ ] **Step 4: Add the protocol adapter and dedicated service loop**

Keep the per-Federate protocol state machine separate from aggregation. Move
`RtiLogicalTimeCoordinator` to `client/coordination/mod.rs` and its focused tests to
`client/coordination/tests.rs`. Expose crate-private operations for idempotent NET submission,
bounded/nonblocking TAG/MSG polling, LTC, Stop, and terminal failure. It must not cache or decide
grant coverage; each returned TAG is an input to the aggregate state, which is the only grant
authority. Preserve its
existing inbound-message-before-TAG and failure-atomic tests. Keep `client/mod.rs` responsible for
transport connection, reader/writer tasks, mailbox ownership, and route records only.
The uncommitted direct trait shim from Step 2 may remain solely so the not-yet-migrated runner
compiles; the new service must not call it, and Step 6 removes it before any commit.

In `federate_coordination/service.rs`, create one dedicated service loop that owns this protocol
coordinator. It drains participant requests between bounded protocol polls and applies actions from
the pure state machine. Do not wrap the protocol coordinator in a shared mutex and do not let a
participant proxy perform protocol I/O.

Each participant proxy contains only its `EnclaveKey`, request/reply channels, and generic wake
capability. The proxy assigns monotonic publication sequences and request ids; every acquire is
bound to the latest publication sequence, and a newer publication supersedes its older pending
acquire/reply. The service enqueues the exact `CoordinationWake { tag, observation_epoch }` through
the participant's `SendContext`; no shared mutable "latest round" value exists. The runtime trait
and scheduler do not interpret Federate ids or RTI messages.

Fixed-point observation is mandatory: once any current-epoch certificate exists, every accepted
participant completion, frontier advance, or candidate revision invalidates every certificate,
increments the epoch, and emits a fresh wake action for every live participant. The service has no
local dependency graph and must not decide that one of these transitions is harmless. Add a
separate pure-state test for each of the three invalidator classes. LTC
is emitted only after all live participants certify the same exact epoch and no invalidating
transition follows. All-idle live participants remain serviceable and must never cause
`NET(FOREVER)`.

Add focused service tests with a fake protocol driver:

- participant publications are drained while a TAG is withheld;
- inbound MSG interrupts protocol polling and reaches the correct Enclave before grant release;
- independent participant requests covered by one cached grant are released without duplicate
  wire NET;
- candidate regression produces the correct revised Federate NET without regressing a granted
  tag;
- post-round quiescence, not a stale publication, triggers LTC;
- deterministic local `A -> B` ordering where B certifies before A releases work invalidates B's
  certificate, re-wakes both participants, and withholds LTC until the next fixed point;
- an unrelated async event between wake send and wake consumption does not acknowledge the epoch;
- a superseded acquire never receives a stale release;
- protocol error and force-stop fail all waiting participants without deadlock;
- normal terminal completion sends exactly one Stop.

Run:

```bash
cargo test -p boomerang_federated --all-features federate_coordination::tests
cargo test -p boomerang_federated --all-features client::coordination::tests -- --list
cargo test -p boomerang_federated --all-features client::coordination::tests
```

Inspect the list output and require a nonzero test count before treating the focused client suite
as evidence.

- [ ] **Step 5: Verify the isolated service without committing the incomplete migration**

```bash
cargo test -p boomerang_runtime sched::tests
cargo test -p boomerang_runtime sched::barrier::tests
cargo test -p boomerang_federated --all-features federate_coordination::state::tests
cargo test -p boomerang_federated --all-features federate_coordination::tests
cargo test -p boomerang_federated --all-features client::coordination::tests
```

Expected: focused slices are GREEN. Do not commit yet. The old runner may still use the explicitly
temporary direct-coordinator shim from Step 2, which is forbidden in the final structure and must
be removed during Step 6.

- [ ] **Step 6: Integrate services in the static runner with failing end-to-end tests**

Keep `static_runner.rs` limited to resource construction and supervision. At runtime, after
`RuntimeFederate` has been consumed, derive the immutable sorted participant layout from its owned
Enclave map, create one service from its Federate-local protocol coordinator, and give every owned
Enclave a proxy. Delete `federate_has_no_initial_work` and all runtime pruning; startup work is not
a membership criterion. Remove the temporary direct `LogicalTimeCoordinator` implementation from
the protocol coordinator so only participant proxies implement the federated scheduler boundary.
Do not persist this layout or any service state back into lowering artifacts.

Retain the existing RED tests:

- `all_enclaves_participate_in_federate_rti_frontier`;
- `scheduler_panic_stops_waiting_peers`.

Add bounded integration tests for:

- same-Federate `A -> B` zero-delay dependency;
- same-Federate positive-delay dependency;
- an initially idle downstream Enclave woken by local work after a Federate grant;
- an Enclave with no startup work retained as an upstream local-barrier participant and woken by
  later local work;
- inbound federated MSG observed before later-tag work;
- an earlier local or federated interruption revising a published candidate;
- all-idle participants remaining live without a finite NET;
- aggregated LTC waiting for post-round quiescence from every participant;
- deterministic upstream-release-after-downstream-certificate invalidation and re-wake;
- exact NET/TAG/LTC/MSG/Stop sequences and frame counts, including no `NET(FOREVER)` while every
  participant is merely idle and live;
- exactly one Stop after normal completion.

The public `federated_static` in-memory test with local cross-Enclave dependencies is a mandatory
deadlock regression. It must run behind a test-only bound, but the runtime must not gain a timeout.

Run:

```bash
cargo test -p boomerang_federated --all-features static_runner::tests::all_enclaves_participate_in_federate_rti_frontier
cargo test -p boomerang_federated --all-features static_runner::tests::same_federate_local_dependency_does_not_deadlock
cargo test -p boomerang_federated --all-features static_runner
cargo test -p boomerang --all-features --test federated_static public_api_runs_static_in_memory_federation
```

Expected: every active Enclave is protected by the Federate grant, local dependencies do not
deadlock, and no global graph is visible to the runtime Federate or its schedulers.

- [ ] **Step 7: Supervise completion before ordered joins**

Have every scheduler thread report its terminal result, including a caught panic payload, over a
completion channel. The runner waits for completion reports rather than blocking on handles in
spawn order. On the first panic or scheduler error, force-stop every Federate coordination
service and abort or close the RTI session before joining remaining scheduler threads. Preserve
the first terminal error as the returned error; cleanup failures remain secondary diagnostics.

On normal completion, scheduler shutdown is the sole authoritative `Finished` publisher; the
runner must not also publish `Finished`. Repeats are idempotent. The service sends exactly one Stop
after the last participant finishes; only then does the runner join services, join scheduler
threads, and await the RTI session. Panic/error does not synthesize per-participant Finished; it
uses global force-stop.

Run the bounded failure test:

```bash
cargo test -p boomerang_federated --all-features static_runner::tests::scheduler_panic_stops_waiting_peers
```

Expected: it returns `SchedulerThreadPanic` within the test bound and never relies on join order or
a runtime timeout.

- [ ] **Step 8: Compile-check and commit the complete Task 5 migration atomically**

```bash
cargo check --workspace --all-features
git add boomerang_runtime/src/event.rs boomerang_runtime/src/sched/barrier.rs boomerang_runtime/src/sched/mod.rs boomerang_federated/src/client boomerang_federated/src/federate_coordination boomerang_federated/src/lib.rs boomerang_federated/src/static_runner.rs boomerang/tests/federated_static.rs
git commit -m "fix(federated): coordinate all federate enclaves"
```

- [ ] **Step 9: Audit phase and data boundaries before Task 6**

Run Graft structural inspection first, then exhaustive concept searches and inspect the results:

```bash
graft build
graft skeleton boomerang_runtime/src/event.rs
graft skeleton boomerang_runtime/src/sched/barrier.rs
graft skeleton boomerang_federated/src/federate_coordination/state.rs
graft skeleton boomerang_federated/src/federate_coordination/service.rs
graft skeleton boomerang_federated/src/client/coordination/mod.rs
graft skeleton boomerang_federated/src/static_runner.rs
graft grep "FederateToRti"
graft grep "RtiGraph"
graft grep "cached_grant"
graft grep "observation_epoch"
graft grep "last_granted"
graft grep "grant_coverage"
graft grep "advertised"
graft grep "pending_net"
graft grep "request_net"
rg -n -i "federate|endpoint|rti|protocol|transport|tokio|graph" boomerang_runtime
rg -n -i "participant|frontier|publication|request_id|grant|round|epoch|certificate|channel|service" \
  boomerang_builder/src boomerang_federated/src/hierarchy.rs
rg -n -i "participantstate|coordinationstate|frontier|publication|grant|round|epoch|certificate|poll_for_tag|run_federate_coordination" \
  boomerang_federated/src/static_runner.rs
rg -n -i "last_granted|grant_coverage|advertised|pending.*net|request_net" \
  boomerang_federated/src/client/coordination/mod.rs \
  boomerang_federated/src/federate_coordination/state.rs \
  boomerang_federated/src/federate_coordination/service.rs
```

Expected:

- the runtime search has no federated/protocol/transport knowledge;
- builder, lowering, and runtime hierarchy contain no mutable coordination state;
- static runner contains construction/supervision calls only, not state-machine or protocol-loop
  implementations.
- `federate_coordination/state.rs` alone decides advertised NET and grant coverage;
  `client/coordination/mod.rs` retains only wire send/poll validation and idempotence bookkeeping,
  while `service.rs` executes state actions without becoming a second authority.

Then run:

```bash
cargo test -p boomerang_runtime
cargo test -p boomerang_federated --all-features
cargo test -p boomerang --all-features --test federated_static
cargo clippy -p boomerang_runtime --all-targets -- -D warnings
cargo clippy -p boomerang_federated --all-targets --all-features -- -D warnings
```

If the known delayed-equivalence test hangs, run it separately with a bound, report it explicitly,
and do not treat skipped or filtered output as a pass. All new split-phase and public static
federation tests must pass with fresh output.

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
cargo check --workspace --all-features
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
cargo check --workspace --all-features
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
[ ] Frontier publication precedes local blocking while local acquire still precedes blocking
    external grant acquisition.
[ ] Stale pre-round Idle/candidate publications cannot satisfy post-round LTC quiescence.
[ ] Builder/lowering/runtime hierarchy contain no mutable participant or service state.
[ ] static_runner constructs and supervises coordination but contains no frontier state machine or
    RTI polling loop.
[ ] boomerang_runtime contains generic coordination only, with no Federate, RTI, transport, Tokio,
    endpoint, or graph knowledge.
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

2026-08-14: Replaced Task 5's blocking all-participant rendezvous after it deadlocked on a
same-Federate local dependency. The revised task separates nonblocking frontier publication from
blocking grant acquisition, isolates pure dynamic aggregation state from protocol I/O and runner
orchestration, and freezes build/lowering artifacts during the runtime-only correction. Versioned
local rounds prevent stale Idle state from causing premature LTC while preserving TAG/NET/LTC/MSG
and Stop wire semantics.
