# Runtime Enclave Ownership Simplification

## Context

`RuntimeEnclaves` wraps a sparse `TinySecondaryMap`, forwards most of its collection API, and
adds a `split_by` operation used only while constructing a `RuntimeFederation`. This replaced the
dense `TinyMap` that originally owned lowered Enclaves, introduced a separate error type, and
forces the static runner to merge independently split maps before execution.

The Enclave runtime types also remain embedded in `env`, even though an Enclave is the scheduler
ownership boundary rather than part of an individual scheduler's resolved `Env` data.

## Design

### Enclave module

Create a public top-level `boomerang_runtime::enclaves` module, following the existing `reactor`
source-module organization. Move `Enclave`, `EnclaveKey`, `UpstreamRef`, `DownstreamRef`, and
`crosslink_enclaves` from `env` into it. Continue re-exporting the public types at the crate root
so existing internal call sites can use the concise `boomerang_runtime::Enclave` form.

All moved structs and their fields retain or gain concise rustdoc comments.

### Dense ownership

Delete `RuntimeEnclaves` and `RuntimeEnclavesError`. Use
`TinyMap<EnclaveKey, Enclave>` directly throughout assembly lowering and local execution. The map
that allocates an `EnclaveKey` remains the sole owner of the corresponding Enclave, preserving the
dense key invariant.

For replay builds, store each Enclave's action replayers on that Enclave. This removes the need
for a parallel per-Enclave collection in the deleted wrapper while keeping replay state with the
runtime object it targets.

### Federated hierarchy

`RuntimeFederation` owns the single dense Enclave map. `RuntimeFederate` owns its protocol bridge
and the ordered `Vec<EnclaveKey>` identifying its Enclaves; it does not own a sparse copy of those
Enclaves. Federation accessors resolve Federate placement against the owning map.

`RuntimeFederation::from_lowered` validates the placement directly. Duplicate ownership, missing
ownership for a non-empty Enclave, and references to unknown Enclaves become variants of the
existing `RuntimeFederationError`; there is no independent collection error.

Consuming the federation returns the topology, dense Enclave map, and Federate metadata. The
static runner consumes these parts directly instead of rebuilding a combined collection from
per-Federate sparse maps.

## Compatibility and scope

Public API compatibility is not required. Callers and tests that currently consume
`RuntimeEnclaves` or expect each `RuntimeFederate` to return an owned sparse collection will be
updated to the dense federation-owned representation. Protocol behavior, Enclave keys, placement,
logical-time coordination, and endpoint routing remain unchanged.

No changes are planned to maps that are genuinely secondary indexes, such as upstream/downstream
Enclave links or scheduler result maps.

## Verification

Run formatting and workspace checks, then exercise the runtime, builder, federated, and combined
federated/replay feature configurations. Existing federation hierarchy, static runner, scheduler,
and replay tests will be updated where their ownership assertions change. Focused tests will
verify placement validation and preservation of the original Enclave keys.
