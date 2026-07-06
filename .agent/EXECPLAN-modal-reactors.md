# Minimal Modal Reactors Layer

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Reference: `.agent/PLANS.md` in the repository root. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

Add a minimal modal layer so a reactor can have an explicit mode state, define transitions between modes, and enable or disable reactions based on the current mode. After this change, a user can define named modes on a reactor, mark which reactions are active in which modes, and optionally move the reactor to a new mode when a reaction completes. The visible behavior is that reactions are skipped when their mode is inactive and mode transitions change which reactions can run next. This will be demonstrated by a new test that toggles between two modes and proves only the expected reactions execute.

## Progress

- [x] (2026-01-26 21:43Z) Drafted ExecPlan for minimal modal reactors layer.
- [x] (2026-01-26 22:07Z) Added runtime and builder data structures for mode definitions, reaction mode filters, and reaction transitions.
- [x] (2026-01-26 22:07Z) Implemented modal filtering and deferred transitions in the scheduler with reusable buffers to avoid hot-loop allocations.
- [x] (2026-01-26 22:07Z) Added builder and macro API for modes and transitions.
- [x] (2026-01-26 22:07Z) Added integration test for modal enable/disable and transitions.
- [x] (2026-01-26 22:07Z) Ran `cargo test -p boomerang modal_basic`.
- [x] (2026-01-26 22:17Z) Added LF MixedReactions port and ran `cargo test -p boomerang mixed_reactions`.
- [x] (2026-01-26 22:24Z) Added dynamic mode switching via `Context::set_mode_name` and updated MixedReactions to use it.

## Surprises & Discoveries

- Observation: Mode transitions must apply after the full tag to match LF MixedReactions semantics; applying after each level allowed a mode-B reaction to run in the same tag.
  Evidence: `MixedReactions` produced `12345` on first tick until transitions were deferred until after `process_tag` completed.
- Observation: Dynamic mode switching requires a runtime-resolvable identifier; mode names are stored in the graph and resolved after each tag.
  Evidence: `Context::set_mode_name("B")` schedules a transition resolved by `ReactionGraph::mode_for_reactor_name`.

## Decision Log

- Decision: Implement transitions as static per-reaction targets (no dynamic guards in the reaction body).
  Rationale: Minimal viable modal layer with clear semantics and low API surface; guards can be modeled by splitting reactions.
  Date/Author: 2026-01-26 / Codex.
- Decision: Apply mode gating at execution time (per reaction invocation) rather than when scheduling triggers.
  Rationale: Keeps scheduling simple while still enforcing modal activation rules.
  Date/Author: 2026-01-26 / Codex.
- Decision: When multiple reactions in the same level transition a reactor in one tag, the last executed reaction wins.
  Rationale: Deterministic given current reaction iteration order and minimal extra bookkeeping.
  Date/Author: 2026-01-26 / Codex.
- Decision: Avoid any new allocations in scheduler hot loops by precomputing mode filters and reusing buffers.
  Rationale: Protect benchmark performance; keep filtering constant-time with no per-tag heap growth.
  Date/Author: 2026-01-26 / Codex.
- Decision: Defer applying mode transitions until after the entire tag completes.
  Rationale: Match LF modal semantics where a mode change takes effect on the next tag; avoid mutable aliasing of the Store while reaction triggers hold borrows.
  Date/Author: 2026-01-26 / Codex.
- Decision: Provide dynamic mode switching by name via `Context::set_mode_name(&'static str)` and resolve names after each tag.
  Rationale: Allows reaction bodies to choose modes without exposing runtime keys; avoids hot-loop allocations.
  Date/Author: 2026-01-26 / Codex.

## Outcomes & Retrospective

Not started yet.

## Context and Orientation

Boomerang is a Rust workspace. Runtime scheduling and reaction execution live in `boomerang_runtime`, while the DSL and builder logic live in `boomerang_builder` and `boomerang_macros`. Reactions are scheduled in `boomerang_runtime/src/sched.rs` and executed through the `Store` in `boomerang_runtime/src/store.rs`. Builder-side reaction definitions are created in `boomerang_builder/src/reaction.rs` and lowered into runtime structures in `boomerang_builder/src/env/build.rs`. Macro syntax for reactions lives in `boomerang_macros/src/reaction.rs`.

Definitions used here:
- Mode: A named state attached to a reactor. Each reactor has zero or more modes; if none are defined, all reactions are always enabled.
- Mode filter: A list of modes in which a reaction is allowed to run.
- Mode transition: A rule that, when a reaction completes, sets the reactor’s current mode to a target mode.
- Hot loop: The scheduler path that runs per-tag and per-reaction in `boomerang_runtime/src/sched.rs`, where new allocations must be avoided.

Key files to modify:
- `boomerang_runtime/src/env/mod.rs` (ReactionGraph fields, ModeKey definition, insert APIs).
- `boomerang_runtime/src/store.rs` (current mode tracking).
- `boomerang_runtime/src/sched.rs` (mode gating + transitions during execution).
- `boomerang_builder/src/reactor.rs` (mode definition API).
- `boomerang_builder/src/reaction.rs` (mode filters + transition on reaction builders).
- `boomerang_builder/src/env/mod.rs` and `boomerang_builder/src/env/build.rs` (mode builders and runtime mapping).
- `boomerang_macros/src/reaction.rs` (optional modal syntax).
- `boomerang/src/lib.rs` (prelude re-exports if needed).
- `boomerang/tests/modal_basic.rs` (new test).

## Plan of Work

First, add a minimal representation of modes to runtime: a `ModeKey` type, per-reactor initial mode, and per-reaction mode filters/transition targets in `ReactionGraph`. Then, add mode state tracking in `Store`, initialized from the graph. Next, update the scheduler so each reaction is skipped if its mode filter does not include the reactor’s current mode, and apply mode transitions after reaction completion. The scheduler changes must avoid new allocations: filtering should reuse preallocated buffers or stack-based iteration, and any per-reaction data needed for filtering should be precomputed and stored in `ReactionGraph` or `Store`.

On the builder side, introduce mode builders keyed by reactor, and methods to define modes and mark an initial mode. Extend reaction builders to accept an optional mode filter and transition target, and lower these into runtime graph fields during build. Update the `reaction!` macro to accept optional modal modifiers, so users can declare modes and transitions without dropping to manual builder chaining.

Finally, add a focused integration test that demonstrates a reactor toggling between two modes, with reactions only firing in their allowed modes, and verify that the transition rule affects downstream behavior.

## Concrete Steps

1. Inspect current build pipeline to place mode definitions.
    - Command (from repo root):
        rg -n "insert_reaction|ReactionGraph" boomerang_runtime/src boomerang_builder/src
    - Expected: references in `boomerang_runtime/src/env/mod.rs` and `boomerang_builder/src/env/build.rs`.

2. Add runtime mode structures and APIs.
    - Edit `boomerang_runtime/src/env/mod.rs`:
        - Define `ModeKey` via `tinymap::key_type!`.
        - Add fields to `ReactionGraph`:
            - `reaction_modes: TinySecondaryMap<ReactionKey, Option<ModeFilter>>`
            - `reaction_transitions: TinySecondaryMap<ReactionKey, Option<ModeKey>>`
            - `reactor_initial_modes: TinySecondaryMap<ReactorKey, Option<ModeKey>>`
            - `reactor_modes: TinySecondaryMap<ReactorKey, Vec<ModeKey>>` (for debug/introspection).
        - Define `ModeFilter` as a compact, allocation-free structure suitable for hot loops (for example, a bitset over ModeKey indices or a small fixed array with a length and inline storage).
        - Add `Enclave::insert_mode(reactor_key: ReactorKey, name: &str, initial: bool) -> ModeKey`.
        - Extend `Enclave::insert_reaction` signature to accept mode filter + transition target and store in graph.
        - Update `Env::validate` to assert new maps are populated for each reaction/reactor.
    - Edit `boomerang_runtime/src/env/debug.rs` to include new fields in debug output.

3. Add store tracking of current modes.
    - Edit `boomerang_runtime/src/store.rs`:
        - Add `reactor_modes: TinySecondaryMap<ReactorKey, Option<ModeKey>>` to `Store`.
        - Initialize in `Store::new` from `ReactionGraph::reactor_initial_modes`.
        - Add helpers:
            - `fn current_mode(&self, reactor_key: ReactorKey) -> Option<ModeKey>`
            - `fn set_mode(&mut self, reactor_key: ReactorKey, mode: ModeKey)`
        - Ensure `Store::into_env` remains unchanged (modes are runtime state only).

4. Apply mode gating and transitions in scheduler without allocations.
    - Edit `boomerang_runtime/src/sched.rs`:
        - Replace any per-level collection that allocates (e.g., building a new Vec each iteration). Instead, reuse a preallocated Vec stored on the scheduler struct, or iterate twice over the key set when necessary.
        - For each reaction key at a level, check `reaction_graph.reaction_modes` and the reactor’s current mode in `Store`.
        - Skip reactions not enabled; only execute reactions whose mode filter matches.
        - After each reaction trigger, if `reaction_graph.reaction_transitions[reaction_key]` is `Some(mode)`, call `store.set_mode(reactor_key, mode)`.
        - Update stats to reflect only executed reactions.
        - Ensure no allocations are introduced in the per-tag execution path; if a temporary buffer is needed, allocate it once on `Scheduler` and clear it between levels.

5. Add builder support for modes.
    - Edit `boomerang_builder/src/reactor.rs`:
        - Add a `BuilderModeKey` slotmap key type.
        - Add mode bookkeeping to `ReactorBuilder` (mode keys + order + initial).
        - Add `ReactorBuilderState::add_mode(name: &str, initial: bool) -> Result<BuilderModeKey, BuilderError>`.
    - Edit `boomerang_builder/src/env/mod.rs`:
        - Add `mode_builders: SlotMap<BuilderModeKey, ModeBuilder>` with `ModeBuilder { name, reactor_key, initial }`.
        - Enforce duplicate-name checks per reactor and single initial mode.
    - Edit `boomerang_builder/src/env/build.rs`:
        - Add mode alias map to `BuilderAliases`.
        - Add a `build_runtime_modes` phase that inserts modes into runtime enclaves before reactions.
        - Map builder mode keys to runtime mode keys for use in reaction lowering.

6. Extend reaction builder to carry modal metadata.
    - Edit `boomerang_builder/src/reaction.rs`:
        - Add `enabled_modes: Option<Vec<BuilderModeKey>>` and `transition_to: Option<BuilderModeKey>` to `ReactionBuilder`.
        - Add builder methods on `PartialReactionBuilder`:
            - `with_modes(modes: impl IntoIterator<Item = BuilderModeKey>)`
            - `with_transition(mode: BuilderModeKey)`
        - Ensure `finish()` validates that referenced modes belong to the same reactor.

7. Update reaction macro syntax for modal annotations.
    - Edit `boomerang_macros/src/reaction.rs`:
        - Extend grammar to accept optional modifiers before the code block:
            - `@modes(mode_a, mode_b)`
            - `@transition(mode_b)`
        - Emit `.with_modes([...])` and `.with_transition(...)` when modifiers are present.
        - Keep existing syntax valid without changes.

8. Re-export new builder types if needed.
    - Edit `boomerang/src/lib.rs` prelude to re-export `BuilderModeKey` if user code needs explicit typing.

9. Add integration test demonstrating modal behavior.
    - Create `boomerang/tests/modal_basic.rs`:
        - Define a reactor with two modes (`mode_a` initial, `mode_b`).
        - Add a logical action `pulse`.
        - Add two reactions triggered by `pulse`, each enabled in a different mode.
        - Reaction A transitions to mode B; Reaction B transitions to mode A.
        - Use a startup reaction to schedule two `pulse` actions at different logical times.
        - Assert that each reaction runs exactly once and in the correct order.

## Validation and Acceptance

Behavioral acceptance:
- When a reactor is in `mode_a`, only reactions declared in `mode_a` execute.
- After a reaction transitions the reactor to `mode_b`, subsequent triggered reactions in the same tag and future tags only run if they are enabled for `mode_b`.
- Benchmarks should show no regression attributable to per-tag allocations (qualitative check: no new allocations in scheduler hot loop).

Validation commands (from repo root):
    cargo test -p boomerang modal_basic
Expected outcome:
    - The new test `modal_basic` passes.
    - No existing tests regress (optionally run `cargo test` for full confidence).

## Idempotence and Recovery

All changes are additive. Re-running build/test steps is safe. If a change causes a compile error, revert the last edited file or comment out the new mode-specific calls in the test to isolate the issue. No destructive operations are required.

## Artifacts and Notes

Expected test transcript snippet (illustrative):
    running 1 test
    test modal_basic ... ok
    test result: ok. 1 passed; 0 failed

Actual test commands:
    cargo test -p boomerang modal_basic
    cargo test -p boomerang mixed_reactions

Example modal usage pattern (non-normative):
    let mode_a = builder.add_mode("mode_a", true)?;
    let mode_b = builder.add_mode("mode_b", false)?;
    reaction! {
        (pulse) @modes(mode_a) @transition(mode_b) {
            state.a_count += 1;
        }
    }

## Interfaces and Dependencies

New/updated interfaces to implement:

- `boomerang_runtime/src/env/mod.rs`:
    - `tinymap::key_type! { pub ModeKey }`
    - `ReactionGraph` fields:
        - `reaction_modes: TinySecondaryMap<ReactionKey, Option<ModeFilter>>`
        - `reaction_transitions: TinySecondaryMap<ReactionKey, Option<ModeKey>>`
        - `reactor_initial_modes: TinySecondaryMap<ReactorKey, Option<ModeKey>>`
        - `reactor_modes: TinySecondaryMap<ReactorKey, Vec<ModeKey>>`
    - `Enclave::insert_mode(reactor_key: ReactorKey, name: &str, initial: bool) -> ModeKey`
    - `Enclave::insert_reaction(..., mode_filter: Option<ModeFilter>, transition_to: Option<ModeKey>)`

- `boomerang_runtime/src/store.rs`:
    - `reactor_modes: TinySecondaryMap<ReactorKey, Option<ModeKey>>`
    - `fn current_mode(&self, reactor_key: ReactorKey) -> Option<ModeKey>`
    - `fn set_mode(&mut self, reactor_key: ReactorKey, mode: ModeKey)`

- `boomerang_builder/src/reactor.rs`:
    - `slotmap::new_key_type! { pub struct BuilderModeKey; }`
    - `ReactorBuilderState::add_mode(...)`

- `boomerang_builder/src/reaction.rs`:
    - `ReactionBuilder { enabled_modes: Option<Vec<BuilderModeKey>>, transition_to: Option<BuilderModeKey>, ... }`
    - `PartialReactionBuilder::with_modes(...)`
    - `PartialReactionBuilder::with_transition(...)`

- `boomerang_builder/src/env/build.rs`:
    - `BuilderAliases::mode_aliases`
    - `build_runtime_modes()` invoked before reactions

- `boomerang_macros/src/reaction.rs`:
    - Optional `@modes(...)` and `@transition(...)` parsing and codegen.

Dependencies:
- No new external crates. Use existing `tinymap`, `slotmap`, and `itertools` already in use.

Note: If using a bitset-based `ModeFilter`, ensure it is constructed once at build time and stored in the graph. Any checks in `Scheduler::process_tag` should be constant-time and allocation-free.

Change log: 2026-01-26 / Codex: updated ExecPlan to address performance constraint (avoid allocations in scheduler hot loops) and wrote plan to `.agent/EXECPLAN-modal-reactors.md`.
Change log: 2026-01-26 / Codex: updated progress, added borrow-related discovery, recorded deferred transition decision, and captured test command after implementation.
Change log: 2026-01-26 / Codex: adjusted transition semantics to apply after the full tag and added LF MixedReactions test plus test command.
Change log: 2026-01-26 / Codex: added dynamic mode switching by name and updated MixedReactions to use it.
