# Federated RTI Phase Boundaries Design

## Goal

Make assembly lowering the only path that constructs federated RTI artifacts. The resulting
runtime data must enforce three boundaries structurally:

- the RTI owns one immutable global coordination graph;
- each Federate owns only its local runtime and protocol-client data;
- mutable RTI coordination state is dense and separate from immutable graph data.

Public API compatibility is not a constraint. Removed APIs will be plumbed through all callers
without compatibility constructors or parallel legacy paths.

## Scope

This change covers static federation construction, RTI graph representation, session startup,
client startup, topology validation, and the builder-to-runtime handoff. It retains the existing
TAG, NET, LTC, MSG, and Stop behavior.

Federation partners are trusted. `Hello` establishes a declared Federate identity but does not
send, hash, or authenticate topology. Authentication and topology hashing are explicitly deferred.

Standalone serialized topology manifests are not supported. `Assembly` lowering is the only
producer of an RTI graph.

## Architecture

`Assembly::into_runtime_assembly` produces the complete federation deployment:

```text
RuntimeFederation
├── rti: RtiGraph
└── federates: BTreeMap<FederateId, RuntimeFederate>
    ├── id: FederateId
    ├── bridge: FederateRuntimeBridge
    └── enclaves: TinyMap<EnclaveKey, Enclave>
```

`RtiGraph` is immutable after lowering. It contains only data used by RTI coordination and route
validation:

- stable Federate and endpoint identities interned into dense keys;
- direct incoming dependencies;
- precomputed minimum-delay transitive incoming paths;
- precomputed downstream reevaluation sets;
- endpoint source, target, and delay records;
- stable-ID lookup maps required at session and protocol boundaries.

It does not retain a source topology manifest, client neighbor structures, or intermediate
all-pairs maps that runtime decisions do not query.

`RtiRuntimeState` contains only a `TinySecondaryMap<FederateKey, FederateCoordination>`. The RTI
engine combines an owned `RtiGraph` with this mutable state. The graph is moved into the RTI
session and is neither deeply cloned nor exposed to Federate clients.

Each `RuntimeFederate` remains the independently deployable client artifact. Its identity,
Enclaves, and `FederateRuntimeBridge` contain all information required to start that Federate.
There is no separate client-topology wrapper because topology is no longer exchanged during
startup and the bridge already owns the Federate's routes, mailbox, and fault state.

## Assembly Graph Analysis

`PartitionAnalysis` becomes the source of truth for the federated dependency graph. Cross-partition
connection analysis records the Federate ownership, endpoint identity, direction, and delay needed
to build a reusable graph. Assembly lowering uses this graph for:

- duplicate Federate and endpoint validation;
- zero-delay cycle detection;
- deterministic dense-key ordering;
- direct incoming dependency construction;
- transitive reachability and downstream affected sets;
- minimum accumulated path delays;
- RTI endpoint projection;
- Federate-local bridge and route projection.

The builder's existing `petgraph` dependency supplies cycle, traversal, and shortest-path
operations. `boomerang_federated::rti` does not rebuild or analyze a declarative graph at runtime.
Parallel protocol endpoints remain distinct routes even when they connect the same Federate pair.

All derived collections use deterministic stable-ID ordering before dense keys are allocated.

## Lowering and Runtime Data Flow

Federated lowering emits final graph and per-Federate artifacts alongside transient connection
boundary metadata:

```text
Assembly declarations
    -> PartitionAnalysis and federate dependency graph
    -> validation and graph algorithms
    -> RtiGraph + Federate-local bridges/routes
    -> RuntimeFederation
    -> consume into one RTI session and independent Federate clients
```

The static runner consumes `RuntimeFederation` once. It creates transports by iterating the owned
Federate map, moves `RtiGraph` into the RTI session, and moves each `RuntimeFederate` into its
client and schedulers. Client connection code never queries the RTI graph.

The following source-manifest and compatibility APIs are removed:

- `FederatedTopology`;
- `TopologyEdge`;
- `NeighborStructure`;
- public `CompiledTopology` construction;
- `RtiState::new` from a raw topology;
- session and TCP-runner entry points that accept raw topology.

The final immutable type is named `RtiGraph`; `CompiledTopology` is removed rather than retained
as an alias.

## Protocol Startup

Trusted startup uses:

```rust
FederateToRti::Hello {
    federate_id: FederateId,
}
```

The session binds each accepted transport to a Federate present in `RtiGraph`. It checks that the
first frame is `Hello`, rejects unknown or duplicate identities, and thereafter passes the cached
dense `FederateKey` to the RTI engine. It does not compare client-supplied topology.

Stable `FederateId` and `EndpointId` values remain on the wire and in diagnostics. Dense keys
remain private and process-local.

## Error Boundaries

Static graph failures are assembly failures. Duplicate IDs, duplicate or conflicting endpoints,
undeclared members, zero-delay cycles, unsupported placement, and minimum-delay overflow are
reported as focused `AssemblyError` variants before runtime artifacts exist.

`RtiError` contains only live state-machine and protocol failures: unknown participants, identity
mismatches, invalid routes or tags, invalid lifecycle transitions, regressions, and runtime tag
arithmetic failures.

`RuntimeFederation` construction consumes complete lowering artifacts. It does not perform a
second topology-validation pass. Mismatches between keyed lowering collections are builder defects
and are represented by internal assertions rather than user-facing runtime topology errors.

Static runner errors cover runner configuration, transport, task, scheduler, and resource setup
failures. They do not report malformed assembly topology.

## Testing

The migration follows test-driven development in small slices.

RTI behavioral tests continue to cover TAG, NET, LTC, MSG, Stop, in-transit messages, delayed
paths, cycles with positive delay, fanout, and deterministic reevaluation. Test fixtures construct
final graph parts through crate-private test helpers; no public raw-topology compiler is retained.

Topology responsibility moves to builder tests. They cover duplicate identities, endpoint
conflicts, zero-delay cycles, deterministic ordering, direct dependencies, reachability,
minimum-delay competing paths, disconnected Federates, positive-delay cycles, and path-delay
overflow.

Session and client tests prove that `Hello` carries only identity and that session participants are
resolved once into dense keys. Structural runner tests prove that each `RuntimeFederate` starts
without access to `RtiGraph` and that RTI startup moves rather than clones the graph.

Existing in-memory and TCP end-to-end federation tests remain the final behavioral proof that
assembly lowering produces complete artifacts.

Verification includes focused crate tests, full workspace tests with relevant features, formatting,
and Clippy with warnings denied.

## Non-Goals

- backward-compatible topology constructors or aliases;
- standalone manifest-driven RTI construction;
- client topology exchange;
- authentication, authorization, or topology hashing;
- dynamic Federation membership;
- changes to logical-time grant semantics.
