# Modal Reactor Scheduler Performance

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Reference: `.agent/PLANS.md` in the repository root. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

Improve the performance shape of the modal reactor scheduler after the full modal semantics implementation. A user should be able to build modal models with many modes, frequent reset or history transitions, and root ports that fan out to modal reactions without paying avoidable allocation or cache-miss costs in the scheduler hot path. The work starts by adding benchmark coverage for the missing performance scenarios, then uses those benchmarks to drive focused scheduler and runtime data-structure improvements.

The observable result is not a new user-facing API. The observable result is that existing modal behavior still passes all tests, new benchmarks exercise transition-heavy and fanout-heavy modal models, and the scheduler no longer allocates in steady-state root action scheduling after warmup. The primary commands are `cargo test`, `cargo bench -p boomerang --bench ping_pong`, `cargo bench -p boomerang --bench modal_modes`, and a new modal scheduler performance benchmark introduced by this plan.

## Progress

- [x] (2026-07-07 09:26Z) Created this standalone performance ExecPlan after reviewing the implemented modal scheduler and existing benchmarks.
- [x] (2026-07-07 09:35Z) Added `boomerang/benches/modal_scheduler_perf.rs`, registered it in `boomerang/Cargo.toml`, and verified `cargo test -p boomerang --bench modal_scheduler_perf` passes for transition churn, inactive modal port fanout, and reset-subtree groups.
- [x] (2026-07-07 09:49Z) Removed per-event `Vec` allocation from root/global scheduled action events by changing `ScheduledEvent` from `action_values: Vec<ScheduledActionValue>` to `action_value: Option<ScheduledActionValue>` and passing `None` for events that are never rebased.
- [x] (2026-07-07 09:57Z) Added `boomerang/tests/scheduler_alloc.rs`, a test-local counting allocator that verifies 1,024 warmed `Scheduler::next()` calls in a non-modal root logical action loop perform zero heap allocations.
- [x] (2026-07-07 09:57Z) Removed a per-action-read allocation discovered by the allocation test by replacing `ActionStore::clear_older_than`'s `BTreeMap::split_off` pruning with allocation-free `pop_first` pruning of older offsets.
- [x] (2026-07-07 10:02Z) Captured Criterion checkpoint results for `ping_pong`, `modal_modes`, and `modal_scheduler_perf` after the first allocation patches. These are not untouched-branch baselines, but they are the comparison point for later modal-specific refactors.
- [x] (2026-07-07 10:10Z) Removed the per-transition second deduplication `Vec` in `Scheduler::process_tag` by collapsing last-wins mode transition requests directly into the reusable `transition_buffer`.
- [x] (2026-07-07 10:10Z) Used `EventManager`'s cached active scope state for current-level modal reaction gating and filtered inactive modal port-trigger reactions before inserting them into later reaction levels.
- [x] (2026-07-07 10:20Z) Added `ModalScheduleIndex` to `ReactionGraph`, populated it during `EnvBuilder::into_runtime_parts`, and used it for reset-subtree clearing, reset timer startup scheduling, reset reactions, startup reaction lookup, and shutdown scheduling.
- [x] (2026-07-07 13:23Z) Split hot active-scope state from cold per-scope event queues by replacing `ScopeTimeState` with separate active/startup flag maps, `ScopeClockState`, and per-scope `EventQueue` storage in `EventManager`.
- [x] (2026-07-07 13:45Z) Ran `modal_scheduler_perf`, `modal_modes`, and two `ping_pong` passes after the state-layout split. Modal fanout improved, transition churn stayed within noise, reset-subtree moved backward by about 2-3%, and `ping_pong` reported a guard regression with noisy repeat data.
- [x] (2026-07-07 13:58Z) Profiled the regressing benchmark areas with pprof and macOS `sample`. The modal reset hot path is dominated by `EventQueue::push_event_inner` zeroing dense `ReactionSet` storage, while `ping_pong` stays on the non-modal root event queue path and does not show hot modal scope-state frames.
- [x] (2026-07-07 14:07Z) Implemented the first diagnosis solution: pooled `ReactionSet`s are now cleared once when returned to a pool, and `EventQueue::next_reaction_set` trusts that pool invariant instead of clearing on checkout.
- [x] (2026-07-07 14:07Z) Re-ran focused validation and benchmarks after the single-clear pooling change. Runtime tests, modal tests, modal scheduler benchmark smoke tests, `reset_subtree`, and `ping_pong` passed; `ping_pong` improved by about 8-14% and the reset-subtree regression was no longer reported.
- [x] (2026-07-07 14:08Z) Re-ran final hygiene after updating this ExecPlan; `cargo fmt --check` and `git diff --check` passed, with only the existing stable-rustfmt warnings.
- [x] (2026-07-07 14:32Z) Wrapped the current state-layout and `ReactionSet` pooling milestone as accepted for focused validation. The milestone is not the whole ExecPlan closeout; full workspace tests and the complete benchmark set remain reserved for final validation after remaining scheduler cleanup.
- [x] (2026-07-07 14:46Z) Trialed converting `EventManager`'s hot boolean scope-state maps from `TinySecondaryMap<ScopeKey, bool>` to `tinymap::KeySet<ScopeKey>`, rejected the scheduler conversion after broad Criterion regressions, and kept only the tested `KeySet::remove` primitive.
- [x] (2026-07-07 15:12Z) Bypassed redundant per-reaction `ModeFilter` scans in the scheduler hot path by relying on static reaction scope activity, while keeping a debug-only invariant that existing mode filters match the static mode scope.
- [x] (2026-07-08 11:50Z) Re-ran the PR-readiness validation set after the scheduler module split: `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test`, `mdbook build book`, `cargo test -p boomerang --bench modal_modes`, `cargo bench -p boomerang --bench modal_scheduler_perf`, `cargo bench -p boomerang --bench modal_modes`, and `git diff --check` all passed. Criterion reported improvements or no detected change across the measured modal scheduler groups.

## Surprises & Discoveries

- Observation: Existing benchmark coverage does not exercise the highest-risk modal scheduler paths.
  Evidence: `boomerang/benches/ping_pong.rs` covers non-modal root action scheduling. `boomerang/benches/modal_modes.rs` covers many inactive local timer queues while root work advances. `boomerang/benches/port_bank.rs` covers fanout and banks without modal gating. There is no benchmark that transitions every tick, resets a large modal subtree, or sends root port events to many inactive modal reactions.

- Observation: The current `ScheduledEvent` shape allocates for root/global action events even though rebasing metadata is only useful for mode-local queues.
  Evidence: `boomerang_runtime/src/event.rs` stores `ScheduledEvent::action_values` as a `Vec<ScheduledActionValue>`. `boomerang_runtime/src/sched.rs` creates that vector with `action_value.into_iter().collect()` when pushing a new action event, then root/non-modal pop paths discard the metadata when constructing `ReadyEvent`.

- Observation: Transition handling currently builds temporary vectors and scans all scopes or actions in several places.
  Evidence: `EventManager::reset_scope_subtree`, `sync_active_scopes`, `schedule_startup_reactions`, `schedule_reset_timer_startups`, `schedule_reset_reactions`, and `schedule_shutdown_at` in `boomerang_runtime/src/sched.rs` allocate or collect temporary `Vec`s. This is tolerable for rare transitions, but not for a model that transitions on every logical tick.

- Observation: Reaction gating is currently computed after downstream port triggers have already been inserted into the reaction set.
  Evidence: `Scheduler::process_tag` filters candidate reactions by active scope before executing a level, but the downstream reactions from set ports are extended into `next_levels` without modal filtering first. A root port that triggers many inactive modal reactions still pays insertion and later filtering costs.

- Observation: Root/global action events do not need rebasing metadata, but mode-local events still need one inline `ScheduledActionValue` while queued.
  Evidence: `cargo test -p boomerang_runtime event` passed after changing `ScheduledEvent` to store `action_value: Option<ScheduledActionValue>`, and `cargo test -p boomerang --bench modal_scheduler_perf` passed all transition, fanout, and reset-subtree benchmark smoke cases.

- Observation: Rebasing mode-local event metadata still drains the binary heap into a temporary `Vec` and rebuilds it.
  Evidence: `EventQueue::rebase_action_values` in `boomerang_runtime/src/sched.rs` still uses `self.event_queue.drain().collect::<Vec<_>>()`. The root/global allocation patch intentionally leaves this transition-path allocation for the later queue/index refactor because stable heap mutation while preserving heap invariants is not exposed as a simple safe hot-path operation here.

- Observation: Reading a present action value in a same-offset logical action loop allocated once per scheduler step before the action-store pruning change.
  Evidence: The first version of `boomerang/tests/scheduler_alloc.rs` failed with `steady-state non-modal root action scheduling allocated 1024 times after warmup; first kind=1, first size=808`. The source was `ActionStore::clear_older_than`, whose `BTreeMap::split_off` call allocates even when it is only pruning older offsets.

- Observation: Filtering inactive modal port triggers before insertion removes the main fanout pathology.
  Evidence: `cargo bench -p boomerang --bench modal_scheduler_perf` after the filtering patch improved `inactive_port_fanout/modes/256` from a 7.1895 ms checkpoint median estimate to 2.8080 ms, and `inactive_port_fanout/modes/1024` from 5.3171 ms to 1.7425 ms. Criterion reported improvements of about 61% and 67% respectively.

- Observation: Cached active-scope gating and in-place transition dedup also improve transition-heavy cases, even before the larger subtree index refactor.
  Evidence: The same benchmark run improved `transition_churn/reset/100_000` from 31.259 ms to 28.087 ms and `transition_churn/history/100_000` from 27.746 ms to 24.916 ms.

- Observation: The flattened modal schedule index moves reset-subtree scaling and transition churn, while fanout stays at the improved filtered level.
  Evidence: After adding `ModalScheduleIndex`, `cargo bench -p boomerang --bench modal_scheduler_perf` improved `reset_subtree/medium` from 35.031 ms to 30.304 ms and `reset_subtree/large` from 243.94 ms to 221.90 ms relative to the post-filtering run. It also improved `transition_churn/reset/100_000` from 28.087 ms to 23.560 ms.

- Observation: The state-layout split can be made without changing modal scheduler behavior.
  Evidence: After replacing the combined `ScopeTimeState` with separate `scope_active`, `scope_ever_active`, `scope_startup_fired`, `scope_clocks`, and `scope_queues` maps, `cargo test -p boomerang_runtime`, `cargo test -p boomerang modal`, and `cargo test -p boomerang --bench modal_scheduler_perf` all passed.

- Observation: The state-layout split improves inactive fanout cases but is not an unqualified performance win yet.
  Evidence: `cargo bench -p boomerang --bench modal_scheduler_perf` on 2026-07-07 13:45Z reported improvements for `inactive_port_fanout/modes/32`, `/256`, and `/1024`, with `/1024` improving by about 18.69% versus local Criterion history. The same run reported regressions for `reset_subtree/medium/64` and `/large/512` of about 2.81% and 2.23%, respectively.

- Observation: The non-modal guard benchmark is currently noisy and cannot be used to claim clean progress after the state-layout split.
  Evidence: The first `cargo bench -p boomerang --bench ping_pong` pass on 2026-07-07 13:45Z reported regressions of about 4.94%, 5.32%, and 4.25% for sizes 100, 10,000, and 1,000,000. A repeat pass reported the 100 case as within noise, but the 10,000 case had four high-severe outliers and the 1,000,000 case still regressed by about 5.48%. A process check showed no active Cargo or rustc benchmark process and only idle CodeStory repair processes.

- Observation: The reset-subtree profile points at dense reaction-set clearing/initialization, not the new active flag maps.
  Evidence: `/tmp/modal_reset_large.sample.txt` captured `reset_subtree/large/512` with 2,869 samples under `EventQueue::push_event_inner`, including 2,531 samples in `_platform_memset` and 332 in `__bzero`. State-split-specific frames were small by comparison: `local_to_global` had 20 samples, `EventManager::refresh_frontier` 18, `EventManager::peek_frontier_tag` 7, and `TinySecondaryMap::insert` 6. This matches `ReactionSet::new` and `ReactionSet::clear`, which allocate or clear `tinymap::KeySet` storage for every level at full reaction capacity.

- Observation: The `ping_pong` profile does not show the modal state split as a direct hot path.
  Evidence: `/tmp/ping_pong_1m.sample.txt` captured `ping_pong/1_000_000` with 1,857 samples in `Scheduler::process_tag`, 488 in `Scheduler::next`, 292 in `EventQueue::push_event_inner`, 183 in `tracing::span::Span::record_all`, and 61 in `OffsetBucket::insert`. The profile did not show hot `scope_active`, `scope_clocks`, or `peek_frontier_tag` frames, consistent with the `!has_local_scopes` fast path still using only the root event queue.

- Observation: The redundant `ReactionSet` clear was real and affected the non-modal guard path as well as modal reset noise.
  Evidence: After changing pooled `ReactionSet`s to clear only on return, `cargo bench -p boomerang --bench ping_pong` reported `ping_pong/100` at 14.259 us, `ping_pong/10_000` at 1.1818 ms, and `ping_pong/1_000_000` at 116.74 ms, with Criterion reporting improvements of about 7.79%, 13.89%, and 13.32%. Focused `reset_subtree` rerun reported no detected change for small, medium, or large, and the large case tightened to 229.06 ms rather than repeating the earlier noisy 244.16 ms profile-pass result.

- Observation: The three hot scope-state boolean maps can be represented more compactly as bitsets.
  Evidence: `boomerang_tinymap::TinySecondaryMap<K, V>` stores `Vec<Option<V>>` plus a present-value count, while `boomerang_tinymap::KeySet<K>` stores a `FixedBitSet`. The current `EventManager` uses `TinySecondaryMap<ScopeKey, bool>` for `scope_active`, `scope_ever_active`, and `scope_startup_fired`; these maps are logically sets of scopes where the flag is true. The existing `KeySet` API supports insertion and indexing, but it does not yet expose a single-key removal or `set(key, bool)` operation needed for `scope_active` deactivation.

- Observation: Directly replacing the hot scope-state boolean maps with `KeySet<ScopeKey>` is smaller but slower for current scheduler access patterns.
  Evidence: After adding `KeySet::remove`, converting `EventManager` flags to `KeySet`, and passing focused functional checks, `cargo bench -p boomerang --bench modal_scheduler_perf` on 2026-07-07 14:46Z reported regressions in every `transition_churn` case, including about 7.69% slower for `transition_churn/reset/100_000` and about 7.92% slower for `transition_churn/history/100_000`. The same run reported inactive fanout regressions of about 19.76% for 256 modes and about 24.57% for 1024 modes. `reset_subtree` was neutral, so the regression is concentrated in frequent flag reads and writes rather than reset clearing.

- Observation: Runtime `ModeFilter` checks are redundant for the current builder API.
  Evidence: `ReactionBuilder::in_mode_scope` is the only code path that populates `enabled_modes`, and it also records `scope_mode`. Builder lowering turns `scope_mode` into `ReactionGraph::reaction_scopes`, and the scheduler already gates reactions on `EventManager::scope_active(scope_key)`. After bypassing the `Store::current_mode` lookup and `ModeFilter::allows` vector scan, `cargo test -p boomerang_runtime`, `cargo test -p boomerang modal`, and `cargo test -p boomerang --bench modal_scheduler_perf` passed. `cargo bench -p boomerang --bench modal_scheduler_perf` then reported improvements in all transition-churn cases and large inactive-fanout cases.

## Decision Log

- Decision: Add benchmark and allocation coverage before changing scheduler internals.
  Rationale: The current implementation already passed semantic tests and a limited inactive-mode benchmark. New benchmark coverage makes performance regressions and improvements visible, and it keeps structural scheduler changes honest.
  Date/Author: 2026-07-07 / Codex.

- Decision: Create a new benchmark file named `boomerang/benches/modal_scheduler_perf.rs` rather than expanding `modal_modes.rs`.
  Rationale: `modal_modes.rs` is a focused dormant-local-queue benchmark. The new work needs several targeted scenarios with different model shapes and measurements. Keeping a separate benchmark keeps each file readable and makes command output easier to interpret.
  Date/Author: 2026-07-07 / Codex.

- Decision: Treat "hot path" as the repeated scheduler work in `Scheduler::next`, `EventManager::push_action_event`, event popping, reaction gating, downstream port trigger propagation, and transition application when transitions happen every tick.
  Rationale: A transition-heavy modal model can make transition application part of normal steady-state execution, so transition code must not be dismissed as purely cold setup work.
  Date/Author: 2026-07-07 / Codex.

- Decision: Favor flattened or dense indexed data over per-call tree walks and temporary vectors.
  Rationale: Boomerang runtime keys are compact indexes. Using arrays, ranges, and `TinySecondaryMap` lookups keeps scheduler state cache-friendly and avoids allocator traffic during repeated scheduling.
  Date/Author: 2026-07-07 / Codex.

- Decision: Store at most one `ScheduledActionValue` inline per queued event and only for mode-local queues.
  Rationale: Root and global-time scheduled action events are never rebased, so carrying a heap-allocated vector for them creates allocator work in the scheduler's common action scheduling path. Mode-local events still need rebasing metadata when a suspended history mode is re-entered.
  Date/Author: 2026-07-07 / Codex.

- Decision: Treat action-store pruning as part of the hot path covered by this performance plan.
  Rationale: `Scheduler::next` triggers reactions that commonly call `ActionRef::is_present` or `get_value_at`, and those calls go through `ActionStore::clear_older_than`. The allocation guard exposed that scheduler allocation goals cannot be met while action reads allocate every step.
  Date/Author: 2026-07-07 / Codex.

- Decision: Use `EventManager`'s active flags as the scheduler's modal reaction-gating source of truth.
  Rationale: `EventManager::sync_active_scopes` already updates these flags after transitions. Reading the cached flag avoids walking parent scopes through `Store::scope_is_active` for every candidate reaction. The full structure-of-arrays split remains pending, but this captures the main hot-path lookup improvement with a small semantic surface.
  Date/Author: 2026-07-07 / Codex.

- Decision: Filter only by cached active scope before inserting downstream port triggers into later levels, while retaining the full `ModeFilter` check when a reaction reaches execution.
  Rationale: Scope activity captures the expensive inactive-modal fanout case and does not require borrowing `Store` while iterating set ports. Keeping `ModeFilter` at execution preserves existing semantics until a separate review proves it is redundant.
  Date/Author: 2026-07-07 / Codex.

- Decision: Build `ModalScheduleIndex` as flattened ranges on `ReactionGraph` after builder lowering, not as per-scope `Vec`s on scheduler state.
  Rationale: The index is static graph metadata: scope descendants, logical actions in a subtree, timer startup actions in a subtree, reset reactions in a subtree, startup reactions by scope, and shutdown reactions do not depend on runtime execution. Building it once keeps transition code on dense slices and avoids per-transition scans and collects.
  Date/Author: 2026-07-07 / Codex.

- Decision: Split `EventManager` scope state into separate maps rather than introduce a larger abstraction around scope records.
  Rationale: Scheduler hot-path checks only need active and ever-active flags, while event queues are larger cold structures. Separate `TinySecondaryMap` storage keeps those hot checks to direct indexed loads and preserves the existing time conversion helper methods on a small `ScopeClockState`.
  Date/Author: 2026-07-07 / Codex.

- Decision: Maintain a clear-on-return invariant for pooled `ReactionSet`s.
  Rationale: `ReactionSet::clear` walks dense per-level bitsets. Clearing again when a set is checked out from the pool repeats the same memory zeroing before any new reactions are inserted. Centralizing pool returns through `EventQueue::recycle_reaction_set` keeps the invariant local and removes the redundant clear from both root and modal event queue paths.
  Date/Author: 2026-07-07 / Codex.

- Decision: Add a separate KeySet flag-map milestone before removing redundant `ModeFilter` checks.
  Rationale: The `EventManager` state-layout milestone intentionally split hot flags from cold queue state, but using `TinySecondaryMap<ScopeKey, bool>` leaves those flags byte-and-option backed rather than bitset backed. Converting them to `KeySet<ScopeKey>` is small, independently testable, and directly aligned with the state-layout goal. It should be measured before changing the mode-filter path so any benchmark movement can be attributed cleanly.
  Date/Author: 2026-07-07 / Codex.

- Decision: Keep `KeySet::remove`, but do not use `KeySet<ScopeKey>` for `EventManager` hot flags in this implementation.
  Rationale: The direct conversion preserves behavior and reduces the conceptual shape of the data, but the relevant modal benchmarks regressed broadly. `FixedBitSet` indexing is not a win for this hot path as currently used, and memory compression is not worth slowing transition churn or inactive fanout. Keeping the new primitive is low risk and useful for future set-like callers; reverting the scheduler fields keeps the measured faster layout.
  Date/Author: 2026-07-07 / Codex.

- Decision: Bypass `ModeFilter` in release scheduler reaction gating, but keep the graph field and a debug-only invariant check.
  Rationale: Removing the runtime graph field would be a wider compatibility and serialization change. The scheduler hot path only needs to know whether the reaction's static scope is active, because the only current source of a mode filter is the same mode scope. A `debug_assert!` preserves the assumption during development without paying `Store::current_mode` and vector scan costs in optimized builds.
  Date/Author: 2026-07-07 / Codex.

## Outcomes & Retrospective

The first implementation milestone is complete: benchmark coverage exists for transition churn, inactive modal fanout, and reset-subtree scaling, root/global scheduled action events no longer allocate a per-event metadata vector, and a steady-state non-modal root action loop now has a passing zero-allocation guard after warmup. The allocation guard also exposed and drove a no-allocation pruning change in `ActionStore`. A second scheduler hot-path milestone is complete: inactive modal port fanout is filtered before insertion, current-level modal gating uses cached active flags, and transition requests are deduplicated in the reusable buffer. A third milestone is complete: `ReactionGraph` now carries a flattened modal schedule index used by reset, startup, and shutdown scheduling paths. A fourth implementation milestone is now accepted for focused validation: `EventManager` no longer stores active flags, clock fields, and per-scope queues in one combined `ScopeTimeState`, and pooled `ReactionSet`s now clear only on return. The profiling diagnosis showed that dense reaction-set clearing was masking the state-layout result; after the single-clear fix, `ping_pong` improved materially and focused `reset_subtree` no longer reports the earlier regression. The KeySet flag-map trial is complete but rejected for scheduler storage: the direct bitset conversion regressed transition churn and inactive fanout, so `EventManager` remains on `TinySecondaryMap<ScopeKey, bool>` while `KeySet::remove` stays as a tested utility. The `ModeFilter` hot-path cleanup is complete: optimized scheduler gating now relies on active static reaction scope and leaves the old filter only as debug-checked graph metadata. Before closing the full ExecPlan, run a complete final validation and benchmark pass.

## Context and Orientation

This repository is a Rust workspace. The modal scheduler code lives primarily in `boomerang_runtime/src/sched.rs`. The static runtime graph that feeds the scheduler is `ReactionGraph` in `boomerang_runtime/src/env/mod.rs`. Builder lowering that fills `ReactionGraph` lives in `boomerang_builder/src/env/build.rs`. Existing Criterion benchmarks live in `boomerang/benches/`, and benchmark binaries are registered in `boomerang/Cargo.toml` with `[[bench]]` entries.

A "scheduler hot path" means code that runs repeatedly while the model executes, often once per logical tag or once per scheduled action. Memory allocation in that path is risky because allocator work can dominate small event-processing loops and can introduce unpredictable latency. "Cache-friendly" means related values that are read together are stored in dense, predictable memory layouts, usually arrays or maps keyed by compact integer keys, so the CPU can load them efficiently.

The current modal implementation introduces `EventManager` in `boomerang_runtime/src/sched.rs`. It keeps a root event queue for normal global-time events, per-scope local queues for mode-owned logical actions and timers, and a frontier heap that exposes the next active local event. A "scope" is a runtime region represented by `ScopeKey`; each reactor has a root scope and each mode has a child scope. A "reset transition" enters a mode from local time zero and clears pending local events in the reset subtree. A "history transition" enters a mode while preserving its suspended local time and pending local events.

The existing benchmarks cover only part of this performance space. `ping_pong` catches broad non-modal scheduler regressions. `modal_modes` verifies that many inactive mode-local timer queues do not create a linear scan on every root tick. `port_bank` measures non-modal fanout and banking. This plan adds coverage for frequent modal transitions, inactive modal fanout from root ports, reset of a large modal subtree, and allocator behavior in steady-state scheduler loops.

## Plan of Work

First, add benchmark coverage. Create `boomerang/benches/modal_scheduler_perf.rs` and register it in `boomerang/Cargo.toml` with:

    [[bench]]
    name = "modal_scheduler_perf"
    harness = false

The new benchmark should use Criterion like the existing benchmarks. It should support `BOOMERANG_PROFILE=1` with `pprof::criterion::PProfProfiler`, matching the pattern in `boomerang/benches/modal_modes.rs`.

The first benchmark scenario is `transition_churn`. Build a reactor with two sibling modes and a root logical `tick` action. Each tick runs a reaction in the active mode, schedules the next tick, and requests a transition to the sibling mode. Include both reset and history cases, because reset clears and reschedules local state while history rebases suspended local state. Use iteration counts large enough to make scheduler overhead visible, for example 10,000 and 100,000 ticks.

The second benchmark scenario is `inactive_port_fanout`. Build a parent reactor that repeatedly sends a value into a child reactor input port. In the child reactor, declare many sibling modes and place reactions inside those modes that trigger on the same root-scoped input port. Keep only one mode active. This measures the cost of adding and later filtering many inactive modal reactions when a root port is set. Include fanout sizes such as 1, 32, 256, and 1024 reactions if compile time remains reasonable.

The third benchmark scenario is `reset_subtree`. Build a modal reactor where the target mode contains many child reactors or many scoped logical actions and timers. Trigger a reset transition repeatedly. This scenario measures the transition path that clears local queues, resets child modes, schedules reset reactions, and restarts timers. Include a small case for sanity and a larger case that clearly exposes scans over all scopes or all actions.

Add allocator coverage after the Criterion scenarios are in place. A simple approach is a new integration test in `boomerang/tests/scheduler_alloc.rs` with a test-only global allocator wrapper around `std::alloc::System`. Because a Rust crate can have only one global allocator per test binary, keep the allocator wrapper local to this new integration test file. The test should build a small scheduler, warm it up until internal queues and reusable buffers have capacity, reset the allocation counter, run a fixed number of `Scheduler::next()` calls, and assert that the count is zero for the non-modal root action chain. If the test proves too brittle across platforms, mark it `#[ignore]` and keep it as a documented local diagnostic, but first try to make it stable for this repository.

The allocation test now uses a two-action root logical ping-pong model. Each reaction reads the trigger action with `is_present(ctx)` before scheduling the other `()` action, so old action-store values are pruned as they would be in ordinary action-trigger code. The test warms the scheduler for 4,096 calls, counts allocations for 1,024 further calls, and asserts the count is zero.

Second, capture baseline numbers. Run the new benchmark before scheduler changes. Record concise results in this ExecPlan under `Artifacts and Notes`. Also run `ping_pong` and `modal_modes` so later changes can be compared against known non-modal and dormant-local-queue behavior.

Third, remove root/global action-event allocation. Change `boomerang_runtime/src/event.rs` so `ScheduledEvent` no longer stores `action_values: Vec<ScheduledActionValue>`. Prefer a single inline field:

    pub(crate) action_value: Option<ScheduledActionValue>

Then change `EventQueue::push_event_inner` in `boomerang_runtime/src/sched.rs` so pure reaction events at the same tag may still merge, but action events that need `action_value` metadata can remain as separate heap entries. `EventQueue::pop_next_event` already merges same-tag events, so keeping separate action-event heap entries preserves execution behavior while avoiding per-event `Vec` allocation. `EventQueue::rebase_action_values` should visit `event.action_value.as_mut()` instead of iterating a vector. Root/global action events should pass `None` because they are never rebased.

Fourth, precompute modal scope indexes during lowering. Extend `ReactionGraph` in `boomerang_runtime/src/env/mod.rs` with a compact modal index. Prefer flattened arrays and small range descriptors over nested `Vec`s per scope. Because this repository can require Rust 1.96 or newer, use `core::range::Range<usize>` directly for the range descriptors instead of carrying a local wrapper type. One concrete shape is:

    use core::range::Range;

    #[derive(Debug, Default)]
    pub struct ModalScheduleIndex {
        pub scope_descendant_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
        pub scope_descendants: Vec<ScopeKey>,
        pub scope_logical_action_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
        pub scope_logical_actions: Vec<ActionKey>,
        pub scope_timer_startup_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
        pub scope_timer_startups: Vec<(ActionKey, Tag)>,
        pub scope_reset_reaction_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
        pub scope_reset_reactions: Vec<LevelReactionKey>,
        pub scope_startup_reaction_ranges: tinymap::TinySecondaryMap<ScopeKey, Range<usize>>,
        pub scope_startup_reactions: Vec<LifecycleReaction>,
        pub all_shutdown_reactions: Vec<LifecycleReaction>,
        pub all_shutdown_actions_unique: Vec<ActionKey>,
    }

The exact names may change, but the important requirement is that transition code can answer "which scopes/actions/timers/reactions are in this subtree?" without collecting a temporary vector and without scanning unrelated scopes. Build this index after modes, scopes, actions, and reactions are known in `EnvBuilder::into_runtime_parts`, likely at the end of `build_runtime_reactions` or immediately after it. Keep the old maps only if they remain useful for debug output or simpler construction; the scheduler should use the new index.

Fifth, refactor transition helpers in `EventManager`. Replace the temporary-vector scans in `reset_scope_subtree`, `sync_active_scopes`, `schedule_startup_reactions`, `schedule_reset_timer_startups`, `schedule_reset_reactions`, and `schedule_shutdown_at` with iterations over the precomputed modal index or reusable buffers stored on `EventManager` or `Scheduler`. For `sync_active_scopes`, avoid `reaction_graph.scopes.keys().collect::<Vec<_>>()`; iterate a stable scope list from the graph or index directly. For reset, clear exactly the descendant scopes and logical actions in the reset subtree.

Sixth, split hot active-state data from cold queue data. `ScopeTimeState` currently stores flags, activation tags, epoch, and an `EventQueue` together. This makes active checks touch a large object. Refactor toward a structure-of-arrays layout inside `EventManager`, for example:

    scope_active: tinymap::TinySecondaryMap<ScopeKey, bool>
    scope_ever_active: tinymap::TinySecondaryMap<ScopeKey, bool>
    scope_startup_fired: tinymap::TinySecondaryMap<ScopeKey, bool>
    scope_clock: tinymap::TinySecondaryMap<ScopeKey, ScopeClockState>
    scope_queues: tinymap::TinySecondaryMap<ScopeKey, EventQueue>

`ScopeClockState` should contain activation and suspended local-time tags plus frontier epoch. The exact decomposition can vary, but `Scheduler::process_tag` must be able to ask whether a reaction scope is active with one or two indexed loads and no parent walk through `Store::scope_is_active`.

Seventh, trial compression of the hot scope boolean maps to bitsets. The state-layout split has already separated `scope_active`, `scope_ever_active`, and `scope_startup_fired` from cold queue state, but these fields still use `TinySecondaryMap<ScopeKey, bool>`. `TinySecondaryMap` is a sparse secondary map backed by `Vec<Option<V>>`; this is appropriate when some keys are absent or when values are larger than a flag. Here the fields are semantically sets of scopes for which a flag is true. `boomerang_tinymap::KeySet<ScopeKey>` is backed by `FixedBitSet` and is designed for this shape, but it first needs a single-key clear or set operation.

Add a method to `boomerang_tinymap/src/key_set/mod.rs`:

    pub fn remove(&mut self, key: K) {
        self.data.set(key.index(), false);
    }

or, if the implementation reads better at call sites:

    pub fn set(&mut self, key: K, value: bool) {
        self.data.set(key.index(), value);
    }

Add focused `KeySet` unit tests for clearing one key without disturbing another. Then change `EventManager` in `boomerang_runtime/src/sched.rs` so `scope_active`, `scope_ever_active`, and `scope_startup_fired` are `tinymap::KeySet<ScopeKey>`. Initialization should insert a scope only when its initial flag is true. Reads such as `self.scope_active[scope]` can stay as indexing operations because `KeySet` already implements `Index<K, Output = bool>`. Writes that set a flag true should call `insert`; writes that set `scope_active` false should call `remove` or `set(scope, false)`. This trial is accepted only if focused scheduler tests still pass and `ping_pong`, `modal_modes`, and `modal_scheduler_perf` do not show a new meaningful regression. The trial on 2026-07-07 failed that performance bar, so keep `KeySet::remove` and leave `EventManager` on `TinySecondaryMap<ScopeKey, bool>` unless a future representation avoids the measured regressions.

Eighth, filter modal reactions before downstream port triggers are inserted into future levels. Add a helper used by `Scheduler::process_tag`, such as `reaction_is_enabled_at_current_tag(reaction_key, terminal)`, that uses cached active-scope state and shutdown activation history. Use this helper both when filtering the current level and when extending `next_levels` from set ports. This ensures inactive modal reactions are not inserted into `ReactionSet` only to be skipped later.

Ninth, remove or bypass redundant mode-filter checks. Inspect how `ReactionBuilder::enabled_modes`, `ReactionGraph::reaction_modes`, and runtime `ModeFilter` are used. If `enabled_modes` is only populated by `in_mode_scope`, then the reaction's scope already determines whether it is active. In that case, remove `ModeFilter` from the hot path and possibly from `ReactionGraph`. If public or macro code still creates mode filters independently, keep the data structure but precompute a cheap per-reaction predicate that does not scan a `Vec<ModeKey>` every time. The 2026-07-07 implementation kept `ReactionGraph::reaction_modes` for compatibility and debug data, but bypassed `ModeFilter` in release scheduler gating. `Scheduler::reaction_is_enabled_at_current_tag` now checks shutdown lifecycle history, active static reaction scope, and then returns true. A debug-only assertion verifies that any existing mode filter is exactly the static mode owning the reaction scope.

Tenth, avoid per-tag transition dedup allocation. `Scheduler::process_tag` currently stores transition requests in `transition_buffer`, then creates a second `Vec` to collapse to last-wins per reactor. Instead, collapse into the existing reusable buffer as requests are observed: when a reaction schedules a mode transition, search the current buffer for that reactor and replace the request if found, otherwise push. This keeps last-wins behavior and removes the second allocation.

Finally, validate behavior and performance. Semantic behavior must stay unchanged. Benchmark improvements should be recorded with before and after numbers. If any optimization worsens `ping_pong`, stop and record the regression in `Surprises & Discoveries` before proceeding.

## Concrete Steps

From the repository root `/Users/johhug01/Source/boomerang`, begin by confirming the working branch and current files:

    git status --short --branch
    rg -n "name = \"modal_modes\"|name = \"ping_pong\"|\\[\\[bench\\]\\]" boomerang/Cargo.toml

Add `boomerang/benches/modal_scheduler_perf.rs` and register it in `boomerang/Cargo.toml`. Compile it first without running a long benchmark:

    cargo test -p boomerang --bench modal_scheduler_perf

Expected result: Cargo builds the benchmark target and exits successfully. Criterion benchmarks may print "running 0 tests" in test mode; that is acceptable. If compilation fails, fix the benchmark before touching scheduler internals.

Capture baseline benchmark results:

    cargo bench -p boomerang --bench ping_pong
    cargo bench -p boomerang --bench modal_modes
    cargo bench -p boomerang --bench modal_scheduler_perf

Expected result: Criterion prints timing reports for all benchmark groups. Record the relevant median or typical times in `Artifacts and Notes` before optimizing.

After each scheduler optimization milestone, run the focused modal tests and benchmark compile checks:

    cargo test -p boomerang modal
    cargo test -p boomerang_runtime
    cargo test -p boomerang --bench modal_scheduler_perf

For the KeySet flag-map milestone, also run the tinymap unit tests because that milestone changes `boomerang_tinymap/src/key_set/mod.rs`:

    cargo test -p boomerang_tinymap key_set

Then run focused benchmarks to compare the bitset-backed flags against the current `TinySecondaryMap<ScopeKey, bool>` layout:

    cargo bench -p boomerang --bench ping_pong
    cargo bench -p boomerang --bench modal_modes
    cargo bench -p boomerang --bench modal_scheduler_perf

After all implementation work, run the broader validation:

    cargo test
    cargo fmt --check
    git diff --check
    cargo bench -p boomerang --bench ping_pong
    cargo bench -p boomerang --bench modal_modes
    cargo bench -p boomerang --bench modal_scheduler_perf

Expected result: tests pass, formatting checks pass, and benchmarks show no meaningful regression in `ping_pong`. `modal_scheduler_perf` should show better transition churn, reset subtree, and inactive port fanout results than the baseline captured at the start of this plan.

## Validation and Acceptance

Acceptance requires both behavior and measurement.

Behavior is accepted when `cargo test` passes for the workspace, including all existing modal integration tests. The modal semantics tested by `boomerang/tests/modal_actions.rs`, `boomerang/tests/modal_timers.rs`, `boomerang/tests/modal_reset_reactions.rs`, `boomerang/tests/modal_startup_shutdown.rs`, and related modal tests must remain unchanged.

Benchmark coverage is accepted when `cargo bench -p boomerang --bench modal_scheduler_perf` includes at least these named groups: `transition_churn`, `inactive_port_fanout`, and `reset_subtree`. Each group must run more than one size or variant so scaling behavior is visible.

Allocation coverage is accepted when a steady-state non-modal root action chain can run repeated `Scheduler::next()` calls after warmup without additional heap allocations, or when a documented ignored diagnostic test explains why the assertion cannot be stable on this platform. The preferred acceptance is an ordinary passing test, not an ignored test.

Performance is accepted when the new benchmark demonstrates that the optimized scheduler avoids the pathologies identified in this plan. Specifically, root/global action scheduling should not allocate per event, transition-heavy cases should not allocate temporary vectors per transition, inactive modal port fanout should avoid inserting known-inactive reactions into future levels, and `ping_pong` should remain within normal Criterion noise of the baseline captured before this plan's implementation.

The KeySet flag-map trial is accepted for the tinymap utility when `boomerang_tinymap::KeySet` has a tested single-key clear or set operation. The scheduler storage conversion is accepted only if `EventManager` can store `scope_active`, `scope_ever_active`, and `scope_startup_fired` as `tinymap::KeySet<ScopeKey>` without regressing `ping_pong`, `modal_modes`, or `modal_scheduler_perf`. The 2026-07-07 direct conversion failed that performance acceptance and was reverted, so the accepted outcome is the utility method plus documented rejection of the scheduler conversion.

## Idempotence and Recovery

All steps are intended to be safe to repeat. Benchmark files and runtime code edits are ordinary source changes. If a benchmark run is interrupted, rerun the same command; Criterion will update local output under `target/criterion`, which should not be committed. If an optimization causes semantic tests to fail, revert only the specific optimization in progress or patch it forward; do not discard unrelated user changes in the worktree.

Avoid destructive Git commands. The working tree may contain user changes unrelated to this plan. Before editing a file, inspect it and preserve unrelated edits. If a benchmark or test produces large logs, keep them under `target/` or another ignored location unless this plan is explicitly updated to track a small excerpt.

## Artifacts and Notes

Initial benchmark coverage assessment:

    `boomerang/benches/ping_pong.rs` covers non-modal root action scheduling.
    `boomerang/benches/modal_modes.rs` covers many inactive mode-local timer queues while root work advances.
    `boomerang/benches/port_bank.rs` covers non-modal port fanout and banking.
    No current benchmark covers transition churn, reset-subtree scaling, inactive modal port fanout, or steady-state allocation counts.

Checkpoint benchmark results after the first allocation patches:

    2026-07-07 10:02Z checkpoint on this workspace:
      ping_pong/100: 14.988 us median estimate, 6.6720 Melem/s.
      ping_pong/10_000: 1.2863 ms median estimate, 7.7743 Melem/s.
      ping_pong/1_000_000: 128.73 ms median estimate, 7.7683 Melem/s.

      modal_modes/inactive_modes/1: 1.8575 ms median estimate, 5.3836 Melem/s.
      modal_modes/inactive_modes/32: 1.8580 ms median estimate, 5.3820 Melem/s.
      modal_modes/inactive_modes/256: 2.1871 ms median estimate, 4.5722 Melem/s.

      modal_scheduler_perf/transition_churn/reset/10_000: 2.8683 ms median estimate, 3.4864 Melem/s.
      modal_scheduler_perf/transition_churn/history/10_000: 2.5583 ms median estimate, 3.9088 Melem/s.
      modal_scheduler_perf/transition_churn/reset/100_000: 31.259 ms median estimate, 3.1990 Melem/s.
      modal_scheduler_perf/transition_churn/history/100_000: 27.746 ms median estimate, 3.6041 Melem/s.
      modal_scheduler_perf/inactive_port_fanout/1: 2.1239 ms median estimate, 4.7084 Melem/s.
      modal_scheduler_perf/inactive_port_fanout/32: 3.2892 ms median estimate, 3.0402 Melem/s.
      modal_scheduler_perf/inactive_port_fanout/256: 7.1895 ms median estimate, 695.46 Kelem/s.
      modal_scheduler_perf/inactive_port_fanout/1024: 5.3171 ms median estimate, 188.07 Kelem/s.
      modal_scheduler_perf/reset_subtree/small: 4.2194 ms median estimate, 2.3700 Melem/s.
      modal_scheduler_perf/reset_subtree/medium: 35.222 ms median estimate, 141.96 Kelem/s.
      modal_scheduler_perf/reset_subtree/large: 244.07 ms median estimate, 4.0972 Kelem/s.

Post-filtering modal scheduler benchmark results:

    2026-07-07 10:10Z after cached active-scope gating, inactive port-trigger filtering, and in-place transition dedup:
      modal_scheduler_perf/transition_churn/reset/10_000: 2.6295 ms median estimate, about 8.23% faster than checkpoint.
      modal_scheduler_perf/transition_churn/history/10_000: 2.2919 ms median estimate, about 10.20% faster than checkpoint.
      modal_scheduler_perf/transition_churn/reset/100_000: 28.087 ms median estimate, about 10.40% faster than checkpoint.
      modal_scheduler_perf/transition_churn/history/100_000: 24.916 ms median estimate, about 10.05% faster than checkpoint.
      modal_scheduler_perf/inactive_port_fanout/1: 2.1165 ms median estimate, no statistically significant change.
      modal_scheduler_perf/inactive_port_fanout/32: 2.4821 ms median estimate, about 24.86% faster than checkpoint.
      modal_scheduler_perf/inactive_port_fanout/256: 2.8080 ms median estimate, about 61.13% faster than checkpoint.
      modal_scheduler_perf/inactive_port_fanout/1024: 1.7425 ms median estimate, about 67.49% faster than checkpoint.
      modal_scheduler_perf/reset_subtree/small: 3.9438 ms median estimate, about 7.13% faster than checkpoint.
      modal_scheduler_perf/reset_subtree/medium: 35.031 ms median estimate, change within noise threshold.
      modal_scheduler_perf/reset_subtree/large: 243.94 ms median estimate, no statistically significant change.

Post-filtering non-modal guard benchmark:

    2026-07-07 10:10Z:
      ping_pong/100: 14.966 us median estimate, no significant change from checkpoint.
      ping_pong/10_000: 1.2744 ms median estimate, no significant change from checkpoint.
      ping_pong/1_000_000: 126.56 ms median estimate, within Criterion noise threshold.

Post-index modal scheduler benchmark results:

    2026-07-07 10:20Z after `ModalScheduleIndex`:
      modal_scheduler_perf/transition_churn/reset/10_000: 2.1306 ms median estimate, about 18.80% faster than post-filtering.
      modal_scheduler_perf/transition_churn/history/10_000: 2.0172 ms median estimate, about 11.90% faster than post-filtering.
      modal_scheduler_perf/transition_churn/reset/100_000: 23.560 ms median estimate, about 15.86% faster than post-filtering.
      modal_scheduler_perf/transition_churn/history/100_000: 22.523 ms median estimate, about 8.93% faster than post-filtering.
      modal_scheduler_perf/inactive_port_fanout/1: 2.0848 ms median estimate, change within noise threshold.
      modal_scheduler_perf/inactive_port_fanout/32: 2.4429 ms median estimate, change within noise threshold.
      modal_scheduler_perf/inactive_port_fanout/256: 2.7761 ms median estimate, change within noise threshold.
      modal_scheduler_perf/inactive_port_fanout/1024: 1.7572 ms median estimate, no statistically significant change.
      modal_scheduler_perf/reset_subtree/small: 3.1035 ms median estimate, about 21.26% faster than post-filtering.
      modal_scheduler_perf/reset_subtree/medium: 30.304 ms median estimate, about 13.35% faster than post-filtering.
      modal_scheduler_perf/reset_subtree/large: 221.90 ms median estimate, about 9.04% faster than post-filtering.

Post-index non-modal guard benchmark:

    2026-07-07 10:20Z:
      ping_pong/100: 14.718 us median estimate, Criterion reported improvement versus previous local history.
      ping_pong/10_000: 1.2311 ms median estimate, Criterion reported improvement versus previous local history.
      ping_pong/1_000_000: 122.82 ms median estimate, Criterion reported improvement versus previous local history.

State-layout split validation:

    2026-07-07 13:23Z after splitting `EventManager` state into hot flag maps, `ScopeClockState`, and per-scope `EventQueue` maps:
      cargo test -p boomerang_runtime: passed, 17 unit tests and 2 doc tests.
      cargo test -p boomerang modal: passed all modal-filtered integration tests.
      cargo test -p boomerang --bench modal_scheduler_perf: passed all Criterion smoke scenarios in test mode.
      cargo fmt --check: passed, with existing stable-rustfmt warnings about unstable wrap comment settings.
      git diff --check: passed.

State-layout split Criterion benchmark results:

    2026-07-07 13:45Z after splitting `EventManager` state:
      modal_scheduler_perf/transition_churn/reset/10_000: 2.1277 ms median estimate, about 1.03% faster than local Criterion history and within noise.
      modal_scheduler_perf/transition_churn/history/10_000: 2.0433 ms median estimate, about 0.94% slower than local Criterion history and within noise.
      modal_scheduler_perf/transition_churn/reset/100_000: 23.207 ms median estimate, about 2.15% faster than local Criterion history and within noise.
      modal_scheduler_perf/transition_churn/history/100_000: 22.363 ms median estimate, about 2.10% faster than local Criterion history and within noise.
      modal_scheduler_perf/inactive_port_fanout/1: 2.0896 ms median estimate, no detected change.
      modal_scheduler_perf/inactive_port_fanout/32: 2.3767 ms median estimate, about 2.96% faster than local Criterion history.
      modal_scheduler_perf/inactive_port_fanout/256: 2.6443 ms median estimate, about 5.05% faster than local Criterion history.
      modal_scheduler_perf/inactive_port_fanout/1024: 1.4452 ms median estimate, about 18.69% faster than local Criterion history.
      modal_scheduler_perf/reset_subtree/small: 3.1269 ms median estimate, no detected change.
      modal_scheduler_perf/reset_subtree/medium: 31.270 ms median estimate, about 2.81% slower than local Criterion history.
      modal_scheduler_perf/reset_subtree/large: 226.85 ms median estimate, about 2.23% slower than local Criterion history.

    2026-07-07 13:45Z modal_modes guard:
      modal_modes/inactive_modes/1: 1.8759 ms median estimate, within noise.
      modal_modes/inactive_modes/32: 1.8845 ms median estimate, within noise.
      modal_modes/inactive_modes/256: 2.1961 ms median estimate, no detected change.

    2026-07-07 13:45Z ping_pong guard:
      First pass:
        ping_pong/100: 15.397 us median estimate, Criterion reported about 4.94% slower.
        ping_pong/10_000: 1.2914 ms median estimate, Criterion reported about 5.32% slower.
        ping_pong/1_000_000: 128.04 ms median estimate, Criterion reported about 4.25% slower.
      Repeat pass:
        ping_pong/100: 15.741 us median estimate, within noise.
        ping_pong/10_000: 1.4488 ms median estimate, Criterion reported about 5.47% slower with four high-severe outliers.
        ping_pong/1_000_000: 135.07 ms median estimate, Criterion reported about 5.48% slower.
      Process check:
        Read-only `ps` check found no active Cargo or rustc process and only idle CodeStory repair processes, so the non-modal regression should not be dismissed purely as background build contention.

State-layout split profiling artifacts:

    2026-07-07 13:58Z:
      BOOMERANG_PROFILE=1 cargo bench -p boomerang --bench ping_pong -- ping_pong/1000000
      result: pprof flamegraph written to `target/criterion/ping_pong/1000000/profile/flamegraph.svg`, with the visible hot frames under `Scheduler::next`, `FnRefsAdapter::trigger`, `Context::schedule_action`, `EventQueue::push_event`, and `OffsetBucket::insert`.

      target/release/deps/modal_scheduler_perf-59cc9c35e8ca5347 --bench reset_subtree/large/512
      sampled with macOS `sample` for 8 seconds after warmup.
      report: `/tmp/modal_reset_large.sample.txt`.
      top diagnosis: `EventQueue::push_event_inner` accounts for the largest sampled runtime, mostly `_platform_memset` and `__bzero`, which indicates dense `ReactionSet` storage clearing or initialization dominates the reset-subtree case.

      target/release/deps/ping_pong-dd8178770d48a9af --bench ping_pong/1000000
      sampled with macOS `sample` for 8 seconds after warmup.
      report: `/tmp/ping_pong_1m.sample.txt`.
      top diagnosis: the profile is rooted in the non-modal scheduler path. `Scheduler::process_tag`, `EventQueue::push_event_inner`, tracing span recording, action scheduling, and action-store insertion dominate; modal state split maps are not visible hot frames.

Single-clear `ReactionSet` pooling results:

    2026-07-07 14:07Z after changing `EventQueue` so pooled `ReactionSet`s are cleared on return rather than again on checkout:
      cargo bench -p boomerang --bench modal_scheduler_perf -- reset_subtree
      reset_subtree/small/1: 3.0684 ms median estimate, no detected change.
      reset_subtree/medium/64: 31.503 ms median estimate, no detected change.
      reset_subtree/large/512: 229.06 ms median estimate, no detected change. This is much tighter than the earlier noisy profile-pass result around 244.16 ms and does not repeat the reported reset-subtree regression.

      cargo bench -p boomerang --bench ping_pong
      ping_pong/100: 14.259 us median estimate, Criterion reported about 7.79% faster.
      ping_pong/10_000: 1.1818 ms median estimate, Criterion reported about 13.89% faster.
      ping_pong/1_000_000: 116.74 ms median estimate, Criterion reported about 13.32% faster.

KeySet flag-map trial:

    2026-07-07 14:46Z after adding `KeySet::remove` and temporarily converting `EventManager` scope flags to `tinymap::KeySet<ScopeKey>`:
      cargo test -p boomerang_tinymap key_set: passed 9 tests, including `key_set::tests::test_remove`.
      cargo test -p boomerang_runtime: passed 17 unit tests and 2 doc-tests, with 1 ignored doc-test.
      cargo test -p boomerang modal: all modal-filtered integration tests passed.
      cargo test -p boomerang --bench modal_scheduler_perf: all benchmark smoke cases reported Success.
      cargo bench -p boomerang --bench ping_pong:
        ping_pong/100: 15.333 us median estimate, Criterion reported about 4.89% slower.
        ping_pong/10_000: 1.1810 ms median estimate, no detected change.
        ping_pong/1_000_000: 117.38 ms median estimate, no detected change.
      cargo bench -p boomerang --bench modal_modes:
        inactive_modes/1: 1.8513 ms median estimate, change within noise threshold.
        inactive_modes/32: 1.8651 ms median estimate, no detected change.
        inactive_modes/256: 2.1919 ms median estimate, no detected change.
      cargo bench -p boomerang --bench modal_scheduler_perf:
        transition_churn/reset/10_000: 2.2434 ms median estimate, Criterion reported about 5.32% slower.
        transition_churn/history/10_000: 2.1516 ms median estimate, Criterion reported about 6.14% slower.
        transition_churn/reset/100_000: 24.890 ms median estimate, Criterion reported about 7.69% slower.
        transition_churn/history/100_000: 23.941 ms median estimate, Criterion reported about 7.92% slower.
        inactive_port_fanout/1: 2.3415 ms median estimate, Criterion reported about 7.52% slower.
        inactive_port_fanout/32: 2.5293 ms median estimate, Criterion reported about 7.09% slower.
        inactive_port_fanout/256: 3.1853 ms median estimate, Criterion reported about 19.76% slower.
        inactive_port_fanout/1024: 1.7818 ms median estimate, Criterion reported about 24.57% slower.
        reset_subtree/small: 3.1100 ms median estimate, no detected change.
        reset_subtree/medium: 31.188 ms median estimate, no detected change.
        reset_subtree/large: 226.88 ms median estimate, change within noise threshold.
      Outcome: the scheduler conversion was reverted. `KeySet::remove` and its unit test remain because they are independently useful and do not affect scheduler performance.

ModeFilter hot-path bypass results:

    2026-07-07 15:12Z after changing `Scheduler::reaction_is_enabled_at_current_tag` to rely on static reaction scope activity and leave `ModeFilter` as debug-checked metadata:
      cargo bench -p boomerang --bench modal_scheduler_perf:
        transition_churn/reset/10_000: 2.1763 ms median estimate, Criterion reported about 2.49% faster.
        transition_churn/history/10_000: 2.0758 ms median estimate, Criterion reported about 3.87% faster.
        transition_churn/reset/100_000: 23.967 ms median estimate, Criterion reported about 3.27% faster.
        transition_churn/history/100_000: 22.955 ms median estimate, Criterion reported about 4.05% faster.
        inactive_port_fanout/1: 2.1048 ms median estimate, no detected change.
        inactive_port_fanout/32: 2.3006 ms median estimate, Criterion reported about 9.41% faster.
        inactive_port_fanout/256: 2.5714 ms median estimate, Criterion reported about 18.20% faster.
        inactive_port_fanout/1024: 1.3227 ms median estimate, Criterion reported about 25.21% faster.
        reset_subtree/small: 3.0532 ms median estimate, change within noise threshold.
        reset_subtree/medium: 31.756 ms median estimate, no detected change.
        reset_subtree/large: 224.60 ms median estimate, change within noise threshold.

      cargo bench -p boomerang --bench ping_pong:
        ping_pong/100: 15.388 us median estimate, no detected change.
        ping_pong/10_000: 1.1258 ms median estimate, Criterion reported about 4.49% faster.
        ping_pong/1_000_000: 112.16 ms median estimate, Criterion reported about 4.44% faster.

Change log:

    2026-07-07 / Codex: created this plan from the modal scheduler performance review. The plan intentionally starts with benchmark coverage before implementation changes so later optimizations have measurable evidence.
    2026-07-07 / Codex: added the first benchmark target and recorded its smoke-test compile/run result; allocator coverage remains a separate follow-up item.
    2026-07-07 / Codex: implemented the first allocation reduction in `ScheduledEvent`, recorded that a full Criterion baseline was not captured before this patch, and kept the remaining mode-local heap drain/rebuild as later transition-path work.
    2026-07-07 / Codex: added steady-state allocation coverage and fixed the action-store pruning allocation it exposed. The new guard passes only after both the scheduled-event metadata change and the `ActionStore::clear_older_than` change.
    2026-07-07 / Codex: captured Criterion checkpoint results after the allocation fixes. Local Criterion history reported improvements for `ping_pong` and `modal_modes`, but later comparisons in this plan should use the explicit checkpoint numbers above.
    2026-07-07 / Codex: completed the state-layout milestone by splitting `EventManager` scope state into hot active/startup flag maps, `ScopeClockState`, and per-scope queue storage. This keeps scheduler active checks on compact indexed booleans while preserving the existing queue and local/global time behavior.
    2026-07-07 / Codex: ran the Criterion benchmark set after the state-layout split. The result is mixed: modal fanout improved materially, transition churn stayed within noise, reset-subtree regressed slightly, and `ping_pong` reported a guard regression that needs follow-up before final validation.
    2026-07-07 / Codex: profiled the regressing areas. The largest modal reset cost is dense `ReactionSet` zeroing inside `EventQueue::push_event_inner`; the non-modal `ping_pong` profile does not show hot modal scope-state frames. This suggests the next investigation should target event queue/reaction-set reuse and clearing behavior before spending effort on mode-filter removal.
    2026-07-07 / Codex: implemented the first profiling diagnosis solution by adding `EventQueue::recycle_reaction_set`, returning merged and cleared events through that helper, and making `EventQueue::next_reaction_set` reuse already-clear pooled sets without clearing again. Focused re-bench results fixed the `ping_pong` guard regression and stabilized the reset-subtree result.
    2026-07-07 / Codex: wrapped the state-layout plus single-clear `ReactionSet` pooling milestone as accepted for focused validation, then added a new next milestone to replace `EventManager`'s `TinySecondaryMap<ScopeKey, bool>` hot flags with bitset-backed `KeySet<ScopeKey>` flags. This preserves benchmark attribution by measuring the flag storage change before continuing to `ModeFilter` cleanup.
    2026-07-07 / Codex: trialed the bitset-backed flag milestone. Added `KeySet::remove` with unit coverage, measured a direct `EventManager` conversion, and rejected/reverted the scheduler conversion after Criterion showed broad transition-churn and inactive-fanout regressions. The next implementation milestone is now the redundant `ModeFilter` cleanup.
    2026-07-07 / Codex: completed the redundant `ModeFilter` hot-path cleanup by removing the release-mode `Store::current_mode` lookup and `ModeFilter::allows` vector scan from scheduler reaction gating. The graph metadata remains for compatibility, and a debug assertion verifies that existing filters match the static mode scope.
    2026-07-07 / Codex: implemented cached active-scope gating, inactive modal port-trigger filtering, and in-place transition dedup. The targeted modal benchmark shows large inactive-fanout gains with no meaningful `ping_pong` regression.
    2026-07-07 / Codex: implemented `ModalScheduleIndex` on `ReactionGraph` and switched transition lifecycle helpers to dense range slices. Reset-subtree and transition-churn benchmarks improved again; `ping_pong` did not regress.
    2026-07-07 / Codex: ran final hygiene and full test validation for this implementation slice. `cargo fmt --check` passes but still prints the repository's existing stable-rustfmt warnings for `wrap_comments` and `comment_width`.
    2026-07-07 / Codex: restored `ReactionGraph::shutdown_reactions()` to read the source maps rather than the derived index so the public helper remains correct even before index rebuild. The scheduler still uses `modal_schedule_index` directly for shutdown scheduling.
    2026-07-07 / Codex: replaced the local `IndexRange` wrapper with `core::range::Range<usize>`. Current Serde still does not serialize this new core range type directly, so `ModalScheduleIndex` now serializes its range maps through an explicit start/end shim while `ReactionGraph` preserves the modal index as pure data.
    2026-07-07 / Codex: moved `ModalScheduleIndex` construction out of `boomerang_runtime` and into the builder lowering pass in `boomerang_builder/src/env/build.rs`. Runtime now owns only the index data type, accessors, and serde representation; builder owns the static construction algorithm. Added module-level docs in both env modules describing this boundary.
    2026-07-08 / Codex: closed the final validation gate after the scheduler was split into `sched/mod.rs`, `sched/queue.rs`, `sched/modal.rs`, and `sched/barrier.rs`. Full workspace tests, clippy, formatting, mdBook, benchmark smoke tests, and complete modal Criterion benchmark runs passed.

Validation transcript:

    2026-07-07 09:49Z:
      cargo test -p boomerang_runtime event
      result: 1 passed; 0 failed; 16 filtered out.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

    2026-07-07 10:10Z:
      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo test -p boomerang --test scheduler_alloc
      result: 1 passed; 0 failed.

      cargo test -p boomerang modal
      result: all modal-filtered integration tests passed; modal_physical_actions emitted expected scheduler lateness warnings.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

      cargo bench -p boomerang --bench modal_scheduler_perf
      result: inactive_port_fanout/1024 improved by about 67.49%; transition_churn/history/100_000 improved by about 10.05%.

      cargo bench -p boomerang --bench ping_pong
      result: no statistically significant change for 100 and 10_000 element cases; 1_000_000 element case remained within Criterion noise threshold.

    2026-07-07 10:20Z:
      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo test -p boomerang --test scheduler_alloc
      result: 1 passed; 0 failed.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

      cargo test -p boomerang modal
      result: all modal-filtered integration tests passed; modal_physical_actions emitted expected scheduler lateness warnings.

      cargo bench -p boomerang --bench modal_scheduler_perf
      result: reset_subtree/large improved by about 9.04% relative to the post-filtering run; transition_churn/reset/100_000 improved by about 15.86%.

      cargo bench -p boomerang --bench ping_pong
      result: Criterion reported improvements for all three cases; no non-modal regression was observed.

    2026-07-07 10:23Z:
      cargo fmt --check
      result: passed; rustfmt printed stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      git diff --check
      result: passed with no whitespace errors.

      cargo test
      result: full workspace tests and doc-tests passed. Notable totals included `boomerang` integration tests, 32 `boomerang_builder` unit tests, 30 `boomerang_macros` unit tests, 17 `boomerang_runtime` unit tests, and 30 `boomerang_tinymap` unit tests; several existing doc-tests are ignored as before.

    2026-07-07 10:24Z:
      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo fmt --check
      result: passed; rustfmt printed stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      git diff --check
      result: passed with no whitespace errors.

    2026-07-07 11:47Z:
      cargo update -p serde -p serde_core -p serde_derive
      result: no newer compatible Serde packages were selected; the registry resolution kept the existing Serde versions.

      cargo test -p boomerang_runtime --features serde
      result: 18 passed; 0 failed; doc-tests 1 passed, 1 ignored. This was before the later builder/runtime boundary change for modal index construction.

      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo test -p boomerang --test scheduler_alloc
      result: 1 passed; 0 failed.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

      cargo fmt --check
      result: passed; rustfmt printed stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      git diff --check
      result: passed with no whitespace errors.

      cargo test
      result: full workspace tests and doc-tests passed. Notable totals included `boomerang` integration tests, 32 `boomerang_builder` unit tests, 30 `boomerang_macros` unit tests, 18 `boomerang_runtime` unit tests, and 30 `boomerang_tinymap` unit tests; existing ignored doc-tests remained ignored.

    2026-07-07 12:05Z:
      cargo test -p boomerang_runtime --features serde
      result: 18 passed; 0 failed; doc-tests 1 passed, 1 ignored. This includes `env::tests::reaction_graph_serde_preserves_modal_schedule_index`.

      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo test -p boomerang_builder
      result: 32 passed; 0 failed; doc-tests 0 passed.

      cargo test -p boomerang --test scheduler_alloc
      result: 1 passed; 0 failed.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

      cargo fmt --check
      result: passed; rustfmt printed stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      git diff --check
      result: passed with no whitespace errors.

      cargo test
      result: full workspace tests and doc-tests passed. Notable totals included `boomerang` integration tests, 32 `boomerang_builder` unit tests, 30 `boomerang_macros` unit tests, 18 `boomerang_runtime` unit tests, and 30 `boomerang_tinymap` unit tests; existing ignored doc-tests remained ignored.

    2026-07-07 09:57Z:
      cargo test -p boomerang_runtime action::store
      result: 4 passed; 0 failed; 13 filtered out.

      cargo test -p boomerang --test scheduler_alloc
      result: 1 passed; 0 failed.

      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo test -p boomerang modal
      result: all modal-filtered integration tests passed; modal_physical_actions emitted expected scheduler lateness warnings.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

    2026-07-07 14:07Z:
      cargo fmt --check
      result: passed; rustfmt printed the repository's existing stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

      cargo test -p boomerang modal
      result: all modal-filtered integration tests passed; modal_physical_actions emitted expected scheduler lateness warnings.

      cargo bench -p boomerang --bench modal_scheduler_perf -- reset_subtree
      result: no detected reset-subtree regression; large case median estimate was 229.06 ms.

      cargo bench -p boomerang --bench ping_pong
      result: Criterion reported improvements for all three cases, with the 1,000,000 element case at 116.74 ms.

    2026-07-07 14:08Z:
      cargo fmt --check
      result: passed; rustfmt printed the repository's existing stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      git diff --check
      result: passed with no whitespace errors.

    2026-07-07 14:46Z:
      cargo test -p boomerang_tinymap key_set
      result: 9 passed; 0 failed. This includes the new `key_set::tests::test_remove`.

      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo test -p boomerang modal
      result: all modal-filtered integration tests passed.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

      cargo fmt --check
      result: passed; rustfmt printed the repository's existing stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      git diff --check
      result: passed with no whitespace errors.

      cargo bench -p boomerang --bench ping_pong
      result: larger guard cases reported no detected change, but `ping_pong/100` regressed by about 4.89% during the temporary `KeySet` scheduler conversion.

      cargo bench -p boomerang --bench modal_modes
      result: no meaningful regression; the 1-mode case was within the configured noise threshold and larger cases reported no detected change.

      cargo bench -p boomerang --bench modal_scheduler_perf
      result: the temporary `KeySet` scheduler conversion regressed transition churn by about 5-8% and inactive fanout by about 7-25%, so the scheduler conversion was reverted while keeping `KeySet::remove`.

      cargo test -p boomerang_tinymap key_set
      result after reverting only the scheduler conversion: 9 passed; 0 failed.

      cargo test -p boomerang_runtime
      result after reverting only the scheduler conversion: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo fmt --check
      result after reverting only the scheduler conversion: passed with the existing stable-rustfmt warnings.

      git diff --check
      result after reverting only the scheduler conversion: passed with no whitespace errors.

    2026-07-07 15:12Z:
      cargo test -p boomerang_runtime
      result: 17 passed; 0 failed; doc-tests 1 passed, 1 ignored.

      cargo test -p boomerang modal
      result: all modal-filtered integration tests passed.

      cargo test -p boomerang --bench modal_scheduler_perf
      result: all transition_churn, inactive_port_fanout, and reset_subtree benchmark test-mode cases reported Success.

      cargo fmt --check
      result: passed; rustfmt printed the repository's existing stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      git diff --check
      result: passed with no whitespace errors.

      cargo bench -p boomerang --bench modal_scheduler_perf
      result: transition churn improved by about 2.49-4.05%, inactive fanout improved by about 9.41-25.21% for the larger mode counts, and reset-subtree cases were neutral.

      cargo bench -p boomerang --bench ping_pong
      result: no non-modal regression; 100 was neutral and the 10,000 and 1,000,000 cases improved by about 4.49% and 4.44%.

    2026-07-08 11:50Z:
      cargo fmt --check
      result: passed; rustfmt printed the repository's existing stable-channel warnings for unsupported `wrap_comments` and `comment_width` config options.

      cargo clippy --workspace --all-targets
      result: passed with no warnings.

      cargo test
      result: full workspace tests and doc-tests passed. Notable totals included 32 `boomerang_builder` unit tests, 30 `boomerang_macros` unit tests, 18 `boomerang_runtime` unit tests, and 30 `boomerang_tinymap` unit tests; existing ignored doc-tests remained ignored.

      mdbook build book
      result: passed; HTML book written under `book/book`.

      cargo test -p boomerang --bench modal_modes
      result: all inactive mode benchmark smoke cases reported Success.

      cargo bench -p boomerang --bench modal_scheduler_perf
      result: transition_churn/history and transition_churn/reset improved or reported no detected change; inactive_port_fanout improved through 256 modes and reported no detected change at 1024 modes; reset_subtree medium and large improved while small reported no detected change.

      cargo bench -p boomerang --bench modal_modes
      result: Criterion reported improvements for inactive mode counts 1, 32, and 256.

      git diff --check
      result: passed with no whitespace errors.

## Interfaces and Dependencies

Use the existing dependencies already present in `boomerang/Cargo.toml`: `criterion` for benchmarks and `pprof` for optional flamegraphs. Do not add a benchmark dependency unless the need is recorded in `Decision Log`.

In `boomerang/benches/modal_scheduler_perf.rs`, define Criterion benchmark groups using the same style as existing benchmark files:

    fn bench(c: &mut Criterion) { ... }

    fn criterion_config() -> Criterion { ... }

    criterion_group! {
        name = benches;
        config = criterion_config();
        targets = bench
    }
    criterion_main!(benches);

In `boomerang_runtime/src/event.rs`, change `ScheduledEvent` so action rebasing metadata is inline and optional rather than a vector allocated for every new event. The preferred field is:

    pub(crate) action_value: Option<ScheduledActionValue>

In `boomerang_runtime/src/env/mod.rs`, add any new modal indexing structs next to `ReactionGraph` because they describe static runtime graph relationships. If a range type is needed, keep it simple, copyable, and serializable behind the existing `serde` feature if `ReactionGraph` remains serializable.

In `boomerang_builder/src/env/build.rs`, build the modal index after runtime scopes, actions, modes, and reactions are available. The index must not depend on scheduler state. It is static metadata.

In `boomerang_runtime/src/sched.rs`, update `EventManager` and `Scheduler::process_tag` to use the new static index and cached active-scope data. Public scheduler behavior and public APIs should not change.

In `boomerang_tinymap/src/key_set/mod.rs`, add a single-key clear or set operation to `KeySet<K>`, with unit coverage in the existing `#[cfg(test)]` module. The method must leave other keys unchanged and must be safe to call for any key index representable by `K`.

In `boomerang_runtime/src/sched.rs`, do not directly convert these fields to bitset-backed `KeySet<ScopeKey>` in the current implementation:

    scope_active: tinymap::KeySet<ScopeKey>
    scope_ever_active: tinymap::KeySet<ScopeKey>
    scope_startup_fired: tinymap::KeySet<ScopeKey>

That direct conversion was benchmarked and rejected on 2026-07-07 because it regressed transition churn and inactive fanout. A future flag representation may still be considered, but it must preserve the public `EventManager::scope_active` and `EventManager::scope_ever_active` helper behavior and must pass the same benchmark acceptance gate before replacing the current `TinySecondaryMap<ScopeKey, bool>` fields.
