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
client startup, topology validation, federate-wide scheduler coordination, runner failure
supervision, and the builder-to-runtime handoff. It retains the existing TAG, NET, LTC, MSG, and
Stop protocol and RTI state-machine behavior.

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

## Federate-Wide Logical-Time Coordination

One protocol client and one `RtiLogicalTimeCoordinator` belong to each Federate, but every active
Enclave scheduler owned by that Federate participates in the Federate's logical-time frontier. An
Enclave is a separately scheduled runtime partition. The frontier is the lowest tag to which the
Federate can safely advance after accounting for all of its active Enclaves and their in-process
dependencies.

The current heuristic of attaching the RTI coordinator to one selected "gateway" Enclave is not
part of the target architecture. A non-gateway Enclave can otherwise advance without an RTI grant
and can outrun an inbound federated message or the Federate's Stop ordering. Selecting an Enclave
because it has an upstream local dependency does not establish a correctness relationship between
that scheduler and every other scheduler in the Federate.

The static runner therefore creates one Federate coordination service and gives each active
Enclave a participant proxy implementing `boomerang_runtime::LogicalTimeCoordinator`. The service
owns the single protocol coordinator and serializes protocol access. Coordination is split into a
nonblocking frontier-publication phase and a later blocking grant-acquisition phase. This mirrors
the federated protocol distinction between a Federate's NET promise and its permission to execute
after TAG.

Before an Enclave can block on an in-process upstream barrier, its scheduler publishes one of
three versioned frontier states through the generic coordinator interface:

- `Candidate(tag)`: the lowest currently queued event tag;
- `Idle`: no finite event is currently queued, but future local or federated input remains
  possible;
- `Finished`: the scheduler is terminal and no longer participates.

Publication never waits for the RTI. A candidate revision caused by an earlier local or federated
event replaces the previous publication before the scheduler attempts that tag again. Once every
active participant has published a current state, the service sends one Federate NET for the
minimum finite candidate. If all participants are idle, it sends no finite NET and continues
servicing participant and protocol input.

After publishing, a scheduler retains the established execution order: acquire in-process
barriers, acquire the external Federate permit, process the tag, release in-process downstream
barriers, and report completion. The service caches RTI grants and releases only participant
requests covered by the grant. Consequently a downstream Enclave can publish its candidate before
waiting for an upstream Enclave, while no reaction executes without both local and federated
permission.

An `Idle` publication is not evidence that the participant has completed every future tag: a
same-Federate upstream Enclave may still enqueue work at that tag. Whenever the cached RTI grant
covers the Federate's current minimum candidate `t`, the service therefore opens a versioned local
round for `t`; an RTI grant higher than `t` does not implicitly complete the higher tag. Every
active participant is woken, if necessary, to observe the round and must either process all newly
visible work through `t` or publish a post-round quiescence certificate. The service sends LTC for
`t` only when every active participant has completed `t`, advanced beyond it, or certified
quiescence for that round after local downstream releases are visible. Stale `Idle` or candidate
publications from an earlier round never satisfy completion. This certification is local control
traffic and does not add a wire-protocol message.

Participant completion removes that Enclave from future frontier calculations. The last
participant causes exactly one Stop. The failure path force-stops the service without waiting for
frontier or quiescence consensus.

This service lives in `boomerang_federated`; `boomerang_runtime` retains only its generic
`LogicalTimeCoordinator` trait and remains unaware of RTI messages, transports, Tokio, Federate
identities, and endpoint identities. Its generic contract gains frontier publication and
grant-version/quiescence concepts, but no federated types. The service must not hold a mutex across
a blocking RTI wait that prevents other Enclave participants from reporting progress. A dedicated
coordination loop polls protocol progress and drains per-Enclave publication, acquisition,
completion, and shutdown channels without exposing `RtiGraph` to any client scheduler.

## Runner Failure Supervision

Scheduler failure is terminal for the whole static federation. The runner must observe scheduler
completion independently of join order. On the first scheduler error or panic it force-stops every
Federate coordination service, aborts or closes the RTI session, and only then joins the remaining
scheduler threads. This ordering unblocks peers waiting for grants and prevents a panic in one
scheduler from hanging the runner while it joins another scheduler first.

Normal shutdown follows the same ownership rule without treating success as failure: all Enclave
participants finish, each Federate coordination service sends one Stop, scheduler threads join,
and the RTI session completes. Panics are converted into `SchedulerThreadPanic`; ordinary scheduler
errors retain `SchedulerRuntime`. Cleanup errors may be retained as secondary diagnostics but must
not replace the first terminal failure.

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

Multi-Enclave runner tests prove that every active Enclave participates in the Federate frontier:
an Enclave without the former gateway heuristic cannot advance past a withheld RTI grant, and an
inbound message is observed before that Enclave advances beyond the message tag. Tests cover
independent siblings, zero-delay and positive-delay same-Federate dependencies, an initially idle
downstream Enclave, candidate regression after local or federated interruption, and stale
publication rejection across grant versions. They prove that LTC waits for post-grant quiescence
from every active Enclave and that normal completion sends exactly one Stop. A bounded failure test
injects a scheduler panic while another scheduler is waiting for a grant and proves that the
runner returns `SchedulerThreadPanic` rather than hanging. The timeout is test evidence, not a
runtime shutdown mechanism.

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
- changes to RTI TAG/NET/LTC grant semantics; projecting all Enclave schedulers into one Federate
  frontier is an in-scope correction to the runner's use of those semantics.

## Reconciliation Note

The 2026-08-13 architecture review found that the existing single-gateway scheduler heuristic can
let sibling Enclaves outrun RTI coordination and that joining scheduler threads before force-stop
can hang after a scheduler panic. This revision makes both findings required runtime invariants of
the phase-boundary refactor because the refactor already owns static-runner construction and claims
to preserve federated logical-time behavior. They are not changes to the wire protocol or the RTI
grant state machine.

The 2026-08-14 implementation review found that waiting for every participant's blocking acquire
request deadlocks when a downstream Enclave waits on a same-Federate upstream barrier: the
downstream cannot request the RTI grant until the upstream releases it, while the upstream cannot
receive a grant until the service sees the downstream request. The primary distributed-DE papers
model NET as a nonblocking Federate event-horizon promise distinct from TAG permission; they do not
define a rendezvous over independently blocking local schedulers. This revision adopts that
distinction through split-phase frontier publication and versioned post-grant quiescence. It
preserves the wire protocol and RTI state machine while adding a pre-local publication step to the
otherwise retained local-before-blocking-external acquisition sequence.
