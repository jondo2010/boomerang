# Native emission-path qualification evidence

## Status and boundary

This report covers the native-path audit in `tests/native_path/`, not a production
`tracing-bounded` subscriber. Its allocator observer is `src/test_allocation.rs`,
shared with unit tests and absent from production builds.
The crate also has a public native subscriber and remains unpublished; its
separate subscriber and backend regressions are described in [README.md](README.md).
This is evidence for the upstream part of specification P2/P8/Q1 only.

The audit installs one test-only native `Subscriber`, emits ordinary `tracing`
events, and asserts its observed accepted/rejected counts and sequence bitmask.
It never calls the subscriber's event handler directly. Its four event sites
have explicit root parents and one field each. Span callbacks deliberately panic
if reached; there is no claim of span support, ring behavior, producer admission,
general loss accounting, or P1–P9 conformance.

## Recorded environment

The initial audit was run on 2026-09-18 with:

- `rustc 1.98.1 (48a229cea 2026-09-01)`, Cargo 1.98.1;
- target/host `aarch64-apple-darwin`, debug and release binary profiles;
- Miri `0.1.0 (a36d05efab 2026-09-09)` for `first-use` and `unsupported`;
- `tracing 0.1.44`, `tracing-core 0.1.36`, `once_cell 1.21.4`, and
  `pin-project-lite 0.2.17`.

The initial standalone audit requested only `std` on tracing and tracing-core,
with default features disabled. The Cargo-managed tests now inherit workspace
dependencies and normal feature unification, including tracing's default
`attributes` feature. They have no independent pins or lockfile. The resolved
versions and features must be recorded for each qualification run; workspace
updates do not automatically inherit these results. The commands below inspect
the package test dependency tree. Whole-workspace or consumer builds can unify
additional features and need their own inspection, especially for `log` and
`log-always`. This is not an MSRV or embedded-target claim.

The migration rerun on the same host retained the four versions above and added
`tracing-attributes 0.1.31` through workspace defaults. Package-scoped tests have
neither `log` nor `log-always`; the broader workspace enables `log` via Foxglove
and is not this Q1 qualification configuration.

## Reproduce the observations

From the root of the `tracing-bounded` crate (including an extracted package),
run:

```sh
cargo test -p tracing-bounded
cargo test -p tracing-bounded --release
cargo tree -p tracing-bounded -e features
cargo clippy -p tracing-bounded --all-targets -- -D warnings
cargo fmt -p tracing-bounded --check

# Miri executes each allocation scenario directly, without subprocess spawning.
cargo +nightly miri test -p tracing-bounded --lib
cargo +nightly miri test -p tracing-bounded --test native_path -- first-use
cargo +nightly miri test -p tracing-bounded --test native_path -- unsupported
cargo +nightly miri test -p tracing-bounded --lib -- \
  --ignored --exact capture::first_backend_event_does_not_allocate
cargo +nightly miri test -p tracing-bounded --lib -- \
  --ignored --exact subscriber_tests::first_native_span_lifecycle_does_not_allocate
```

Ordinary `cargo test` runs the unit tests and the native-path allocation test
executable. The native-path executable starts all seven scenarios in separate
processes, reusing the same Cargo-built binary. It also accepts one scenario name
for direct execution, as in the Miri commands above. It uses `main`, not libtest
filtering: select it with `--test native_path`, without libtest flags such as
`--nocapture` or nextest. The first-backend-event regression is an ordinary unit
test in `capture`; it runs itself in a fresh process selected with `--exact` and
checks that the child completed the probe, not merely exited successfully.
Under Miri it is ignored in the full suite and explicitly run alone with the
command above, without subprocess spawning. Current workspace CI runs ordinary
Cargo tests; its Miri nextest lane selects other packages.
No temporary workspace, source copying or independent build cache is involved.
Add `--offline --locked` when the current resolution is cached and must not change.

| Scenario | Accepted | Rejected | Sequence mask | Measured allocations / deallocations |
| --- | ---: | ---: | ---: | ---: |
| `first-use` | 1 | 0 | 1 | 0 / 0 |
| `repeated` | 1024 | 0 | 1 | 0 / 0 |
| `different-sites` | 2 | 0 | 3 | 0 / 0 on each producer |
| `same-site` | 2 | 0 | 1 | 0 / 0 on each producer |
| `filtered` | 0 | 0 | 0 | 0 / 0 |
| `unsupported` | 0 | 1 | 0 | 0 / 0 |

`allocator-control` detects a deliberate boxed allocation and deallocation
(1 / 1) and verifies an empty interval returns 0 / 0. These observations passed
in both debug and release. The unsupported value's Debug implementation panics
if called; the successful rejection therefore also checks that it was not
formatted.

Thread-local observer state is initialized before measurement without emitting
any event. Installation, thread creation, barrier waits, joins, snapshots,
assertions, and printing occur outside measured intervals. The concurrent cases
start two workers at a barrier, but scheduling may serialize them: they do not
prove that a particular registration race or CAS failure occurred.

The allocator delegates to `System` and observes allocation attempts and
deallocation operations. Successful reallocation counts as one of each. It is
test infrastructure, not part of the production library or a general allocator
profiler. Zero observed operations is limited to these executed scenarios.

The initial implementation also underwent deliberately failing controls:
disabling event capture failed the first-use snapshot, invoking Debug failed
the unsupported sentinel, disconnecting the allocator observer failed its
positive control, and adding a boxed allocation in the event callback failed
the zero-allocation assertion in both debug and release. All mutations were
removed before final verification.

## Closed-callsite inventory and source argument

For the audited `native_path` test executable, a conservative process-wide bound
is **`C = 4`**. It is not a bound for the backend probe, the library unit-test
binary, or the Boomerang workspace, nor inferred from observed events.

The inventory includes the four literal native macro invocations in `sites.rs`:
`first`, `second`, `filtered`, and `rejected`. None constructs callsites
dynamically or registers them manually. The dependency closure above contains
no additional active native emission sites: tracing defines and expands the
macros at their invocation sites; tracing-core provides registration/dispatch;
once_cell and pin-project-lite do not depend on tracing. Dependency source
inspection distinguishes rustdoc examples, macro definitions, and cfg(test)
code from the normal library build. The workspace-default attributes proc macro
is a build-time dependency and is not used by these sites. There are no audit
build scripts, dynamic plugins, or dynamically loaded instrumentation. Rust's
standard library does not depend on this third-party tracing instance.

As an additional artifact check, `nm` on both the debug and release test binaries
was repeated after migration. The debug binary reports four writable `__CALLSITE`
symbols, one for each inventory entry, and four read-only `META` symbols (not
four more callsites). The workspace release profile enables LTO: its four `META`
symbols remain, but writable callsite symbols are optimized away. Unlike the
original standalone release build, symbol counting cannot corroborate all four
callsites there. The source/dependency inventory supplies the bound; symbol
inspection is not a portable standalone proof. Recheck the
inventory and dependency features whenever source, toolchain, flags or resolution
changes. A downstream application's bound must be established independently.

To inspect a current artifact, run `cargo test -p tracing-bounded --test native_path
--no-run --message-format=json` (also with `--release`) and use `nm` on the exact
`executable` path in its `compiler-artifact` record. Count writable `__CALLSITE`
symbols, not `META` entries. No special build script or copied fixture is needed.

Audited upstream source locations (version-specific, available in the crates.io
source archives):

| Source | Relevant argument |
| --- | --- |
| `tracing-0.1.44/src/macros.rs:721–765` | Explicit-parent events use a static `DefaultCallsite`, then native enabled/field/event dispatch |
| `tracing-core-0.1.36/src/callsite.rs:308–343` | Only one thread wins registration for one callsite; another sees `sometimes`, without waiting |
| `tracing-core-0.1.36/src/callsite.rs:437–464` | Strong-CAS append-only list insertion; failed comparisons observe a distinct newer head |
| `tracing-core-0.1.36/src/callsite.rs:544–578` | One fixed dispatcher selects `JustOne`, bypassing the multi-dispatcher registry read lock |
| `tracing-core-0.1.36/src/dispatcher.rs:379–399` | With no scoped dispatcher, `get_default` uses the fixed global dispatcher directly |

Each failed list-head comparison implies at least one distinct intervening
insertion. With at most four nodes and no removal/reuse, an insertion can fail
at most three comparisons before succeeding: at most four list-head CAS
attempts. The separate registration-state CAS adds one operation on a first-use
path. The Q1 bound counts **list insertion attempts**, not all instructions or
all atomic operations in an emission. No thread must wait for another thread
to finish interest registration or for an output consumer.

This argument is conditional on Q1's full deployment envelope. It says nothing
about wall-clock latency, OS scheduling, hardware implementation of atomics,
general span lifecycle, or an arbitrary subscriber's callback behavior. The
allocation observations are additional evidence, not a proof of this retry bound.

## Value-set shape and visitation bound

P2 requires macro-shaped value sets from all producers, including dependencies
and manual native emission. For `tracing-core 0.1.36`, `Event::record`
(`src/event.rs:86–87`) delegates to `ValueSet::record`
(`src/field.rs:1068–1090`). Its explicit branch scans every supplied pair, even
foreign or empty entries, and continues after a visitor latches rejection.
Explicit arrays can be larger than the metadata field set or repeat fields;
neither subscriber storage limits nor early visitor rejection bound that scan.
`Event::fields` exposes declared fields, not the underlying entries, and no
public event accessor exposes their count for a constant-work preflight check.

Under P2, the total entries are at most the declared field count. The backend
checks that count against its configured field limit before visitation, so the
upstream loop also has that bound. Positional values match the declared fields;
sparse/reordered explicit values use each declared field at most once. The
restriction is a deployment precondition, not a runtime validator: arbitrary
out-of-profile value sets have no bounded-work or rejection/accounting guarantee.
Unsupported representations within the shape still undergo P4 rejection without
formatting. Re-audit both native macro expansion and manual producers when
dependencies or instrumentation change.

The backend's `manual_positional_values_retain_declared_field_identities` and
`manual_sparse_reordered_values_retain_supplied_field_identities` tests dispatch
hand-built root events through native `Event::child_of` and assert retained
metadata, exact ordered fields and zero loss. These characterize existing root
capture behavior; they are not shape enforcement, an allocation measurement or
span qualification. Their callsites are in the unit-test binary and do not
change the separate native-path audit's `C = 4` inventory.

## Component assessment

The following source assessments explain why these candidates were not adopted
on the strict capture path; neither is a dependency of the audit:

- `thingbuf 0.1.6`, `src/lib.rs:193–249`: queue push uses a weak-CAS retry/backoff
  loop. Q1's finite one-time callsite argument does not bound recurring queue
  traffic or spurious weak-CAS failures. Its fixed capacity alone is insufficient
  evidence for this contract.
- `tracing-serde-structured 0.4.0`, `src/lib.rs:642–649` and `806–810`: stock
  visitors expose Debug serialization or format into owned allocated strings.
  They cannot be assumed to satisfy the strict primitive-only formatter rule.
  Off-path serialization or reuse of other components remains a separate option.

These are scoped compatibility findings, not a claim that no useful ecosystem
components exist.

## Remaining release conditions

The audit establishes narrow upstream native-output/allocation observations
and an artifact-specific Q1 registration argument. Separate public-subscriber tests
exercise preparation, inherited snapshots, cross-thread handles, updates, slot
reuse, invalid context, reference limits and lock-independent releases. The fresh
process `first_native_span_lifecycle_does_not_allocate` test checks first-use native
span creation, cloning, update, entry/current context, scoped event capture and
final drops with zero allocator operations after preparation. It does not extend
the audit binary's `C = 4` inventory to the library-test binary or an application.
Boomerang now has optional coordination capture, prepared runtime/transport workers,
and generated hosted off/streaming/bounded modes with shutdown export. Its native
output tests cover deterministic worker preparation, first-failure retention after
overwrite, and a concurrent exchange with explicit loss. The generated two-Federate
test exercises all three modes. A complete application callsite/feature inventory
and target-specific P8 evidence remain outstanding. Publication remains disabled.
