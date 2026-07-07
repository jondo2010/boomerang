# Full Modal Reactors

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Reference: `.agent/PLANS.md` in the repository root. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

Implement full modal reactor semantics in Boomerang. After this work, a Boomerang reactor can declare mutually exclusive modes whose contained reactions, timers, logical actions, child reactors, and delayed connections execute only while their enclosing mode is active. A reaction can transition to another mode, using reset behavior by default or history behavior when requested. Reset discards pending local events and reinitializes modal timing behavior; history suspends and later resumes local time as if no time passed while the mode was inactive.

The user-visible result is a reactor that can model behavior such as "idle" and "active" phases without manually guarding every reaction and without timers in inactive modes continuing to consume logical time. This will be demonstrated by integration tests adapted from an external modal-model behavior corpus, especially cases where a pending action in a history mode resumes later while a pending action in a reset mode is discarded.

## Progress

- [x] (2026-07-06 18:26Z) Replaced the prior minimal reaction-gating ExecPlan with this full-semantics modal reactors plan.
- [x] (2026-07-06 20:32Z) Added book documentation requirements, no-external-reference guidance for implementation/user docs, and a minimal upstream test-port set.
- [x] (2026-07-06 20:46Z) Began implementation by adding `ModeKind`, `TransitionKind`, typed `BuilderModeEffect`, `ModeEffectRef::set(ctx)`, `(reset)` parser support, and `reset(mode)` / `history(mode)` effect parsing while preserving the existing spike tests.
- [x] (2026-07-06 21:02Z) Verified the first typed-transition slice with focused builder, macro, and modal integration tests plus `git diff --check` and no-external-reference scans.
- [x] (2026-07-06 21:09Z) Implemented the valid Rust structural syntax `mode! { initial idle { ... } }` and `mode! { active { ... } }`, including macro parser tests and an end-to-end modal integration test.
- [x] (2026-07-06 21:24Z) Added first static scope metadata: runtime root/mode `ScopeKey`s, action/port/reaction scope maps, builder scope ownership for mode-local actions and reactions, port rejection inside modes, and reset-trigger validation.
- [x] (2026-07-06 21:39Z) Extended static scope ownership to child reactor instances and delayed/physical connection helper reactors before scheduler local-time work.
- [x] (2026-07-06 21:49Z) Replaced the temporary compatibility syntax with macro-generated mode handles for typed transition effects in structural mode declarations.
- [x] (2026-07-06 21:58Z) Added scheduler active-scope checks so reactions in child reactors nested under inactive modes do not execute.
- [x] (2026-07-06 22:02Z) Resolved typed mode effects to runtime `ModeKey`s during lowering so `.set(ctx)` no longer does name lookup in the scheduler.
- [x] (2026-07-07 06:45Z) Added the first mode-local scheduling slice: mode-scoped logical actions are held in per-scope local queues, history re-entry rebases pending action values to the resumed global tag, reset re-entry clears pending action queues and values, and child reactor modes under a reset scope return to their initial modes.
- [x] (2026-07-07 07:02Z) Extended mode-local scheduling to timers: history re-entry resumes pending timer firings with remaining local delay, reset re-entry clears stale timer firings and schedules fresh timer startup events from reset local time zero, and initial active zero-offset timers can still fire at startup.
- [x] (2026-07-07 07:11Z) Implemented reset-trigger reactions: builder lowering records reset-triggered reactions by owning runtime scope, reset transitions enqueue reset reactions in the reset subtree for the next microstep, and initial active modes do not run reset reactions merely because the program started.
- [x] (2026-07-07 08:24Z) Implemented modal startup and shutdown lifecycle behavior: first activation schedules scoped startup reactions once, shutdown reactions run for scopes that were activated at least once even if inactive at shutdown, and unreachable mode lifecycle reactions do not run.
- [x] (2026-07-07 08:28Z) Verified delayed connection helper reactors under reset/history: delayed connection actions inherit the enclosing mode scope, history re-entry resumes a pending delayed delivery with remaining local delay, and reset re-entry discards the pending delayed delivery.
- [x] (2026-07-07 08:31Z) Implemented the mode-local physical-action caveat: physical actions carry static action-kind metadata, stay on the global queue, run only if their scope is active at the physical event tag, and are not suspended or replayed by history.
- [x] (2026-07-07 08:33Z) Added the first-wave three-mode cycle integration test, proving reset transitions cycle through three sibling modes and only the active mode responds to each root trigger.
- [x] (2026-07-07 08:45Z) Added linked book documentation for modal reactors, concise glossary entries for modal terms, and removed the obsolete `book/book.toml` `multilingual` field so `mdbook build book` succeeds locally.
- [x] (2026-07-07 08:51Z) Implemented modal cycle-breaker dependency analysis: builder reaction dependency edges now skip pairs of reactions owned by mutually exclusive sibling modes, and `boomerang/tests/modal_cycle_breaker.rs` proves opposing dependencies in sibling modes do not form a static cycle.
- [x] (2026-07-07 08:51Z) Added modal multiport-bank integration coverage in `boomerang/tests/modal_multiport_bank.rs`, proving banked child reactors and multiport connections work inside an active mode.
- [ ] Extend mode-local event queues and local-time scheduling to the planned inactive-scope performance benchmark.
- [ ] Run final full-workspace validation and benchmark suite.

## Surprises & Discoveries

- Observation: The prior WIP proves that per-reaction gating and deferred transition application can pass simple mixed-reaction tests, but that design is not enough for full modal reactors.
  Evidence: The WIP only stores reaction mode filters and current mode state; it has no ownership of timers, logical actions, delayed connections, child reactors, queued local events, reset/history transition kinds, or startup/shutdown special cases.

- Observation: To make mode declarations ergonomic inside `#[reactor]`, the reactor macro must parse the function body itself and intercept `mode!` statements rather than leaving them as ordinary Rust macro calls.
  Evidence: Forward references such as a reaction inside `idle` transitioning to `active` require the macro to discover all mode names before emitting builder code for any mode body.

- Observation: The book exists under `book/src`, and `book/src/SUMMARY.md` currently links only Introduction, Quickstart, and Glossary pages.
  Evidence: `find book/src -maxdepth 2 -type f -print` returns `book/src/replay.md`, `book/src/SUMMARY.md`, `book/src/glossary.md`, `book/src/quickstart.md`, and `book/src/introduction.md`; `book/src/SUMMARY.md` does not yet link `replay.md` or any modal reactors page.

- Observation: The upstream C modal-model directory contains a broad suite, but a smaller set covers distinct semantics needed for Boomerang's first full implementation.
  Evidence: The directory listing includes `Count3Modes.lf`, `MixedReactions.lf`, `ModalActions.lf`, `ModalAfter.lf`, `ModalCycleBreaker.lf`, `ModalMultiport.lf`, `ModalMultiportBank.lf`, `ModalNestedReactions.lf`, `ModalStartupShutdown.lf`, `ModalStateReset.lf`, `ModalTimers.lf`, several bank/state variants, and a `util` directory. The selected port list below keeps one representative for each distinct semantic area.

- Observation: The first implementation slice can add typed mode transition effects without changing scheduler behavior yet.
  Evidence: `cargo test -p boomerang_builder test_mode_kind_effect_and_reset_trigger_builder`, `cargo test -p boomerang_macros reaction::tests`, `cargo test -p boomerang modal_basic`, and `cargo test -p boomerang mixed_reactions` all pass after adding `BuilderModeEffect` and updating `modal_mixed_reactions.rs` to call `mode_b.set(ctx)`.

- Observation: Full `cargo fmt` is temporarily blocked by pre-existing trailing whitespace in `boomerang_macros/src/ports.rs`, outside this modal slice.
  Evidence: `cargo fmt` reports trailing whitespace at `boomerang_macros/src/ports.rs:140:140`, `:164:129`, `:203:147`, and `:209:151`. Targeted `rustfmt` on the touched modal files succeeds, and `git diff --check` passes after removing formatter spillover from unrelated runtime files.

- Observation: Bare `initial mode idle { ... }` syntax cannot be used inside a Rust function body handled by an attribute macro.
  Evidence: `cargo test -p boomerang modal_structural_syntax` failed before macro expansion with `expected one of '!', '.', '::', ';', '?', '{', '}', or an operator, found 'mode'` at `initial mode mode_a`. The working syntax is a valid Rust macro statement that `#[reactor]` intercepts: `mode! { initial mode_a { ... } }`.

- Observation: Static scope metadata can be added without changing scheduler behavior yet, but it must be assigned after reactor and mode aliases exist.
  Evidence: `cargo test -p boomerang_builder`, `cargo test -p boomerang_runtime env::tests`, `cargo test -p boomerang modal_structural_syntax`, `cargo test -p boomerang modal_basic`, and `cargo test -p boomerang mixed_reactions` pass after adding `ScopeKey`, root/mode scopes, and action/port/reaction scope maps. The new builder test `test_runtime_scope_metadata_for_mode_components` verifies a mode-local action and reaction map to the mode scope while a reactor port and root reaction map to the root scope.

- Observation: Child reactors and synthetic connection reactors need scope ownership before local-time scheduling is introduced, because delayed and cross-enclave connections are represented as helper reactors in the builder graph.
  Evidence: `cargo test -p boomerang_builder test_child_and_connection_helper_reactors_inherit_mode_scope` passes after `ReactorBuilderState::add_child_with` and `ConnectionBuilder` propagate the current mode to child/helper reactors. The test verifies that source, target, and delayed connection helper reactor root scopes all have the enclosing mode scope as their parent.

- Observation: The structural macro must tolerate mode handles that are generated for forward references but are not used by the user code in that mode's body.
  Evidence: Converting `boomerang/tests/modal_mixed_reactions.rs` to `mode!` blocks initially produced an unused `mode_a` warning because that mode was only a scope, not a transition effect. Emitting `#[allow(unused_variables)]` on generated mode-effect handles removes the warning while preserving forward references.

- Observation: Once modal integration tests use explicit typed effects, the old unconditional transition graph slot is unnecessary.
  Evidence: `cargo test -p boomerang_macros`, `cargo test -p boomerang_builder`, `cargo test -p boomerang_runtime env::tests`, `cargo test -p boomerang modal`, and `cargo test -p boomerang mixed_reactions` pass after removing `@modes(...)`, `@transition(...)`, `Context::set_mode_name`, `PartialReactionBuilder::with_modes`, `PartialReactionBuilder::with_transition`, and runtime `reaction_transitions`.

- Observation: Child reactors created through the normal macro-facing `add_child_reactor` path did not inherit the enclosing mode scope, even after `add_child_with` was fixed.
  Evidence: The new `boomerang/tests/modal_nested_reactions.rs` initially failed because a child reactor declared inside an inactive `active` mode still ran its `(startup)` reaction and panicked. Threading `scope_mode` through the internal `Reactor::build` path and checking `Store::scope_is_active` in the scheduler makes `cargo test -p boomerang modal_nested_reactions` pass.

- Observation: Typed mode effects can be resolved from builder keys to runtime keys once, when the reaction closure is lowered, instead of leaking mode names and resolving names after every transition request.
  Evidence: `rg -n "Box::leak|new_name|ModeTransitionTarget|mode_for_reactor_name|target_name" boomerang_builder boomerang_runtime` returns no matches after adding `ResolveModeEffects`, and `cargo test -p boomerang_builder`, `cargo test -p boomerang_macros`, `cargo test -p boomerang modal_basic`, `cargo test -p boomerang mixed_reactions`, and `cargo test -p boomerang modal_structural_syntax` pass.

- Observation: The existing action store prunes values older than the current global tag, so a mode-local action value cannot simply remain stored under its original local tag while the mode is inactive.
  Evidence: The new `boomerang/tests/modal_actions.rs` history test initially failed with the pending action firing at 6 ns instead of 7 ns, and the reset test initially failed because the stale pending action still fired. After adding per-scope queues and rebasing stored action values on history activation, `cargo test -p boomerang --test modal_actions` passes.

- Observation: Reaction result handling cannot borrow `Store` immutably while reaction contexts still hold the mutable borrow used for executing the current level.
  Evidence: The first `EventManager::push_action_event` version queried `Store::action_is_logical` from the scheduled-action loop and failed to compile with `cannot borrow *self.store as immutable because it is also borrowed as mutable`. This slice now routes by static action scope only; the physical-action caveat remains for the later semantics/documentation milestone.

- Observation: Timer actions and reactor lifecycle startup actions are both represented as `ActionType::Timer`, but reset transitions must restart only user-declared timers, not `(startup)` lifecycle reactions.
  Evidence: `ReactorBuilderState::new` creates an internal action named `__startup` through `add_startup_action`, and `EnvBuilder::build_runtime_reactions` previously inserted all `ActionType::Timer` triggers into `ReactionGraph::startup_actions`. The timer reset slice adds `ReactionGraph::timer_startup_actions` and excludes the internal `__startup` action so reset can reschedule timers without treating startup reactions as timer firings.

- Observation: Initial active mode scopes and transition-activated mode scopes need different handling for immediate local events.
  Evidence: A zero-offset mode-local timer in an initial mode should be eligible at program startup, but a zero-offset timer in a mode entered by transition must wait until the next microstep after the transition tag. `ScopeTimeState::allow_activation_tag` captures this distinction, and `cargo test -p boomerang --test modal_timers` passes with both history and reset timer behavior.

- Observation: Reset-triggered reactions already participate in the builder reaction graph as ordinary reaction nodes, even though they have no action or port triggers.
  Evidence: `build_reaction_graph` explicitly adds every `BuilderReactionKey` as a node before level assignment, so the reset-trigger slice could reuse `build_runtime_level_map` and only needed new runtime metadata: `ReactionGraph::reset_reactions`, filled during `EnvBuilder::build_runtime_reactions`.

- Observation: Reset reactions must be scheduled after the transition reaction completes, not run inline during transition application.
  Evidence: The new `boomerang/tests/modal_reset_reactions.rs` test asserts that a reset reaction entered from a tag at microstep 0 runs at microstep 1. `cargo test -p boomerang --test modal_reset_reactions` passes after scheduling reset reactions at `current_tag.delay(Duration::ZERO)`.

- Observation: Reactor lifecycle startup and shutdown are represented by internal actions whose action scope is the reactor root, even when the lifecycle reaction is declared inside a mode.
  Evidence: Mode startup on first activation could not be implemented by re-triggering the internal startup action normally, because that would also run root startup reactions again. The lifecycle slice adds scoped startup and shutdown reaction maps that carry both the reaction and the internal action key, so the runtime can make `startup` or `shutdown` present while scheduling only the scoped lifecycle reactions.

- Observation: Timeout shutdown events are scheduled during program startup, before later mode activations can mark their scopes as activated.
  Evidence: The new `modal_timeout_shutdown_uses_activation_history_at_shutdown_time` test would miss a shutdown reaction for a mode activated after startup if shutdown reactions were filtered when the timeout event was scheduled. The scheduler now places all shutdown lifecycle reactions in terminal events and filters them at terminal processing time using per-scope activation history.

- Observation: Delayed connections did not need a separate runtime event type after helper reactors inherited mode scope.
  Evidence: Delayed connections are lowered into helper reactors with a logical action carrying the delay. Because that helper action is already mode-scoped and stored in the per-scope local queue, `cargo test -p boomerang --test modal_delayed_connections` passes for both history resume at 13 ns and reset discard without additional runtime changes.

- Observation: Physical actions need action-kind metadata in `ReactionGraph` because event routing cannot infer logical versus physical behavior from scope alone.
  Evidence: Prior scoped action routing would place a mode-local physical action into the per-scope local queue, making history re-entry replay it. The new `action_is_logical` runtime map lets `EventManager::push_action_event` keep physical actions on the root global queue, and `cargo test -p boomerang --test modal_physical_actions` proves an event that occurs while the mode is inactive is dropped rather than replayed after history re-entry.

- Observation: The local `mdbook` version rejects the existing `book/book.toml` `multilingual = false` field.
  Evidence: `mdbook build book` failed with `unknown field 'multilingual'` before the documentation slice changed `book/book.toml`. Removing that obsolete field lets `mdbook build book` complete and write the HTML book to `book/book`.

- Observation: A model with no startup work and no events can underflow natural shutdown scheduling from `Tag::NEVER`.
  Evidence: The first `modal_cycle_breaker` test had no startup reaction and panicked at `boomerang_runtime/src/time.rs:75` while calculating shutdown from `Tag::NEVER`. The test now schedules an ordinary startup shutdown so this milestone specifically covers dependency-cycle behavior. The no-event shutdown edge remains a separate runtime cleanup candidate.

- Observation: The existing internal reaction-priority chain creates edges between all reactions in a reactor, regardless of mode scope.
  Evidence: The cycle-breaker slice rewrote the internal edge construction to add priority edges only between reaction pairs that can be active together, while retaining a total order among compatible pairs. `cargo test -p boomerang_builder` and `cargo test -p boomerang modal` pass after this change.

- Observation: Modal banked child reactors and multiport connections work with the existing scope propagation.
  Evidence: The new `boomerang/tests/modal_multiport_bank.rs` test creates a bank of child reactors inside the active mode, fully connects each banked output to each multiport input, and passes with `cargo test -p boomerang --test modal_multiport_bank` and `cargo test -p boomerang modal`.

## Decision Log

- Decision: Implement full modal semantics, not the minimal reaction-filter feature.
  Rationale: The feature is only useful as "modal reactors" if it controls modal components and local time, not just whether a reaction body is skipped.
  Date/Author: 2026-07-06 / Codex, confirmed by user.

- Decision: Parse `mode! { initial name { ... } }` and `mode! { name { ... } }` statements as part of the `#[reactor]` macro input.
  Rationale: This gives users structural mode blocks, supports forward references to modes, and lets the macro generate typed mode handles before expanding mode bodies. The earlier bare `initial mode name { ... }` spelling is not valid Rust in function bodies before attribute macro expansion.
  Date/Author: 2026-07-06 / Codex.

- Decision: Reset is the default transition kind; `history(mode_name)` explicitly requests history behavior.
  Rationale: Reset is the safer default because it discards stale local events and starts the target mode from a predictable timing state. History is less common and should be visually explicit at the transition site.
  Date/Author: 2026-07-06 / Codex, confirmed by user.

- Decision: Do not introduce automatic Rust state field reset in the first full implementation. Use explicit reset reactions over existing Rust state.
  Rationale: Boomerang state is arbitrary Rust data created from `#[state]` function parameters. Automatically resetting selected fields would require a second state-definition DSL or trait bounds that do not fit existing ergonomics. Explicit reset reactions keep state reset behavior local, visible, and compatible with normal Rust ownership.
  Date/Author: 2026-07-06 / Codex.

- Decision: Use typed mode transition effects instead of `Context::set_mode_name(&'static str)`.
  Rationale: String lookup is slow, typo-prone, and bypasses builder validation. A transition effect declared in a reaction signature lets the builder validate ownership and transition kind once.
  Date/Author: 2026-07-06 / Codex.

- Decision: Represent local time with per-scope local event queues plus an active-scope frontier, not by scanning all queued events on every transition.
  Rationale: Performance depends on inactive modes being cheap. Suspending a mode should not require updating every event in the system, and finding the next event should consider only active scopes.
  Date/Author: 2026-07-06 / Codex.

- Decision: Keep references to the external reference language and test suite inside this ExecPlan only.
  Rationale: Boomerang's public API, implementation comments, diagnostics, tests, and user book should stand on their own. User-facing documentation should describe Boomerang behavior directly, not as a comparison to another language.
  Date/Author: 2026-07-06 / Codex, requested by user.

- Decision: Add book documentation as a required milestone for the feature.
  Rationale: Modal reactors introduce new front-end syntax and non-obvious timing semantics. The feature is not complete unless users can learn the syntax, reset/history behavior, lifecycle triggers, and local-time behavior from `book/src` without reading the implementation or this ExecPlan.
  Date/Author: 2026-07-06 / Codex, requested by user.

- Decision: Port a minimal first-wave set of upstream modal-model tests, and explicitly defer redundant or language-specific cases.
  Rationale: The first-wave set should prove one representative of each semantic class without spending implementation time on duplicates. Rust-specific state reset behavior must be tested with Boomerang-native reset reactions rather than by copying language-specific state syntax.
  Date/Author: 2026-07-06 / Codex.

- Decision: Preserve the spike's `@modes(...)`, `@transition(...)`, and `Context::set_mode_name(...)` APIs temporarily while introducing typed transition effects.
  Rationale: This keeps the existing modal spike tests green while the implementation is migrated toward the planned API. These compatibility paths should be deleted before final acceptance once structural mode blocks and runtime transition semantics are complete.
  Date/Author: 2026-07-06 / Codex.

- Decision: Remove the spike compatibility syntax and unconditional transition builder/runtime path now that structural `mode!` tests use typed effects.
  Rationale: Keeping two modal front-end APIs would obscure the developer API and create a second transition semantic where a reaction always transitions merely because it declared a target. The intended API requires user code to call `.set(ctx)`, which makes conditional transitions explicit and matches reset/history effect declarations.
  Date/Author: 2026-07-06 / Codex.

- Decision: Treat the optional enclosing mode on `Reactor::build` as internal macro/developer plumbing and expose `ReactorBuilderState::set_scope_mode` as `#[doc(hidden)]`.
  Rationale: Normal users should continue to express scope with `mode!` blocks. The builder still needs a validated way for generated child-reactor construction to attach a child root scope to the enclosing mode before the child adds ports, actions, reactions, and its own children.
  Date/Author: 2026-07-06 / Codex.

- Decision: Resolve typed mode transition effects to runtime `ModeKey`s during reaction lowering.
  Rationale: A mode transition is a hot-path runtime operation. The builder already knows the exact target mode and has alias maps during lowering, so name strings and scheduler lookups are unnecessary overhead and make invalid targets harder to reason about.
  Date/Author: 2026-07-06 / Codex.

- Decision: Introduce an internal `EventManager` that keeps the existing root event queue and adds per-scope local queues with a frontier heap.
  Rationale: This preserves non-modal root-scope behavior while giving mode-owned actions a queue that can become dormant while inactive. Frontier entries carry an epoch so queue changes, activation, and reset can invalidate stale heap entries without scanning queued events on every tag.
  Date/Author: 2026-07-07 / Codex.

- Decision: Rebase stored action values when a local queue becomes active rather than changing the public `Context::schedule_action` or `ActionRef` API in this slice.
  Rationale: `Context::schedule_action` currently stores values immediately through `ActionRef`, and `ActionStore` expects lookup tags to be monotonic in global execution. Rebasing keeps pending local values visible at the resumed global tag and avoids changing user-facing reaction signatures.
  Date/Author: 2026-07-07 / Codex.

- Decision: Collapse transition requests to last-wins per reactor before applying reset/history side effects.
  Rationale: The previous loop set modes sequentially. Once transitions clear queues, reset child modes, and rebase local time, applying intermediate same-tag transitions would create observable side effects that should not survive the tag.
  Date/Author: 2026-07-07 / Codex.

- Decision: Track timer startup events separately from general startup actions in `ReactionGraph`.
  Rationale: Program startup still needs one unified startup scheduling pass, but reset entry needs to reschedule only timers in the reset subtree. A separate `timer_startup_actions` list keeps reset semantics explicit and avoids accidentally running lifecycle startup reactions on reset entry.
  Date/Author: 2026-07-07 / Codex.

- Decision: Store whether a scope activation may use the activation tag itself.
  Rationale: Scopes active at program startup should preserve existing zero-offset timer behavior at `Tag::ZERO`. Scopes activated by a transition should map local zero to the next microstep so newly active modal work does not run in the same tag as the transition.
  Date/Author: 2026-07-07 / Codex.

- Decision: Store reset-triggered reactions by runtime scope and enqueue the reset subtree on reset entry.
  Rationale: Resetting a mode can reset child reactor modes too. Storing reset reactions by scope lets the scheduler collect the target scope and descendants, then rely on the existing active-scope check to execute only reactions whose scopes are active after the mode switch.
  Date/Author: 2026-07-07 / Codex.

- Decision: Do not run reset reactions for initially active modes at program startup.
  Rationale: `(startup)` and `(reset)` are different lifecycle events. A reset reaction should run because a mode is entered by reset, not because its enclosing mode starts active at program initialization.
  Date/Author: 2026-07-07 / Codex.

- Decision: Track `ever_active` and `startup_fired` per runtime scope.
  Rationale: Startup reactions inside a mode must run once on first activation, while shutdown reactions must run for any scope that was activated at least once. Keeping this state next to the scope-local queue lets the scheduler answer both questions without scanning reactor mode history.
  Date/Author: 2026-07-07 / Codex.

- Decision: Schedule shutdown terminal events with all shutdown lifecycle reactions and filter by activation history at execution time.
  Rationale: A timeout shutdown can be scheduled before a mode is activated. Filtering at schedule time would permanently omit shutdown reactions for modes activated later, so terminal processing must make the final eligibility decision.
  Date/Author: 2026-07-07 / Codex.

- Decision: Treat delayed connections as covered by scoped helper reactors and scoped logical actions rather than adding connection-specific scheduler machinery.
  Rationale: The builder already lowers delayed connections to helper reactors with delayed logical actions, and prior scope ownership work makes those helpers children of the enclosing mode scope. Reusing the existing local action queue keeps the scheduler smaller and gives reset/history behavior identical to user-declared logical actions.
  Date/Author: 2026-07-07 / Codex.

- Decision: Keep mode-scoped physical actions on the global event queue and filter them by active scope at the event tag.
  Rationale: Physical actions happen in wall-clock time, so history should not suspend and replay them as local-time events. Root-queue delivery plus the existing active-scope reaction filter gives the intended behavior: if the mode is inactive when the physical tag is processed, the reaction does not run and the event is not replayed later.
  Date/Author: 2026-07-07 / Codex.

- Decision: Fix the existing mdBook config while adding modal documentation.
  Rationale: The documentation acceptance command is `mdbook build book`. Leaving an obsolete config field in place would make the new modal page unbuildable in this environment even though the page content is valid.
  Date/Author: 2026-07-07 / Codex.

- Decision: Treat reactions scoped to different modes of the same reactor as mutually exclusive for static dependency edges.
  Rationale: Such reactions cannot execute in the same active configuration, so port dependencies or priority-ordering edges between them should not make the global reaction graph cyclic. Root reactions and reactions in the same mode remain ordered normally because they can run together.
  Date/Author: 2026-07-07 / Codex.

## Outcomes & Retrospective

2026-07-07: The first local-time runtime slice is implemented for mode-scoped logical actions scheduled from reactions. The new modal action tests prove the core distinction: history preserves the pending action's remaining local delay, while reset discards the stale pending action. Remaining gaps after this slice were mode-local timers, delayed connections, lifecycle startup/shutdown/reset triggers, physical-action caveat handling, and the performance benchmark that proves inactive queues stay cheap.

2026-07-07: Mode-local timers now use the same scoped queue machinery. The new modal timer tests prove that history resumes a timer after the remaining active local delay and reset restarts the timer from reset entry. Remaining gaps are delayed connections, lifecycle startup/shutdown/reset triggers, physical-action caveat handling, and the inactive-scope performance benchmark.

2026-07-07: Reset-triggered reactions now run on reset entry and remain distinct from startup. The new modal reset reaction tests prove a reset reaction can restore Rust state at the next microstep after a reset transition and that an initially active mode does not run reset reactions at program startup. Remaining gaps are delayed connections, modal startup/shutdown behavior, physical-action caveat handling, and the inactive-scope performance benchmark.

2026-07-07: Modal lifecycle startup and shutdown behavior is now implemented. The new `boomerang/tests/modal_startup_shutdown.rs` tests prove that startup inside a mode runs once on first activation at the next microstep, that re-entering the mode does not run startup a second time, that shutdown inside a previously activated mode runs even after the mode becomes inactive, that unreachable mode lifecycle reactions do not run, and that timeout shutdown uses activation history at shutdown processing time. Remaining gaps are delayed connections, physical-action caveat handling, and the inactive-scope performance benchmark.

2026-07-07: Mode-local delayed connections are now covered by integration tests. The new `boomerang/tests/modal_delayed_connections.rs` tests prove that a pending delayed delivery inside a mode is suspended while inactive, resumes after the remaining active local delay when the mode is re-entered by history, and is discarded when the mode is re-entered by reset. No runtime code change was needed in this slice because delayed connection helper reactors already inherit mode scope and use scoped logical actions. Remaining gaps are physical-action caveat handling, the inactive-scope performance benchmark, remaining first-wave modal tests, and book documentation.

2026-07-07: The physical-action caveat is now implemented and tested. Mode-scoped physical actions are accepted, but they are not local-time events: they remain scheduled by global physical/logical tag conversion, run only if their scope is active at that tag, and are not replayed by history. The new `boomerang/tests/modal_physical_actions.rs` test uses a short wall-clock run to prove an inactive physical event is dropped and not replayed after history re-entry. Remaining gaps are the inactive-scope performance benchmark, remaining first-wave modal tests, and book documentation.

2026-07-07: The three-mode reset cycle first-wave test is now covered by `boomerang/tests/modal_count_3_modes.rs`. The test uses a root one-shot driver action and records the active mode sequence, proving only one sibling mode reacts at each step and reset transitions cycle `one`, `two`, `three` twice. Remaining first-wave gaps include cycle-breaker and multiport/bank coverage, plus the inactive-scope performance benchmark and book documentation.

2026-07-07: User-facing modal reactor documentation is now linked from the book. `book/src/modal-reactors.md` explains mode syntax, reset and history transitions, lifecycle reactions, local-time components, transition timing, and physical actions without naming the external reference language or test corpus. `book/src/glossary.md` now defines the modal terms used by the page. The local `mdbook build book` command passes after removing the obsolete `multilingual` field from `book/book.toml`. Remaining gaps are the inactive-scope performance benchmark and remaining first-wave cycle-breaker and multiport/bank coverage.

2026-07-07: Modal cycle-breaker behavior is now implemented. The builder dependency graph skips port and priority edges between reactions owned by sibling modes of the same reactor, and `boomerang/tests/modal_cycle_breaker.rs` proves opposing same-tag dependencies in mutually exclusive modes build and execute without a static cycle error. Remaining first-wave gaps are multiport/bank coverage and the inactive-scope performance benchmark.

2026-07-07: First-wave multiport and bank coverage is now implemented. `boomerang/tests/modal_multiport_bank.rs` creates banked child reactors inside a mode and checks that a fully connected multiport network receives one value from every peer before shutdown. The focused modal test suite passes with this coverage in place. Remaining gaps are the inactive-scope performance benchmark and final full-workspace validation.

## Context and Orientation

Boomerang is a Rust workspace. User-facing macros live in `boomerang_macros`, builder-side model construction lives in `boomerang_builder`, runtime scheduling lives in `boomerang_runtime`, and integration tests live in `boomerang/tests`.

The existing reactor syntax is a Rust function annotated with `#[reactor]`. The macro provides a `builder` variable in the function body, and helper macros such as `timer!` and `reaction!` expand into builder calls. The current WIP on the branch before this plan added a minimal mode key, reaction filters, and transition after a tag. That code should be treated as a spike: useful evidence, but not the target architecture.

Important current files:

- `boomerang_macros/src/reactor.rs` parses the `#[reactor]` function and emits builder code.
- `boomerang_macros/src/reaction.rs` parses `reaction!` syntax and emits `builder.add_reaction(...).with_trigger(...).with_effect(...).finish()?`.
- `boomerang_builder/src/env/mod.rs` owns the builder graph: reactors, ports, actions, reactions, connections, and validation.
- `boomerang_builder/src/env/build.rs` lowers builder graph data into runtime `Enclave`s, `Env`, and `ReactionGraph`.
- `boomerang_runtime/src/env/mod.rs` defines runtime graph keys and static dependency maps.
- `boomerang_runtime/src/sched.rs` owns the event queue and reaction scheduling loop.
- `boomerang_runtime/src/store.rs` owns runtime reactor state, action stores, port stores, reaction objects, and cached reaction borrow contexts.
- `book/src/SUMMARY.md` is the mdBook table of contents. Add a modal reactors page there.
- `book/src/glossary.md` defines user-facing terms. Add modal terms there only if the new modal page cannot define them clearly enough on first use.

Terms used in this plan:

A mode is a named mutually exclusive state of one reactor. Exactly one sibling mode in a modal group is active at a logical instant.

A mode scope is the static region of the model contained by a mode. Reactions, timers, logical actions, child reactors, and delayed connections can belong to a mode scope. Root scope means the component is outside any mode and always active.

Local time is logical time measured only while a mode scope is active. If a mode is inactive for one second of global logical time, timers and scheduled logical actions in that mode do not age by one second.

A reset transition enters a mode as if its local timing history were new. It discards pending local events in the target scope and recursively resets contained modal reactors to their initial modes. Reset is the default transition kind.

A history transition enters a mode preserving its local timing history. Queued local timers and actions resume with the same remaining delay they had when the mode was deactivated.

A transition effect is a typed value passed to a reaction because the reaction declared that it may set a mode. It is not a port or action value. Calling `active.set(ctx)` records a transition request in the current reaction context.

## Proposed Front-End Syntax

The preferred user syntax integrates modes directly into `#[reactor]` using `mode!` statements. The `mode!` spelling is a valid Rust macro invocation, so the Rust compiler accepts it inside a function body before the `#[reactor]` attribute macro expands. The `#[reactor]` macro intercepts these statements, discovers all mode names first, emits mode handles, and then emits scoped builder code for each mode body. This permits forward references between sibling modes.

Example:

    #[reactor]
    fn Controller(
        #[state] model: ControllerState,
        #[input] cmd: Command,
        #[output] status: Status,
    ) -> impl Reactor {
        timer! { heartbeat(0 sec, 1 sec) }

        reaction! {
            (heartbeat) {
                state.model.global_ticks += 1;
            }
        }

        mode! { initial idle {
            timer! { poll(0 sec, 100 msec) }

            reaction! {
                (startup) {
                    state.model.idle_started = true;
                }
            }

            reaction! {
                (reset) {
                    state.model.reset_idle();
                }
            }

            reaction! {
                (cmd) -> active, status {
                    if cmd.as_ref() == Some(&Command::Start) {
                        active.set(ctx);
                    }
                    *status = Some(Status::Idle);
                }
            }
        } }

        mode! { active {
            timer! { tick(0 sec, 10 msec) }
            let work = builder.add_logical_action::<Work>("work", Some(Duration::milliseconds(500)))?;

            reaction! {
                (tick) -> work {
                    ctx.schedule_action(&mut work, Work::new(), None);
                }
            }

            reaction! {
                (cmd) -> history(idle), status {
                    if cmd.as_ref() == Some(&Command::Pause) {
                        idle.set(ctx);
                    }
                    *status = Some(Status::Active);
                }
            }
        } }
    }

In the example, `heartbeat` and its reaction are outside modes and always active. `poll`, `tick`, `work`, and the mode-local reactions are scoped to their enclosing mode. `-> active` declares a reset transition effect because reset is the default. `-> history(idle)` declares a history transition effect. The closure argument named `active` or `idle` is a typed transition handle, not a string. Calling `.set(ctx)` schedules the mode change; merely declaring the effect does not transition.

Mode syntax is intentionally restricted:

Ports remain reactor-level declarations. A mode may use reactor-level ports in reactions and connections, but may not declare new input or output ports because ports are the stable interface of a reactor.

Nested `mode!` blocks are not allowed directly inside another mode. Hierarchical modal behavior is expressed by instantiating a child reactor that itself has modes inside a parent mode. This keeps the builder graph simple and uses the existing reactor composition model.

An unqualified mode effect such as `-> active` means reset. The explicit spelling `-> reset(active)` is accepted as an alias. The spelling `-> history(active)` requests history.

The `reaction!` macro gains a `reset` trigger keyword. A reset-triggered reaction is valid only inside a mode scope and runs when that mode is entered by reset behavior. A reset reaction is not a transition effect; it is a reaction trigger.

The lower-level builder syntax remains available for generated code and tests:

    let idle = builder.add_mode("idle", ModeKind::Initial)?;
    let active = builder.add_mode("active", ModeKind::Normal)?;
    builder.in_mode(idle, |builder| {
        builder.add_reaction(Some("enter_idle"))
            .with_reset_trigger()
            .with_reaction_fn(|ctx, state, ()| {
                state.model.reset_idle();
            })
            .finish()?;
        Ok(())
    })?;

This lower-level API is not the primary documentation surface, but it provides a stable target for macro expansion.

## Runtime Semantics

At startup, each modal reactor activates its initial mode. Components outside modes are active immediately. Startup reactions outside modes run at program startup as they do today. Startup reactions inside a mode run at most once, when that mode scope is first activated. Shutdown reactions inside modes run when the reactor shuts down if their enclosing mode scope has been activated at least once, even if the mode is inactive at shutdown.

When a reaction sets a mode transition at tag `(t, m)`, the current mode remains active for the rest of that tag. The target mode can first produce reactions at a future tag. If a reset activation produces an immediate reset reaction or a zero-offset timer, schedule it at `(t, m + 1)`. If no immediate local event exists, the target mode waits for its next local event or external trigger.

Multiple reactions may request transitions in the same tag. The scheduler uses existing deterministic reaction order; the last executed transition request for a modal reactor wins. If a reaction declares two transition effects and sets both, the last `.set(ctx)` call inside that reaction wins for that reactor.

For reset transitions, the runtime discards pending local events owned by the target scope and all recursively contained scopes, resets contained modal reactors to their initial modes, clears activation history for contained startup where appropriate, schedules reset-triggered reactions, and schedules initial timer offsets for the entered scope. Rust state is reset only by user-written reset reactions.

For history transitions, the runtime preserves queued local events. While the scope was inactive, its local clock did not advance, so each queued local event keeps the same remaining local delay. On reactivation, the runtime maps the next local event to global time based on the reactivation tag.

Mode-local logical actions, timers, and delayed connections are scheduled in local time. Root-scope actions, timers, and connections keep current global behavior.

External physical actions require careful treatment because physical events happen in wall-clock time, not logical local time. For the first full implementation, physical actions declared in a mode should be accepted but their incoming events should be delivered only if their enclosing mode is active at the event tag; they are not suspended and replayed by history. This behavior must be documented and tested. If this proves surprising during implementation, record the discovery and consider rejecting mode-local physical actions until a better semantic model is designed.

## Reference Test Porting

Use the Lingua Franca C modal-model tests only as an external behavior corpus for this ExecPlan. Do not mention Lingua Franca, LF, or the upstream test suite in Boomerang implementation comments, public API docs, diagnostics, user book pages, or test names. The Boomerang tests should be written as native Rust integration tests with direct assertions over observed output and state.

Port or adapt this first-wave set because each file covers a distinct semantic area:

- `Count3Modes.lf` becomes `boomerang/tests/modal_count_3_modes.rs`. It proves the smallest useful reset cycle through three sibling modes and verifies that only the active mode responds to each trigger.
- `MixedReactions.lf` becomes `boomerang/tests/modal_mixed_reactions.rs`. It proves deterministic ordering between root-scope reactions and mode-scope reactions, and it proves that a transition requested at a tag does not make the target mode run at that same tag.
- `ModalStateReset.lf` becomes `boomerang/tests/modal_reset_reactions.rs`, adapted to Boomerang's Rust state model. It should not implement automatic `reset state` syntax; instead, it proves that a `(reset)` reaction runs on reset entry and can explicitly restore user state.
- `ModalTimers.lf` becomes `boomerang/tests/modal_timers.rs`. It proves that timers declared inside modes are suspended while inactive, reset on reset entry, and resumed according to local time on history entry.
- `ModalActions.lf` becomes `boomerang/tests/modal_actions.rs`. It proves that logical actions scheduled inside modes do not fire while inactive, resume with remaining local delay on history entry, and do not leak across reset entry.
- `ModalAfter.lf` becomes `boomerang/tests/modal_delayed_connections.rs`. It proves that delayed connections inside modes use mode-local time and obey the same reset/history behavior as mode-local logical actions.
- `ModalStartupShutdown.lf` becomes `boomerang/tests/modal_startup_shutdown.rs`. It proves startup, reset, and shutdown reactions in modes, including an unreachable mode whose lifecycle reactions must never run.
- `ModalNestedReactions.lf` becomes `boomerang/tests/modal_nested_reactions.rs`. It proves that reactions and connections indirectly nested in an inactive mode through a child reactor are disabled.
- `ModalCycleBreaker.lf` becomes `boomerang/tests/modal_cycle_breaker.rs`. It proves that the static dependency graph and cycle analysis account for mutually exclusive mode scopes and do not reject a model only because inactive-mode edges would form a cycle if all modes were flattened together.
- `ModalMultiportBank.lf` becomes `boomerang/tests/modal_multiport_bank.rs`. It proves that mode-scoped reactor banks and multiport connections route only through the active branch. If port-bank support is temporarily unstable while modal scheduling is being built, first port `ModalMultiport.lf` as `boomerang/tests/modal_multiport.rs`, then replace or extend it with `ModalMultiportBank.lf` before final acceptance.

Do not port these in the first wave unless a failure points directly at them: `BanksCount3ModesSimple.lf`, `BanksCount3ModesComplex.lf`, `BanksModalStateReset.lf`, `ConvertCaseTest.lf`, `MultipleOutputFeeder_2Connections.lf`, and `MultipleOutputFeeder_ReactionConnections.lf`. They are useful second-wave regression tests, but their first-wave value is covered by the files above or by Boomerang-native tests. Do not directly port `ResetStateVariableOfTypeTime.lf` or `ResetStateVariableWithParameterizedValue.lf`; those depend on state reset syntax that this plan intentionally does not add. Cover their useful behavior with explicit reset reactions over Rust state instead.

## Performance Design

The performance target is that non-modal programs pay only small constant overhead, and modal programs do not scan inactive modes at every tag.

Static graph data should be computed once in `boomerang_builder/src/env/build.rs` and stored in `boomerang_runtime/src/env/mod.rs`. Every reaction, action, timer, port connection, and reactor instance gets a compact `ScopeKey`. Root scope is represented by a sentinel or `ScopeKey::from(0)`. Each scope stores its parent scope, owning reactor, optional mode key, initial child mode if it is a modal group, and precomputed descendants for reset. For active checks, store a precomputed ancestor chain or compact bitset. If the number of scopes is small enough to fit in a machine word, use an inline mask; otherwise store ranges over a flattened scope tree.

The scheduler should avoid per-tag allocation. Add reusable buffers to `Scheduler` for transition requests, reset scopes, and reaction filtering. Do not allocate a new vector while iterating levels. Existing WIP added reusable buffers; keep that idea but move it behind full scope semantics.

Use per-scope local event queues. A local event stores a local tag, an event payload, the owning scope, and a generation number. Each active scope has at most one frontier entry in a global frontier heap: the global tag corresponding to the head of that scope's local queue. When a local queue changes or a scope is activated, push a new frontier entry with that scope's queue epoch. Stale frontier entries are skipped by comparing epoch and active generation. This makes finding the next global event proportional to the number of active scopes with pending events, not all inactive events.

Reset should invalidate old events by incrementing a scope generation and clearing that scope's local queues. If clearing a large queue is too expensive, replace the queue with an empty queue and let old heap entries expire by generation. The reset cost should be proportional to the events owned by the reset subtree, not the entire model.

History should not rewrite queued local tags. On deactivation, record the local time at suspension. On activation, record the global tag where the scope resumed and map local queue heads to global tags lazily as `activation_global_tag + (event_local_tag - activation_local_tag)`. This avoids touching every queued event on history transition.

Reaction gating remains necessary because root-scope ports can trigger reactions inside inactive modes. The check must be a cheap scope-active check at execution time and when extending downstream levels from set ports. It must not perform name lookup, heap allocation, or scan ancestor chains in the hot loop.

Benchmark requirements:

- `cargo bench -p boomerang --bench ping_pong` should show no meaningful regression for non-modal models.
- A new modal benchmark should include many inactive modes each with timers/actions queued, proving inactive modes do not add per-tag cost.
- If `BOOMERANG_PROFILE=1 cargo bench -p boomerang --bench modal_modes` is run, the flamegraph should show the scheduler spending time on active event queues and reaction execution, not scanning inactive scopes.

## Plan of Work

First, remove or quarantine the minimal WIP API from the previous spike. The old `@modes(...)`, `@transition(...)`, and `Context::set_mode_name` design should not become the long-term API. If retaining compatibility temporarily helps development, put it behind private tests or feature-gated spike code and delete it before acceptance.

Next, extend `boomerang_builder` with static scopes. Add builder keys for modes and scopes, store each component's scope, and add validation rules: each modal reactor has exactly one initial mode; mode names are unique per reactor; mode effects may only target sibling modes of the current modal reactor; reset triggers are only valid inside modes; mode-local ports are rejected; direct nested mode blocks are rejected.

Then extend `boomerang_macros/src/reactor.rs` so the `#[reactor]` parser recognizes `mode! { initial name { ... } }` and `mode! { name { ... } }` statements. The parser should collect mode declarations before emitting code, then emit builder calls to create all sibling mode handles, then emit each mode body inside a scoped builder. Extend `boomerang_macros/src/reaction.rs` to parse `reset` as a trigger and `reset(mode)` / `history(mode)` / bare `mode` as mode transition effects.

After the builder and macro layers compile, add runtime static graph support in `boomerang_runtime/src/env/mod.rs`. Define `ScopeKey`, `ModeKey`, `TransitionKind`, `ModeTransitionEffect`, and maps from reactions/actions/timers/connections to scopes. Keep public exports minimal; most keys should remain runtime/builder implementation details unless user code must name them.

Next, refactor scheduling. `boomerang_runtime/src/sched.rs` currently uses a global event queue. Introduce a root global queue plus per-scope local queues or one generalized event manager that treats root as always-active local time. All public scheduler behavior must remain deterministic. Keep existing tests passing after each refactoring step by making root-scope behavior equivalent to today's behavior before enabling mode-local behavior.

Then implement transition application after each tag. Collect transition requests from triggered reactions, resolve last-wins per modal reactor, and apply reset/history activation. Activation must schedule any immediate local events at the next microstep, not in the same tag.

Finally, implement startup/shutdown/reset modal triggers and recursive reset behavior. Add tests first or alongside implementation so failures describe the missing semantics precisely.

After the feature behavior is implemented and before final acceptance, document it in the book. Add `book/src/modal-reactors.md` and link it from `book/src/SUMMARY.md`. The page must show the proposed `#[reactor]` syntax, explain mode scopes, transition effects, reset as the default, `history(mode)` transitions, `(reset)` reactions, lifecycle triggers, mode-local timers/actions/delayed connections, and the physical-action caveat. Write the page as Boomerang documentation, not as comparative documentation, and do not name the external reference language or its test suite.

If the new page introduces terms that users will encounter elsewhere, update `book/src/glossary.md` with concise entries for "mode", "mode scope", "reset transition", "history transition", and "local time". Keep the glossary entries short and link readers back to `modal-reactors.md` for examples.

## Concrete Steps

From repository root `/Users/johhug01/Source/boomerang`, start by confirming the branch and baseline:

    git branch --show-current

Expected output:

    modal-reactors2

Run the current tests before implementation begins:

    cargo test

Expected output is all current tests passing. If this fails before modal work begins, fix or record the pre-existing failure in `Surprises & Discoveries`.

Add syntax parser tests in `boomerang_macros/src/reactor.rs` and `boomerang_macros/src/reaction.rs`. Include tests for:

- two sibling `mode!` blocks, one initial;
- forward transition reference from the first mode body to the second mode;
- `-> active` and `-> reset(active)` both parse as reset;
- `-> history(idle)` parses as history;
- `(reset)` parses as a reset trigger;
- direct nested `mode!` blocks fail with a clear diagnostic.

Add builder validation tests in `boomerang_builder/src/tests.rs` for duplicate mode names, missing initial mode, multiple initial modes, transition to mode in another reactor, reset trigger outside mode, and mode-local port declarations.

Add runtime tests under `boomerang/tests/`:

- `modal_count_3_modes.rs`: adapted from `Count3Modes.lf`; a reset transition cycles through three modes and only the active mode's reaction runs.
- `modal_mixed_reactions.rs`: adapted from `MixedReactions.lf`; root reactions and mode reactions execute in deterministic order, and a newly active mode does not run in the same tag.
- `modal_reset_reactions.rs`: adapted from `ModalStateReset.lf`; reset-triggered reactions explicitly restore Rust state on reset entry.
- `modal_timers.rs`: adapted from `ModalTimers.lf`; mode-local timers suspend while inactive and reset or resume according to transition kind.
- `modal_actions.rs`: adapted from `ModalActions.lf`; a mode schedules a logical action, exits before the action fires, re-enters by history, and the action fires after the remaining local delay; the reset path discards stale pending actions.
- `modal_delayed_connections.rs`: adapted from `ModalAfter.lf`; delayed connections inside modes use mode-local time and obey reset/history behavior.
- `modal_startup_shutdown.rs`: adapted from `ModalStartupShutdown.lf`; startup inside a mode runs once on first activation, reset reactions run on reset entry, shutdown inside a previously activated mode runs at program shutdown, and unreachable modes do not run lifecycle reactions.
- `modal_nested_reactions.rs`: adapted from `ModalNestedReactions.lf`; child reactors and connections nested in inactive modes are disabled.
- `modal_cycle_breaker.rs`: adapted from `ModalCycleBreaker.lf`; cycle validation respects mutually exclusive mode scopes.
- `modal_multiport_bank.rs`: adapted from `ModalMultiportBank.lf`; modal scopes work with multiports and reactor banks. If this blocks early scheduler work, temporarily add the simpler `modal_multiport.rs` from `ModalMultiport.lf` and replace or extend it before final acceptance.

Add book documentation after the user-facing syntax is stable:

- Create `book/src/modal-reactors.md` with examples and semantics for users.
- Link the page from `book/src/SUMMARY.md`.
- Update `book/src/glossary.md` only for terms that remain useful outside the modal reactors page.
- Ensure the book and implementation do not mention the external reference language or test suite outside this ExecPlan.

After each milestone, run the narrowest useful test:

    cargo test -p boomerang_macros modal
    cargo test -p boomerang_builder modal
    cargo test -p boomerang_runtime modal
    cargo test -p boomerang modal

For the first mode-local logical-action slice, also run:

    cargo test -p boomerang --test modal_actions
    cargo test -p boomerang_runtime
    cargo test -p boomerang_builder
    cargo test -p boomerang --test modal_mixed_reactions
    git diff --check

For the mode-local timer slice, also run:

    cargo test -p boomerang --test modal_timers
    cargo test -p boomerang --test modal_actions
    cargo test -p boomerang_runtime
    cargo test -p boomerang_builder
    cargo test -p boomerang modal
    cargo test -p boomerang --test modal_mixed_reactions
    git diff --check

For the reset-trigger reaction slice, also run:

    cargo test -p boomerang --test modal_reset_reactions
    cargo test -p boomerang_runtime
    cargo test -p boomerang_builder
    cargo test -p boomerang --test modal_actions
    cargo test -p boomerang --test modal_timers
    cargo test -p boomerang modal
    cargo test -p boomerang --test modal_mixed_reactions
    git diff --check

For the modal lifecycle slice, also run:

    cargo test -p boomerang --test modal_startup_shutdown
    cargo test -p boomerang_runtime
    cargo test -p boomerang_builder
    cargo test -p boomerang --test modal_actions
    cargo test -p boomerang --test modal_timers
    cargo test -p boomerang --test modal_reset_reactions
    cargo test -p boomerang modal
    cargo test -p boomerang --test modal_mixed_reactions
    cargo test -p boomerang --test modal_nested_reactions
    git diff --check

For the delayed connection slice, also run:

    cargo test -p boomerang --test modal_delayed_connections
    cargo test -p boomerang modal
    git diff --check

For the physical-action caveat slice, also run:

    cargo test -p boomerang --test modal_physical_actions
    cargo test -p boomerang_runtime
    cargo test -p boomerang_builder
    cargo test -p boomerang --test modal_delayed_connections
    cargo test -p boomerang modal
    git diff --check

For the three-mode cycle test slice, also run:

    cargo test -p boomerang --test modal_count_3_modes
    cargo test -p boomerang modal
    git diff --check

For the book documentation slice, also run:

    mdbook build book
    rg -n "Lingua Franca|lf-lang|LF-style|LF modal" boomerang_builder boomerang_runtime boomerang_macros book/src
    rg -n "Lingua Franca|lf-lang|LF-style|LF modal" boomerang/tests --glob 'modal_*.rs'
    git diff --check

For the cycle-breaker slice, also run:

    cargo test -p boomerang --test modal_cycle_breaker
    cargo test -p boomerang_builder
    cargo test -p boomerang modal
    git diff --check

For the modal multiport-bank slice, also run:

    cargo test -p boomerang --test modal_multiport_bank
    cargo test -p boomerang modal
    git diff --check

Before completion, run:

    cargo fmt --check
    cargo test
    cargo bench -p boomerang --bench ping_pong
    cargo bench -p boomerang --bench modal_modes
    mdbook build book
    rg -n "Lingua Franca|lf-lang|LF-style|LF modal" boomerang_builder boomerang_runtime boomerang_macros book/src
    rg -n "Lingua Franca|lf-lang|LF-style|LF modal" boomerang/tests --glob 'modal_*.rs'

The `mdbook build book` command should complete without errors. If `mdbook` is not installed in the local environment, record that in `Progress` and run it in CI or another environment before final merge. The `rg` commands should return no matches; they intentionally exclude this ExecPlan and older non-modal test ports because provenance for the modal test corpus belongs here.

## Validation and Acceptance

The feature is accepted only when a user can write one `#[reactor]` function with `mode! { initial ... }` and `mode! { ... }` blocks, declare timers/actions/reactions inside those blocks, transition with reset and history effects, and observe correct local-time behavior.

Behavioral acceptance:

When a reaction inside `idle` sets `active` at tag `(t, m)`, no reaction inside `active` executes at `(t, m)`. If `active` has a zero-offset timer or reset reaction, its earliest reaction occurs at `(t, m + 1)`.

When a pending local action is scheduled in a mode and the mode is exited, that action does not fire while the mode is inactive. If the mode is re-entered by history, the action fires after the remaining local delay. If the mode is re-entered by reset, the action is discarded.

When a parent mode is reset, contained modal reactors return to their initial modes and their pending local events are discarded.

When a startup reaction is inside a mode, it runs at most once on first activation. When a shutdown reaction is inside a mode, it runs at program shutdown if its mode has ever been active.

Performance acceptance:

Non-modal `ping_pong` benchmark results should stay within normal noise of the baseline. If the median changes significantly, investigate before merging. The modal benchmark should demonstrate that adding many inactive modes does not increase per-tag scheduler cost linearly with all inactive events.

Documentation acceptance:

The book contains a linked `book/src/modal-reactors.md` page that explains modal syntax and semantics as Boomerang features. Running `mdbook build book` succeeds where `mdbook` is available. Running `rg -n "Lingua Franca|lf-lang|LF-style|LF modal" boomerang_builder boomerang_runtime boomerang_macros book/src` and `rg -n "Lingua Franca|lf-lang|LF-style|LF modal" boomerang/tests --glob 'modal_*.rs'` returns no matches, proving that modal implementation details and user docs do not name the external reference language or test suite.

## Idempotence and Recovery

The implementation should be done in small commits. Each milestone should leave the workspace compiling or should explicitly mark a temporary expected failure in `Progress`. If a parser or builder refactor fails, it is safe to revert only that milestone commit and keep this ExecPlan. Do not use destructive git commands unless explicitly requested by the user.

Generated benchmark artifacts under `target/` should not be committed. Runtime or tool logs under `logs/` should not be committed unless a future plan explicitly says otherwise.

If the local-time scheduler refactor becomes too large, split it behind a new internal `EventManager` type while preserving the old root-scope behavior. Keep all existing non-modal tests passing before adding modal local-time behavior.

## Artifacts and Notes

Prior WIP checkpoint:

    2026-07-06: Current branch `modal-reactors` was committed as `964ef3b feat: add modal reactors spike` before creating `modal-reactors2`.

The old WIP is useful for these ideas:

- store current mode state per reactor;
- defer transition application until after a tag;
- use reusable scheduler buffers;
- validate mode ownership in the builder.

The old WIP should not be copied directly for these API decisions:

- `@modes(...)` as the main syntax;
- `@transition(...)` without reset/history kind;
- `Context::set_mode_name(&'static str)`;
- treating modes as reaction filters only.

## Interfaces and Dependencies

No new external crates are expected. Use existing `slotmap` for builder keys and `boomerang_tinymap` for runtime maps. If a compact bitset is needed, first evaluate whether a small inline representation is enough before adding a dependency.

In `boomerang_builder/src/reactor.rs`, define builder-side keys and data:

    slotmap::new_key_type! { pub struct BuilderModeKey; }
    slotmap::new_key_type! { pub struct BuilderScopeKey; }

    pub enum ModeKind {
        Initial,
        Normal,
    }

    pub enum TransitionKind {
        Reset,
        History,
    }

In `boomerang_builder/src/env/mod.rs`, add mode and scope builders:

    pub struct ScopeBuilder {
        pub parent: Option<BuilderScopeKey>,
        pub reactor_key: BuilderReactorKey,
        pub mode_key: Option<BuilderModeKey>,
    }

    pub struct ModeBuilder {
        pub name: String,
        pub reactor_key: BuilderReactorKey,
        pub scope_key: BuilderScopeKey,
        pub kind: ModeKind,
    }

ReactorBuilderState should gain scoped builder support:

    pub fn add_mode(&mut self, name: &str, kind: ModeKind) -> Result<BuilderModeKey, BuilderError>;
    pub fn in_mode<R>(&mut self, mode: BuilderModeKey, f: impl FnOnce(&mut Self) -> Result<R, BuilderError>) -> Result<R, BuilderError>;

The exact lifetime shape may differ; the important interface is that macro-generated code can enter a mode scope and existing `timer!`, `reaction!`, child reactor, action, and connection builder calls record that scope.

In `boomerang_builder/src/reaction.rs`, add mode transition effects:

    pub struct BuilderModeEffect {
        pub target: BuilderModeKey,
        pub transition: TransitionKind,
    }

    pub trait PartialReactionBuilderField {
        // Existing behavior remains for ports/actions.
    }

    impl PartialReactionBuilderField for BuilderModeEffect { ... }

Add `with_reset_trigger()` for reset reactions. Reset triggers are not action keys; they are mode-scope lifecycle triggers lowered into runtime reset reactions.

In `boomerang_runtime/src/env/mod.rs`, define runtime keys and graph fields:

    tinymap::key_type! { pub ScopeKey }
    tinymap::key_type! { pub ModeKey }

    pub enum TransitionKind {
        Reset,
        History,
    }

    pub struct ScopeInfo {
        pub parent: Option<ScopeKey>,
        pub reactor: ReactorKey,
        pub mode: Option<ModeKey>,
        pub descendants: Vec<ScopeKey>,
    }

    pub struct ModeInfo {
        pub reactor: ReactorKey,
        pub scope: ScopeKey,
        pub name: String,
        pub initial: bool,
    }

ReactionGraph should include scope ownership maps:

    pub scopes: TinyMap<ScopeKey, ScopeInfo>;
    pub modes: TinyMap<ModeKey, ModeInfo>;
    pub reaction_scopes: TinySecondaryMap<ReactionKey, ScopeKey>;
    pub action_scopes: TinySecondaryMap<ActionKey, ScopeKey>;
    pub action_is_logical: TinySecondaryMap<ActionKey, bool>;
    pub port_connection_scopes: Vec<ConnectionScopeInfo>;
    pub reset_reactions: TinySecondaryMap<ScopeKey, Vec<LevelReactionKey>>;
    pub startup_reactions: TinySecondaryMap<ScopeKey, Vec<LifecycleReaction>>;
    pub shutdown_reactions_by_scope: TinySecondaryMap<ScopeKey, Vec<LifecycleReaction>>;

where `LifecycleReaction` stores both the level/reaction pair and the internal startup or shutdown action key that should be present when the lifecycle reaction runs:

    pub struct LifecycleReaction {
        pub reaction: LevelReactionKey,
        pub action: ActionKey,
    }

In `boomerang_runtime/src/context.rs`, replace string mode scheduling with typed transition recording:

    pub(crate) struct TriggerRes {
        pub scheduled_actions: Vec<(ActionKey, Tag)>,
        pub scheduled_shutdown: Option<Tag>,
        pub scheduled_mode: Option<ModeTransitionRequest>,
    }

    pub struct ModeEffectRef {
        target: ModeKey,
        transition: TransitionKind,
    }

    impl ModeEffectRef {
        pub fn set(&self, ctx: &mut Context) { ... }
    }

In `boomerang_runtime/src/sched.rs`, introduce an event manager:

    struct EventManager {
        root_queue: BinaryHeap<ScheduledEvent>,
        scope_queues: TinySecondaryMap<ScopeKey, LocalEventQueue>,
        active_frontier: BinaryHeap<ScopeFrontierEntry>,
    }

The exact names can change, but the design must preserve root-scope behavior and make inactive local queues dormant without scanning them on each global tag.

Change log: 2026-07-06 / Codex: replaced minimal modal reactors plan with full-semantics plan after user confirmed full modal semantics, reset default, and branch serialization workflow.

Change log: 2026-07-06 / Codex: added book documentation requirements, constrained external reference mentions to this ExecPlan, and selected the minimal upstream modal tests to port or adapt.

Change log: 2026-07-06 / Codex: started implementation with typed builder/runtime transition effect plumbing, reset-trigger parser support, and focused tests while keeping spike compatibility.

Change log: 2026-07-06 / Codex: corrected the proposed structural syntax to valid Rust `mode!` statements after discovering that bare `initial mode` blocks are rejected before attribute macro expansion, and added the first parser/integration implementation slice for that syntax.

Change log: 2026-07-06 / Codex: added the first static scope metadata layer for reactors, modes, actions, ports, and reactions, plus validation for mode-local ports and reset triggers outside mode scopes.

Change log: 2026-07-06 / Codex: extended mode scope ownership to child reactor instances and synthetic delayed/physical connection helper reactors, and recorded focused verification for that slice.

Change log: 2026-07-06 / Codex: removed spike modal syntax and unconditional static transitions, converted modal integration tests to structural `mode!` blocks with typed `.set(ctx)` effects, and documented the verification.

Change log: 2026-07-06 / Codex: added active scope checks to the scheduler, threaded mode scope through macro-generated child reactor builds, and added a nested inactive-child startup regression test.

Change log: 2026-07-06 / Codex: changed typed mode effects to resolve to runtime `ModeKey`s at lowering time and removed the remaining name-based transition target path.

Change log: 2026-07-07 / Codex: added the first scoped event manager implementation for mode-local logical actions, reset/history transition application for pending action queues and child mode reset, `modal_actions` integration tests, and focused verification transcripts.

Change log: 2026-07-07 / Codex: extended scoped local-time scheduling to timers by recording timer startup actions separately from lifecycle startup actions, rescheduling timer startup events on reset entry, preserving startup-tag behavior for initially active scopes, and adding `modal_timers` integration tests.

Change log: 2026-07-07 / Codex: implemented reset-triggered reactions by recording reset reactions per runtime scope, scheduling reset-subtree reset reactions at the next microstep after reset entry, adding modal reset reaction tests, and documenting the startup/reset distinction.

Change log: 2026-07-07 / Codex: implemented modal lifecycle startup/shutdown behavior with scoped lifecycle reaction metadata, per-scope activation history, timeout-safe terminal shutdown filtering, and `modal_startup_shutdown` integration tests.

Change log: 2026-07-07 / Codex: added delayed-connection integration coverage showing that scoped helper reactors and local logical-action queues already provide reset/history delayed delivery semantics.

Change log: 2026-07-07 / Codex: added action-kind metadata and physical-action modal coverage so mode-scoped physical events stay global, are filtered by active scope at their event tag, and are not replayed by history.

Change log: 2026-07-07 / Codex: added the three-mode modal cycle integration test using native Boomerang actions to prove active-mode-only reaction behavior across repeated reset transitions.

Change log: 2026-07-07 / Codex: added modal reactor book documentation, glossary entries, and a small mdBook config compatibility fix required for the documented build command to pass locally.

Change log: 2026-07-07 / Codex: made builder reaction dependency analysis skip edges between mutually exclusive sibling mode scopes and added a modal cycle-breaker integration test.

Change log: 2026-07-07 / Codex: added modal multiport-bank integration coverage and recorded that banked child reactors plus multiport routing work inside an active mode with the existing scope propagation.
