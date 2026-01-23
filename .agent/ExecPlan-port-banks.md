# Add runtime-sized port banks, explicit multiport connection helpers, and allocation-free bank extraction

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This ExecPlan is governed by `.agent/PLANS.md` in the repository root and must be maintained in accordance with that file.

## Purpose / Big Picture

Boomerang currently supports multiport arrays only at compile time, which prevents dynamically building port arrays for FMI-based simulators. After this change, a user can create runtime-sized port banks, connect them safely with explicit zip/broadcast/cartesian helpers, and use them in reactions without per-trigger heap allocation for port bank extraction. A user can demonstrate this by building a reactor that creates port banks with runtime sizes, wiring them with the new helpers, and running tests that fail prior to the change and pass after. Performance-sensitive runs can be compared before and after to confirm fewer allocations in the reaction hot loop.

## Progress

- [x] (2025-02-14 15:30Z) Implement core PortBank types, runtime extraction, and strict connection helpers.
- [x] (2025-02-14 15:55Z) Add unit test for `connect_ports` length mismatch.
- [x] (2025-02-14 16:05Z) Add integration tests for port bank reactions and connection helpers.
- [x] (2025-02-14 16:40Z) Extend `#[reactor]` macro and `#[reactor_ports]` derive to support `len`-based PortBank fields.
- [x] (2025-02-14 17:10Z) Fix `#[reactor]` banked-port inference by annotating tuple types in the generated closure.
- [x] (2025-02-15 09:40Z) Replace port bank extraction allocations with allocation-free views (completed: planning, added RefsSlice/RefsSliceMut, updated bank refs + PortBank extraction, adjusted tests, validated via cargo test -p boomerang).

## Surprises & Discoveries

- Observation: `#[reactor]` banked-port path needed explicit tuple typing to avoid inference failures in reaction closures.
  Evidence: Added typed `ports: (#(...),*)` destructuring in the generated banked-port closure.

- Observation: `Refs::take` collided with `Iterator::take` method resolution in callers.
  Evidence: `refs.ports.take(len)` resolved to `std::iter::Take`, requiring UFCS (`Refs::take(&mut refs.ports, len)`).

## Decision Log

- Decision: Define runtime-sized port banks that do not store FMI variable indices and rely on user-managed mappings.
  Rationale: The user indicated VR tracking remains external to Boomerang; this keeps the runtime data model minimal.
  Date/Author: 2025-02-14 / Codex

- Decision: Make `connect_ports` strict on iterator lengths and add explicit `connect_broadcast` and `connect_cartesian` helpers.
  Rationale: Silent truncation is error-prone; explicit helpers preserve flexibility while making intent clear.
  Date/Author: 2025-02-14 / Codex

- Decision: Replace `Vec`-backed port bank extraction with allocation-free slices of runtime port pointers, validating types during extraction.
  Rationale: Reaction triggering is a hot loop; avoiding per-trigger heap allocations reduces overhead while preserving type safety.
  Date/Author: 2025-02-15 / Codex

## Outcomes & Retrospective

No outcomes yet. This section will be updated as milestones complete.

## Context and Orientation

Boomerang is a Rust workspace with a builder/macro front-end and a runtime/store backend. Multiport arrays today are handled through const-generics and arrays, with macros in `boomerang_macros/src/ports.rs` emitting `add_input_ports` and `add_output_ports` for `[T; N]` fields. Builder-side multiport support lives in `boomerang_builder/src/reactor.rs` and `boomerang_builder/src/reaction.rs`, while runtime extraction and partitioning of ports lives in `boomerang_runtime/src/refs_extract.rs` and `boomerang_runtime/src/refs.rs`. Port values and port references are defined in `boomerang_runtime/src/port/mod.rs`, and the scheduler uses `boomerang_runtime/src/store.rs` to build cached pointers for reaction execution. Port bank extraction currently allocates a new `Vec` of typed port references per reaction trigger in `boomerang_builder/src/port.rs` and stores those `Vec`s inside `InputBankRef` and `OutputBankRef`, which is the allocation this plan removes.

A “port bank” in this plan is a runtime-sized collection of ports that can be addressed by index in reactions. It is distinct from a compile-time array, and it should behave similarly from the user perspective while allowing dynamic sizing. A “broadcast connection” means one source port connects to every destination port. A “cartesian connection” means every source port connects to every destination port.

## Plan of Work

### Milestone 1: Introduce port bank types and builder APIs

The goal of this milestone is to make runtime-sized port collections possible without changing scheduler behavior. Add a new builder-side handle type that represents a port bank as a `Vec` of `TypedPortKey`. Create new builder APIs for runtime-sized input and output banks, and keep existing array-based APIs intact by delegating to the bank implementation where possible.

Edits:

- In `boomerang_builder/src/port.rs`, define a new public struct `PortBank<T, Q, A = Local>` that owns a `Vec<TypedPortKey<T, Q, A>>`. Provide `len`, `iter`, `iter_mut` (if needed), `get`, `keys`, and `into_vec` methods. Implement `Copy`/`Clone` only if the internal representation supports it; otherwise use cheap clones via `Vec<TypedPortKey<...>>` and document that `PortBank` is a lightweight handle. Provide a `contained()` method analogous to `TypedPortKey::contained` that returns a bank with `Contained` marker, and a `map_contained` helper for macro code generation.

- In `boomerang_builder/src/reactor.rs`, add:
  - `add_input_bank<T>(name: &str, len: usize) -> Result<PortBank<T, Input>, BuilderError>`
  - `add_output_bank<T>(name: &str, len: usize) -> Result<PortBank<T, Output>, BuilderError>`
  - `add_ports_bank<T, Q>(name: &str, len: usize) -> Result<PortBank<T, Q>, BuilderError>` as internal helper.
  Keep `add_input_ports`/`add_output_ports` for `[T; N]` by delegating to the bank implementation and converting to arrays for compatibility. Document that these are const-sized wrappers.

- In `boomerang_builder/src/reaction.rs`, implement `PartialReactionBuilderField` for `PortBank<T, Q, A>` by iterating over keys and recording relations, mirroring the existing array impl. This ensures that `with_trigger(bank)` and `with_effect(bank)` work the same way as arrays.

- Update `boomerang_builder/src/macro_support.rs` and `boomerang_macros/src/ports.rs` to support a runtime-sized port bank field. The macro should detect `Vec<T>` fields or a new type alias (for example, `boomerang::builder::PortBank<T, _>`) and emit `add_input_bank`/`add_output_bank` instead of array builders. For the contained ports struct emitted by the macro, create a `PortBank` with `contained()` so that child reactor ports are represented correctly.

Acceptance for Milestone 1: A reactor can build a port bank using `add_input_bank` or through a `Vec<T>` field in a `#[reactor]` macro, and the code compiles without modifying the runtime or scheduler yet.

### Milestone 2: Runtime extraction for port banks in reactions

The goal of this milestone is to allow reaction functions to receive and use port banks as typed references without losing type safety.

Edits:

- In `boomerang_runtime/src/port/mod.rs`, add `InputBankRef<'a, T>` and `OutputBankRef<'a, T>` wrappers that hold `Vec<InputRef<'a, T>>` and `Vec<OutputRef<'a, T>>` or a small custom iterator/borrowed view if possible. Provide `len`, `iter`, `get`, and `keys` for ergonomics. Keep these lightweight and avoid storing any FMI VR metadata.

- In `boomerang_runtime/src/refs_extract.rs`, implement `ReactionRefsExtract` for the new `PortBank<T, Q, A>` builder handle types by consuming the right number of ports from `ReactionRefs` and producing `InputBankRef` or `OutputBankRef`. The extraction should validate that the number of ports available matches the bank length and return a clear error on mismatch.

- In `boomerang_runtime/src/refs.rs`, add `Partition` and `PartitionMut` support for the bank reference types if needed to integrate with `partition()` usage patterns, or ensure the `ReactionRefsExtract` path covers the intended ergonomics in macro-generated reaction closures.

Acceptance for Milestone 2: A reaction can take an input bank and iterate over its ports, and this works in a test reactor using a runtime-sized bank.

### Milestone 3: Strict zip and explicit multiport connection helpers

The goal of this milestone is to make connections safe and intentional by validating `connect_ports` length and offering explicit connection patterns.

Edits:

- In `boomerang_builder/src/reactor.rs`, update `connect_ports` to require equal lengths. Introduce a new error variant in `boomerang_builder/src/lib.rs` for mismatched lengths (for example `BuilderError::PortConnectionLengthMismatch`) and return it when the iterators differ in length. Implement the check by collecting into `Vec` or by peeking and counting with a small helper; favor clarity over micro-optimization.

- Add `connect_broadcast` and `connect_cartesian` methods to `ReactorBuilderState`. `connect_broadcast` should take one source port and an iterator of targets and connect the source to each target. `connect_cartesian` should take iterators of sources and targets and connect all pairs. Keep `connect_ports` as the strict zip.

Acceptance for Milestone 3: A test that previously relied on silent truncation now fails with a clear error, and new tests demonstrate broadcast and cartesian behavior.

### Milestone 4: Tests and documentation updates

The goal of this milestone is to lock in behavior and provide guidance for users.

Edits:

- Add tests under `boomerang/tests/` to cover:
  - runtime-sized port bank creation and use in a reaction,
  - strict length mismatch in `connect_ports`,
  - broadcast and cartesian connection helpers.

- Update any relevant docs in `README.md` or `book/src/` to mention runtime-sized port banks and the new connection helpers. Provide a small example snippet using a bank and broadcast connection.

Acceptance for Milestone 4: `cargo test` passes and the new tests fail before these changes and pass after. Documentation includes a short example of runtime-sized port banks.

### Milestone 5: Allocation-free port bank extraction

The goal of this milestone is to eliminate per-trigger heap allocations for `PortBank` extraction by using slice-style views over the existing runtime pointer arrays, while preserving type checking and ergonomics. The visible behavior should remain the same for users; the only changes should be API adjustments where bank iteration previously yielded `&InputRef`/`&OutputRef`.

Edits:

- In `boomerang_runtime/src/refs.rs`, introduce lightweight view types that represent a contiguous range of the existing port pointer arrays without allocating:
  - `RefsSlice<'a, T: ?Sized>` for immutable references and `RefsSliceMut<'a, T: ?Sized>` for mutable references. Each should store a `NonNull<NonNull<T>>` pointer and a length. Provide `len`, `is_empty`, `get`, and iterators that yield `&'a T` or `&'a mut T` by advancing the pointer, similar to the existing `Refs` and `RefsMut` iterators.
  - Add `fn take(&mut self, n: usize) -> Result<RefsSlice<'a, T>, ReactionRefsError>` to `Refs` and `fn take(&mut self, n: usize) -> Result<RefsSliceMut<'a, T>, ReactionRefsError>` to `RefsMut`. These should return an error if `n` exceeds the remaining length, otherwise advance the internal pointer by `n`.

- In `boomerang_runtime/src/port/mod.rs`, rework `InputBankRef` and `OutputBankRef` to wrap `RefsSlice`/`RefsSliceMut` rather than owning a `Vec`:
  - `InputBankRef<'a, T>` should store a `RefsSlice<'a, dyn BasePort>` and a `PhantomData<T>`.
  - `OutputBankRef<'a, T>` should store a `RefsSliceMut<'a, dyn BasePort>` and a `PhantomData<T>`.
  - Update constructors to accept the new slice types, for example `InputBankRef::from_slice(slice: RefsSlice<'a, dyn BasePort>)` and `OutputBankRef::from_slice(slice: RefsSliceMut<'a, dyn BasePort>)`.
  - Adjust `iter`/`iter_mut` and `get`/`get_mut` to return `InputRef<'a, T>` or `OutputRef<'a, T>` by value rather than `&InputRef`/`&OutputRef`, because there is no backing `Vec` to borrow from. Note that this is a user-visible signature change; update tests and any example code accordingly.

- In `boomerang_builder/src/port.rs`, update each `ReactionRefsExtract` implementation for `PortBank` to:
  - call the new `take` methods on `refs.ports` or `refs.ports_mut` to obtain a slice of raw port references without allocation,
  - validate types by iterating over the slice once and calling `InputRef::try_from(DynPortRef(..))` or `OutputRef::try_from(DynPortRefMut(..))` for each entry, returning the first error encountered,
  - construct the `InputBankRef`/`OutputBankRef` from the slice and return it.

- Update any tests or example usage that relied on `InputBankRef::iter` yielding `&InputRef` or `OutputBankRef::iter_mut` yielding `&mut OutputRef`. Adjust to use owned `InputRef`/`OutputRef` values.

Acceptance for Milestone 5: Port bank extraction performs no heap allocations per reaction trigger, bank iteration remains ergonomic, and all tests continue to pass.

## Concrete Steps

Work from the repository root. Use these commands and update the plan with actual outputs as work proceeds.

- List or open files as needed with `rg` and `sed`.
- Run tests in the workspace:

    (boomerang/) cargo test

Expected outcome: all tests pass, including new tests added in this plan. If a subset is added, update this section with the exact targeted test command.

## Validation and Acceptance

Acceptance is achieved when the following behaviors are observable:

- A reactor can create a runtime-sized input or output bank using `add_input_bank`/`add_output_bank` (or by declaring a `Vec<T>` in a `#[reactor]` macro), and a reaction can iterate over that bank in a test.
- `connect_ports` returns an explicit error when the number of ports on either side differs, with a message or error variant that points to the mismatch.
- `connect_broadcast` and `connect_cartesian` are available and demonstrated by tests connecting one-to-many and all-to-all across port banks.
- Port bank extraction does not allocate a `Vec` per trigger, as verified by code inspection and (optionally) a profiling run that shows no allocations in the extraction path.
- `cargo test` completes with all tests passing.

## Idempotence and Recovery

Edits are additive and can be re-applied safely. If a change introduces test failures, revert only the specific commit for that change and re-apply with corrections. Avoid `git reset --hard`.

## Artifacts and Notes

Record short transcripts or diffs here as the plan evolves. Examples should be indented and concise. No artifacts yet.

## Interfaces and Dependencies

The following interfaces must exist at the end of this plan:

- In `boomerang_builder/src/port.rs`, a new public `PortBank<T, Q, A = Local>` with methods `len`, `iter`, `get`, and `contained` (for contained child ports).
- In `boomerang_builder/src/reactor.rs`, new APIs:

    pub fn add_input_bank<T: runtime::ReactorData>(&mut self, name: &str, len: usize) -> Result<PortBank<T, Input>, BuilderError>
    pub fn add_output_bank<T: runtime::ReactorData>(&mut self, name: &str, len: usize) -> Result<PortBank<T, Output>, BuilderError>
    pub fn connect_broadcast<T, Q1, Q2, A1, A2>(&mut self, source: TypedPortKey<T, Q1, A1>, targets: impl Iterator<Item = TypedPortKey<T, Q2, A2>>, after: Option<runtime::Duration>, physical: bool) -> Result<(), BuilderError>
    pub fn connect_cartesian<T, Q1, Q2, A1, A2>(&mut self, sources: impl Iterator<Item = TypedPortKey<T, Q1, A1>>, targets: impl Iterator<Item = TypedPortKey<T, Q2, A2>>, after: Option<runtime::Duration>, physical: bool) -> Result<(), BuilderError>

- In `boomerang_runtime/src/port/mod.rs`, new bank references:

    pub struct InputBankRef<'a, T: ReactorData>
    pub struct OutputBankRef<'a, T: ReactorData>

- In `boomerang_runtime/src/refs_extract.rs`, `ReactionRefsExtract` implementations for `PortBank` that produce the bank reference types.

- In `boomerang_builder/src/lib.rs`, a new `BuilderError` variant for connection length mismatch, used by `connect_ports`.

- In `boomerang_runtime/src/refs.rs`, new view types and methods:

    pub struct RefsSlice<'a, T: ?Sized>
    pub struct RefsSliceMut<'a, T: ?Sized>
    impl<'a, T: ?Sized> Refs<'a, T> { pub fn take(&mut self, n: usize) -> Result<RefsSlice<'a, T>, ReactionRefsError>; }
    impl<'a, T: ?Sized> RefsMut<'a, T> { pub fn take(&mut self, n: usize) -> Result<RefsSliceMut<'a, T>, ReactionRefsError>; }

- In `boomerang_runtime/src/port/mod.rs`, updated `InputBankRef`/`OutputBankRef` APIs that return `InputRef`/`OutputRef` by value from `iter`/`iter_mut` and `get`/`get_mut`.

These interfaces should be documented in the code where defined and used in tests.

## Plan Change Notes

Initial ExecPlan created to define port bank APIs, runtime extraction, and explicit multiport connection helpers, based on user requirements for FMI-style dynamic port arrays and safe connection semantics.

Updated Progress to include implemented bank APIs/tests and to highlight pending `#[reactor]` macro support (per user request).

Updated Purpose, Context, Plan of Work, Validation, and Interfaces to add allocation-free port bank extraction, because the current `PortBank` extraction allocates in a hot loop and we need to eliminate per-trigger `Vec` creation.

Updated Progress to note implementation work for allocation-free port bank extraction and the test adjustment, so the next contributor can focus on validation and any cleanup.
