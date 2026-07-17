# Runtime Enclave Ownership Simplification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `RuntimeEnclaves` with a dense `TinyMap`, make `RuntimeFederation` the single owner of federated Enclaves, and move Enclave runtime types into `boomerang_runtime::enclaves`.

**Architecture:** Assembly lowering allocates one dense `TinyMap<EnclaveKey, Enclave>` and transfers it unchanged to either local execution or `RuntimeFederation`. Federates retain only their Enclave keys and protocol bridges. Replay handlers move onto their target Enclave so no wrapper or parallel owner is needed.

**Tech Stack:** Rust, `tinymap`, Cargo feature configurations, existing builder/federated/runtime tests.

---

### Task 1: Extract the public Enclave module

**Files:**
- Create: `boomerang_runtime/src/enclaves.rs`
- Create: `boomerang_runtime/tests/enclaves_module.rs`
- Modify: `boomerang_runtime/src/env/mod.rs:1-715`
- Modify: `boomerang_runtime/src/lib.rs:8-50`
- Modify: `boomerang_runtime/src/sched/mod.rs:18`

- [ ] **Step 1: Add a public-module smoke test**

```rust
use boomerang_runtime::enclaves::{Enclave, EnclaveKey};

#[test]
fn enclave_types_are_available_from_their_own_module() {
    let enclaves = [Enclave::default()]
        .into_iter()
        .collect::<tinymap::TinyMap<EnclaveKey, Enclave>>();
    assert_eq!(enclaves.len(), 1);
}
```

- [ ] **Step 2: Verify the test initially fails**

Run: `cargo test -p boomerang_runtime --test enclaves_module`
Expected: compilation fails because `boomerang_runtime::enclaves` does not exist.

- [ ] **Step 3: Move the Enclave ownership types**

Create `enclaves.rs` containing `EnclaveKey`, `UpstreamRef`, `DownstreamRef`, `Enclave`, their
implementations, and `crosslink_enclaves`. Import `Env` and `ReactionGraph` from `crate::env` and
retain all existing behavior. Declare every struct and field with at least one line of rustdoc.
Expose the module and retain root-level convenience exports:

```rust
pub mod enclaves;
mod env;

pub use enclaves::{
    crosslink_enclaves, DownstreamRef, Enclave, EnclaveKey, UpstreamRef,
};
pub use env::{
    BankInfo, Env, Level, LevelReactionKey, LifecycleReaction, ModalScheduleIndex, Mode,
    ModeFilter, ModeKey, ReactionGraph, ScopeInfo, ScopeKey, TransitionKind,
};
```

Update scheduler imports from `env::{Enclave, EnclaveKey}` to `enclaves::{Enclave, EnclaveKey}`.

- [ ] **Step 4: Run the focused module test**

Run: `cargo test -p boomerang_runtime --test enclaves_module`
Expected: PASS.

- [ ] **Step 5: Commit the module extraction**

```bash
git add boomerang_runtime/src/enclaves.rs boomerang_runtime/src/env/mod.rs \
  boomerang_runtime/src/lib.rs boomerang_runtime/src/sched/mod.rs \
  boomerang_runtime/tests/enclaves_module.rs
git commit -m "refactor(runtime): extract enclave module"
```

### Task 2: Restore direct dense-map ownership

**Files:**
- Modify: `boomerang_runtime/src/enclaves.rs`
- Modify: `boomerang_runtime/src/replay.rs:190-275`
- Delete: `boomerang_runtime/src/runtime_enclaves.rs`
- Modify: `boomerang_runtime/src/lib.rs:15-50`
- Modify: `boomerang_builder/src/assembly/build.rs:48-220,1058-1072`
- Modify: `boomerang_util/src/runner.rs:185-215`
- Modify: local-runtime consumers found by `rg -n "RuntimeEnclaves" --glob '*.rs'`

- [ ] **Step 1: Add a dense-key regression assertion**

Extend `boomerang_runtime/tests/enclaves_module.rs`:

```rust
#[test]
fn enclave_owner_allocates_dense_keys() {
    let mut enclaves = tinymap::TinyMap::<EnclaveKey, Enclave>::new();
    let first = enclaves.insert(Enclave::default());
    let second = enclaves.insert(Enclave::default());
    assert_eq!(tinymap::Key::index(&first), 0);
    assert_eq!(tinymap::Key::index(&second), 1);
}
```

- [ ] **Step 2: Run the dense-key test before replacement**

Run: `cargo test -p boomerang_runtime --test enclaves_module enclave_owner_allocates_dense_keys`
Expected: PASS, documenting the `TinyMap` invariant that the refactor must preserve.

- [ ] **Step 3: Replace `RuntimeEnclaves` in lowering and local execution**

Use the raw owner type everywhere:

```rust
tinymap::TinyMap<runtime::EnclaveKey, runtime::Enclave>
```

Change `RuntimeExecution::Local`, `RuntimeAssemblyContext::enclaves`, default construction,
`local_enclaves`, and `into_local` to this type. Replace `RuntimeEnclaves::new()` with
`tinymap::TinyMap::new()` and delete the `runtime_enclaves` module and exports.

- [ ] **Step 4: Move replay handlers onto their target Enclave**

Under `#[cfg(feature = "replay")]`, add a documented Enclave field and accessors:

```rust
/// Replay handlers keyed by their target runtime action.
replayers: tinymap::TinySecondaryMap<crate::ActionKey, Box<dyn crate::replay::ReplayFn>>,

#[doc(hidden)]
pub fn replayers_mut(
    &mut self,
) -> &mut tinymap::TinySecondaryMap<crate::ActionKey, Box<dyn crate::replay::ReplayFn>> {
    &mut self.replayers
}

pub fn take_replayers(
    &mut self,
) -> tinymap::TinySecondaryMap<crate::ActionKey, Box<dyn crate::replay::ReplayFn>> {
    std::mem::take(&mut self.replayers)
}
```

Initialize the field in both Enclave constructors. Change builder replay lowering to
`runtime_assembly.enclaves[enclave_key].replayers_mut().insert(action_key, replayer)`.
In the utility runner, collect `(EnclaveKey, action_replayers)` from the dense map before calling
`create_replayer`; keep `create_replayer` accepting `ReplayersMap` and a borrowed dense `TinyMap`.

- [ ] **Step 5: Verify runtime and replay configurations**

Run: `cargo test -p boomerang_runtime --all-features`
Expected: all tests PASS.

Run: `cargo check -p boomerang_util --features runner,replay`
Expected: exit 0.

- [ ] **Step 6: Commit dense local ownership**

```bash
git add boomerang_runtime boomerang_builder/src/assembly/build.rs boomerang_util/src/runner.rs
git commit -m "refactor(runtime): restore dense enclave ownership"
```

### Task 3: Make `RuntimeFederation` own the dense map

**Files:**
- Modify: `boomerang_federated/src/hierarchy.rs:1-130`
- Modify: `boomerang_builder/src/tests/federated.rs:1390-1490`
- Modify: `boomerang/tests/federated_static.rs:170-215`

- [ ] **Step 1: Update the public ownership test first**

Replace the independent-Federate assertion with a federation-owner assertion:

```rust
let (topology, enclaves, federates) = federation.into_parts();
assert_eq!(topology.topology().edges.len(), 1);
assert_eq!(enclaves.len(), 3);
assert_eq!(federates[&FederateId::new("a")].enclave_keys().len(), 2);
assert_eq!(federates[&FederateId::new("b")].enclave_keys().len(), 1);
```

- [ ] **Step 2: Verify the ownership test fails against the old API**

Run: `cargo test -p boomerang --features federated public_api_owns_dense_runtime_enclave_map`
Expected: compilation fails because `into_parts` returns two elements and `enclave_keys` is absent.

- [ ] **Step 3: Change the hierarchy representation**

Use these ownership fields:

```rust
pub struct RuntimeFederate {
    /// Protocol identity for this Federate.
    id: FederateId,
    /// Dense-map keys of the Enclaves assigned to this Federate.
    enclave_keys: Vec<boomerang_runtime::EnclaveKey>,
    /// Protocol bridge serving this Federate's Enclaves.
    bridge: FederateRuntimeBridge,
}

pub struct RuntimeFederation {
    /// Immutable topology used to start the RTI.
    topology: CompiledTopology,
    /// Dense owner of every runtime Enclave in the Federation.
    enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    /// Federate metadata and protocol bridges keyed by protocol identity.
    federates: BTreeMap<FederateId, RuntimeFederate>,
}
```

Expose `RuntimeFederate::enclave_keys`, `RuntimeFederation::enclaves`, and an `into_parts` returning
`(CompiledTopology, TinyMap<EnclaveKey, Enclave>, BTreeMap<FederateId, RuntimeFederate>)`.

- [ ] **Step 4: Fold placement validation into `RuntimeFederationError`**

Build a temporary `TinySecondaryMap<EnclaveKey, FederateId>` secondary ownership index in
`from_lowered`. Validate referenced keys before insertion, reject duplicate owners, then reject
unassigned non-empty Enclaves. Add documented variants directly to `RuntimeFederationError`:

```rust
#[error("Enclave {0:?} is assigned to more than one Federate")]
DuplicateEnclaveOwner(boomerang_runtime::EnclaveKey),
#[error("Enclave {0:?} has no owning Federate")]
MissingEnclaveOwner(boomerang_runtime::EnclaveKey),
#[error("Federate placement references unknown Enclave {0:?}")]
UnknownEnclave(boomerang_runtime::EnclaveKey),
```

Remove the transparent `RuntimeEnclavesError` variant.

- [ ] **Step 5: Run hierarchy and builder federation tests**

Run: `cargo test -p boomerang_builder --all-features federated`
Expected: all selected tests PASS.

Run: `cargo test -p boomerang --features federated public_api_owns_dense_runtime_enclave_map`
Expected: PASS.

- [ ] **Step 6: Commit federation ownership**

```bash
git add boomerang_federated/src/hierarchy.rs boomerang_builder/src/tests/federated.rs \
  boomerang/tests/federated_static.rs
git commit -m "refactor(federated): centralize enclave ownership"
```

### Task 4: Remove static-runner split/merge behavior

**Files:**
- Modify: `boomerang_federated/src/static_runner.rs:250-420,900-1050`
- Modify: any hierarchy consumers found by `rg -n "RuntimeFederate|into_parts\(\)" boomerang_federated boomerang_builder boomerang --glob '*.rs'`

- [ ] **Step 1: Add a static-runner ownership regression assertion**

In the existing static-runner preparation test, assert that preparation retains every original
key rather than reinserting Enclaves:

```rust
let expected_keys = runtime.enclaves().keys().collect::<Vec<_>>();
let (prepared, _) = prepare_static_federation(runtime).unwrap();
assert_eq!(prepared.enclaves.keys().collect::<Vec<_>>(), expected_keys);
```

- [ ] **Step 2: Run the focused test before changing preparation**

Run: `cargo test -p boomerang_federated --all-features static_runner`
Expected: compilation fails until test setup and the three-part `into_parts` API are adopted.

- [ ] **Step 3: Consume the dense map directly**

Change preparation to begin with:

```rust
let (topology, enclaves, federates) = runtime.into_parts();
let mut by_federate = BTreeMap::new();
let mut connections = BTreeMap::new();
for (map_id, federate) in federates {
    let (id, enclave_keys, bridge) = federate.into_parts();
    // retain the existing map-key/id validation
    by_federate.insert(id.clone(), enclave_keys);
    connections.insert(id, bridge);
}
```

Delete `RuntimeEnclaves::new`, `insert_at`, and per-Federate map merging. Update tests and builder
consumers to select Enclaves from the one dense owner using each Federate's key list.

- [ ] **Step 4: Run federated crate tests**

Run: `cargo test -p boomerang_federated --all-features`
Expected: all tests PASS.

Run: `cargo test -p boomerang_builder --all-features`
Expected: all tests PASS.

- [ ] **Step 5: Commit static-runner simplification**

```bash
git add boomerang_federated/src/static_runner.rs boomerang_builder boomerang/tests
git commit -m "refactor(federated): consume dense enclave map"
```

### Task 5: Align documentation and run full verification

**Files:**
- Modify: `boomerang_builder/src/assembly/mod.rs:1-15`
- Modify: `docs/federated-runtime.md:40-75`
- Modify: other references found by `rg -n "RuntimeEnclaves|independently owned.*Enclave|owned scheduler set" --glob '*.rs' --glob '*.md'`

- [ ] **Step 1: Update ownership documentation**

Describe `RuntimeFederation` as the dense Enclave owner and `RuntimeFederate` as placement plus
bridge metadata. Remove claims that each Federate owns an independently movable Enclave map.
Retain the distinction that each Enclave still has an independent scheduler.

- [ ] **Step 2: Confirm obsolete names and claims are gone**

Run: `rg -n "RuntimeEnclaves|RuntimeEnclavesError|independently owned.*Enclave|owned scheduler set" --glob '*.rs' --glob '*.md'`
Expected: no obsolete source or documentation matches outside historical design/plan documents.

- [ ] **Step 3: Format and check the complete workspace**

Run: `cargo fmt --all -- --check`
Expected: exit 0.

Run: `cargo check --workspace --all-targets --all-features`
Expected: exit 0.

- [ ] **Step 4: Run the complete workspace test suite**

Run: `cargo test --workspace --all-features`
Expected: all tests PASS, with only explicitly ignored integration tests skipped.

- [ ] **Step 5: Validate documentation and the final diff**

Run: `mdbook build book -d /tmp/boomerang-mdbook-enclave-ownership`
Expected: exit 0.

Run: `git diff --check`
Expected: exit 0.

Run: `git diff --stat HEAD~4..HEAD`
Expected: the implementation removes `runtime_enclaves.rs` and has a net-negative source diff.

- [ ] **Step 6: Commit documentation cleanup**

```bash
git add boomerang_builder/src/assembly/mod.rs docs/federated-runtime.md
git commit -m "docs: clarify federated enclave ownership"
```
