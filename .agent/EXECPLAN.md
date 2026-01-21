# Implement Offset-Bucketed ActionStore With Pruned Microsteps

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `.agent/PLANS.md`.

## Purpose / Big Picture

After this change, the action store maintains correct logical ordering while never leaking memory as logical time advances. The store no longer tracks microstep state in a separate global map; instead, each logical offset owns its microstep bookkeeping so pruning old offsets also prunes their microstep state. This matters because long-running simulations will keep a stable memory footprint. The change is observable by running the action store tests: the new tests will prove that clearing old tags removes the per-offset state and that the next microstep computed after pruning does not depend on stale data.

## Progress

- [x] (2026-01-21 13:05Z) Read current action store implementation, call sites, and existing tests to anchor behavior and constraints.
- [x] (2026-01-21 13:06Z) Draft the new offset-bucket data model and method contracts in this plan.
- [x] (2026-01-21 13:18Z) Refactor `ActionStore` implementation to offset buckets and remove `next_microstep` global map.
- [x] (2026-01-21 13:25Z) Rewrite tests for concise, non-overlapping coverage of ordering, replacement, pruning, and microstep allocation.
- [x] (2026-01-21 13:28Z) Run `boomerang_runtime` tests and capture output evidence.

## Surprises & Discoveries

- Observation: The current store keeps `next_microstep` in a `HashMap<Duration, usize>` that is not pruned, which will leak memory as logical time advances.
  Evidence: `boomerang_runtime/src/action/store.rs` tracks `next_microstep` in `push` and does not delete entries in `clear_older_than`.
- Observation: `ping_pong` benchmark regressed after the offset-bucket refactor compared to the baseline commit.
  Evidence: baseline in `/tmp/boomerang-baseline` ran `ping_pong/100` at ~13.28 µs vs current ~13.88 µs; `ping_pong/10000` ~1.145 ms vs ~1.357 ms; `ping_pong/1000000` ~116.9 ms vs ~144.6 ms.
- Observation: Profiling `ping_pong/1000000` shows `BTreeMap::insert` as a dominant leaf inside `Context::schedule_action`, suggesting the new per-offset `BTreeMap` storage is the primary cost.
  Evidence: `target/criterion/ping_pong/1000000/profile/flamegraph.svg` reports `alloc::collections::btree::map::BTreeMap::insert` at ~68.99% of samples.
- Observation: Replacing per-offset microstep storage with a `VecDeque<Option<T>>` removed the regression and improved throughput beyond baseline.
  Evidence: `ping_pong/100` ~12.12 µs, `ping_pong/10000` ~1.05 ms, `ping_pong/1000000` ~106.0 ms after the change, compared to baseline ~13.28 µs / 1.145 ms / 116.9 ms.

## Decision Log

- Decision: Use an offset-bucketed structure (`BTreeMap<Duration, OffsetBucket<T>>`) so per-offset microstep state is pruned automatically when offsets are removed.
  Rationale: Keeps microstep bookkeeping tied to the lifecycle of each logical offset, eliminating leaks while preserving ordering.
  Date/Author: 2026-01-21 / Codex

## Outcomes & Retrospective

To be filled after implementation. This will summarize whether the new store maintains ordering, replacement, and pruning behavior, and whether the tests prove the memory-pruning behavior.

## Context and Orientation

The action store lives in `boomerang_runtime/src/action/store.rs` and is used by the action system in `boomerang_runtime/src/action/mod.rs` and `boomerang_runtime/src/action/action_ref.rs`. The store currently uses a `BinaryHeap<ActionEntry<T>>` to keep actions ordered by `Tag` and a `HashMap<Duration, usize>` named `next_microstep` to track the next microstep for an offset. `Tag` is defined in `boomerang_runtime/src/time.rs` and includes `offset: Duration` and `microstep: usize`. The method `next_microstep_for_offset` is used by `ActionRef` to compute the next microstep when scheduling actions at a given offset.

The new architecture replaces the heap + global map with per-offset buckets. Each bucket owns the actions for one logical offset and tracks `next_microstep`. This ensures that clearing old offsets also clears the microstep state and any actions, preventing unbounded growth.

## Plan of Work

First, replace the storage data structure in `boomerang_runtime/src/action/store.rs`. Introduce a private `OffsetBucket<T>` struct that owns a map from microstep to `ActionEntry<T>` (or directly to the payload type) and a `next_microstep` counter. The top-level `ActionStore<T>` should be a `BTreeMap<Duration, OffsetBucket<T>>` keyed by offset, plus any counters needed for stable replacement semantics. Define how ordering is determined: offsets are ordered by `Duration`, and within an offset microsteps are ordered by their numeric value. The existing requirement that pushing a new value for the same tag replaces the old one should be preserved by overwriting the entry in the bucket for that microstep.

Second, re-implement the methods:

`push(tag, data)` should find or create the bucket for `tag.offset()`, insert or replace the payload for `tag.microstep()`, and update the bucket’s `next_microstep` to at least `tag.microstep() + 1`. This must not allocate outside the bucket’s storage beyond the standard collection growth.

`next_microstep_for_offset(offset, min_microstep)` should return the bucket’s `next_microstep` if present, otherwise return `min_microstep`. It should always return at least `min_microstep`, even if the stored `next_microstep` is lower.

`clear_older_than(clear_tag)` should remove all buckets with `offset < clear_tag.offset()`. For the bucket at `clear_tag.offset()`, remove all actions with microstep `< clear_tag.microstep()`. If the bucket becomes empty after this, remove it entirely so its `next_microstep` is pruned as well.

`get_current(tag)` should call `clear_older_than(tag)` first, then look for an action at `tag.offset()` and `tag.microstep()`. It should return `Some(&T)` only when that exact tag exists; otherwise return `None`.

Third, rewrite the tests in `boomerang_runtime/src/action/store.rs` to be concise and non-overlapping. The goal is minimal tests that each cover one distinct behavior. Remove the heap-ordering tests that are implementation-specific and replace them with tests that assert observable behavior of `ActionStore`:

One test should assert out-of-order insertions are retrieved in tag order via `get_current` and that repeated requests for the same tag return the same value while entries older than the requested tag are pruned.

One test should assert that pushing multiple values at the same tag overwrites the previous value and that the newest value is returned.

One test should assert `next_microstep_for_offset` returns the minimum when no data exists and returns updated values after pushes. It should also show that clearing older tags removes per-offset state by checking that a cleared offset falls back to the minimum again.

One test should assert that clearing at a tag prunes microsteps below the tag but retains the current and newer microsteps for the same offset.

These tests should use small, explicit tag sequences and avoid duplicating checks. Any helper functions should be kept local to the test module.

## Concrete Steps

All commands should be run from `/Users/johhug01/Source/boomerang`.

1. Re-open the action store implementation and its tests:
   - `sed -n '1,240p' boomerang_runtime/src/action/store.rs`
   - `sed -n '1,200p' boomerang_runtime/src/action/action_ref.rs`
2. Implement the new offset-bucket store in `boomerang_runtime/src/action/store.rs`, replacing the heap and `next_microstep` map.
3. Rewrite the test module in the same file to match the coverage described above.
4. Run tests:
   - `cargo test -p boomerang_runtime`

Expected short transcript to capture:

    /Users/johhug01/Source/boomerang$ cargo test -p boomerang_runtime
    ...
    test result: ok. <N> passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

## Validation and Acceptance

The change is accepted when:

The action store returns the correct value for each tag in monotonic tag order, including when values are inserted out of order.

Clearing older tags removes per-offset microstep state so `next_microstep_for_offset` does not return values derived from offsets that have been fully cleared.

Replacing a value at the same tag returns the most recent value.

All `boomerang_runtime` tests pass, and the new tests demonstrate the pruning behavior that prevents unbounded memory growth.

## Idempotence and Recovery

These changes are safe to apply multiple times because they are local to the action store and its tests. If a change breaks compilation, revert only the action store file to the last known working state and reapply the steps in smaller edits. If tests fail due to overly strict assertions, relax the test to match the documented observable behavior while keeping the pruning guarantees.

## Artifacts and Notes

Capture short snippets of the new `ActionStore` data structure and a representative test that proves pruning. For example:

    struct ActionStore<T: ReactorData> {
        offsets: BTreeMap<Duration, OffsetBucket<T>>,
        counter: usize,
    }

    #[test]
    fn clears_offset_microstep_state() {
        ...
    }

Test evidence:

    /Users/johhug01/Source/boomerang$ cargo test -p boomerang_runtime
    ...
    test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

## Interfaces and Dependencies

No new external dependencies are required. The implementation should use standard library collections (`BTreeMap` or `Vec` as appropriate) in `boomerang_runtime/src/action/store.rs`.

Keep these public method signatures stable in `boomerang_runtime/src/action/store.rs`:

    pub fn push(&mut self, tag: Tag, data: T)
    pub fn next_microstep_for_offset(&self, offset: Duration, min_microstep: usize) -> usize
    pub fn clear_older_than(&mut self, clear_tag: Tag)
    pub fn get_current(&mut self, tag: Tag) -> Option<&T>

The `BaseActionStore` trait in the same file must continue to match the `clear_older_than` behavior.

Change note (2026-01-21): Created the initial ExecPlan for the offset-bucket action store refactor, capturing tests and pruning requirements in a self-contained plan.
Change note (2026-01-21): Updated progress and artifacts after implementing the offset-bucket store and running `boomerang_runtime` tests.
