# Federate-Local Enclave Ownership

## Context

The runtime currently allocates every Enclave in one federation-wide
`TinyMap<EnclaveKey, Enclave>`. `RuntimeFederate` records only a list of keys into that map, while
`RuntimeFederation` owns the actual runtime data. The static runner later combines the placement
metadata with the global map when it starts scheduler threads.

This representation makes `EnclaveKey` federation-global and gives each deployable Federate an
indirect dependency on the Enclaves belonging to every other Federate. It differs from the
established Reactor ownership model: each Enclave owns its own dense Reactor map, and builder
aliases pair the Enclave identity with the Enclave-local `ReactorKey`.

## Design

### Federate-local key space

`EnclaveKey` becomes local to one runtime owner. For local execution, that owner is the local
runtime's single dense Enclave map. For federated execution, each `RuntimeFederate` owns an
independent `TinyMap<EnclaveKey, Enclave>`. Consequently, two Federates may both own an
`EnclaveKey` with index zero without ambiguity.

Builder aliases identify the owner as well as the local key through a runtime Enclave reference:

```rust
pub enum RuntimeEnclaveRef {
    Local(EnclaveKey),
    #[cfg(feature = "federated")]
    Federated {
        federate: FederateId,
        enclave: EnclaveKey,
    },
}
```

Aliases for Reactors, Actions, Ports, Reactions, and Modes retain their object-local key and
replace the bare `EnclaveKey` component with `RuntimeEnclaveRef`.

### Direct allocation during lowering

Partition analysis already determines which assembly partition belongs to which Federate before
runtime Enclaves are allocated. Assembly lowering uses that information to allocate each Enclave
directly into its final owner's `TinyMap`; it never creates a federation-wide Enclave map and does
not split or rekey Enclaves during finalization.

The internal assembly context owns either:

- one dense Enclave map for local execution, or
- dense Enclave maps keyed by `FederateId` for federated execution.

All subsequent lowering resolves a `RuntimeEnclaveRef` through this context before inserting or
accessing runtime objects. Local partition boundaries are crosslinked only after confirming that
both Enclaves have the same owner. Cross-Federate boundaries are lowered to protocol routes and do
not retain direct Enclave references.

### Runtime hierarchy

`RuntimeFederate` owns its protocol identity, dense Enclave map, and protocol bridge:

```rust
pub struct RuntimeFederate {
    id: FederateId,
    enclaves: TinyMap<EnclaveKey, Enclave>,
    bridge: FederateRuntimeBridge,
}
```

`RuntimeFederation` owns only the compiled topology and the Federates participating in it:

```rust
pub struct RuntimeFederation {
    topology: CompiledTopology,
    federates: BTreeMap<FederateId, RuntimeFederate>,
}
```

Federation construction pairs each lowered Federate store with its bridge. Enclave placement is
represented by ownership rather than a parallel `Vec<EnclaveKey>` or secondary placement index.
The hierarchy validates that every topology Federate has exactly one runtime store and bridge,
that no unknown Federates were lowered, and that every Federate has at least one Enclave.
Duplicate, missing, and unknown Enclave placement errors are removed because those invalid states
cannot be represented by this structure.

### Static runner and results

The static runner consumes the `RuntimeFederation` as a map of independent `RuntimeFederate`
values. It chooses the RTI gateway from each Federate's local Enclave map and starts schedulers
using keys meaningful only inside that Federate.

Scheduler results preserve the ownership boundary:

```rust
pub type FederationEnvs = BTreeMap<
    FederateId,
    TinySecondaryMap<EnclaveKey, Env>,
>;
```

Thread results carry both `FederateId` and `EnclaveKey`. The runner never flattens Federate-local
keys into a federation-wide Enclave map.

## Error handling

Lowering reports an internal consistency error if a local boundary resolves to Enclaves with
different owners or if an alias references an absent owner or Enclave. Federation finalization
continues to report missing runtimes, missing bridges, unknown topology Federates, and unknown
lowered Federates. It additionally rejects a topology Federate with an empty Enclave store because
the static runner requires a gateway Enclave.

The existing partition analysis remains responsible for rejecting mixed local and federated
boundaries. Protocol connection, codec, and RTI errors are unchanged.

## Compatibility and scope

Public API compatibility is not required. Runtime aliases, hierarchy accessors, `into_parts`
methods, static-runner results, tests, and documentation will be updated to reflect Federate-local
Enclave identities.

The non-federated runtime retains one dense `TinyMap<EnclaveKey, Enclave>`. Reactor, Action, Port,
Reaction, Mode, and scheduler-local key spaces remain unchanged. This work does not alter protocol
semantics, logical-time coordination, payload codecs, or topology construction.

## Verification

Focused tests will establish that:

- two Federates can independently allocate the same numeric `EnclaveKey`;
- each `RuntimeFederate` owns only its own Enclaves;
- aliases resolve through the correct Federate and Enclave;
- local cross-Enclave coordination remains confined to one Federate;
- cross-Federate communication still uses protocol routes; and
- static-runner results remain separated by `FederateId`.

During implementation, each checklist task runs only the relevant `cargo check` command. After
all implementation tasks, formatting, the full workspace test suite, and relevant feature
combinations are run once as final verification.
