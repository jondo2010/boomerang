# Native emission-path qualification evidence

## Status and boundary

This report covers the standalone audit fixture in
`tests/fixtures/native-path`, not a production `tracing-bounded` subscriber.
The crate still has no production capture implementation and remains unpublished.
This is evidence for the upstream part of specification P2/P8/Q1 only.

The fixture installs one test-only native `Subscriber`, emits ordinary `tracing`
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
  `pin-project-lite 0.2.17`, with registry checksums in the fixture lockfile.

The standalone fixture workspace requests only `std` on tracing and tracing-core,
with their default features disabled. The resulting tree enables once_cell's
`default`, `std`, `alloc`, and `race` features and pin-project-lite's default
feature. There is no `log`, `log-always`, attributes proc macro, external formatter,
executor, or Boomerang dependency. This is not an MSRV or embedded-target claim.
The host workspace's feature graph is not this fixture's feature graph.

## Reproduce the observations

From the root of the `tracing-bounded` crate (including an extracted package),
using a POSIX shell:

```sh
sh tests/run-native-path.sh all
sh tests/run-native-path.sh miri
sh tests/run-native-path.sh inventory
```

The runner copies `Cargo.toml.in`, the lockfile and sources into a fresh temporary
workspace and removes only that temporary workspace when it exits. The template
name prevents Cargo's nested-package exclusion from omitting the fixture from
the published archive. Build output is confined to that temporary workspace.
`all` runs seven scenarios in debug and release, followed by feature inspection,
strict Clippy and formatting. `miri` runs the two interpreter scenarios, and
`inventory` builds both profiles and lists their callsite symbols using `nm`.

Offline commands require the locked dependencies to be cached. Each scenario
executes in a new process. Running the parent crate's
`cargo test` does **not** execute this standalone fixture. Miri runs the selected
scenario directly, without spawning subprocesses under the interpreter.

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

For this fixture's standalone normal binary build, a conservative process-wide
bound is **`C = 4`**. This is not inferred from the number of observed events.

The inventory includes the four literal native macro invocations in `sites.rs`:
`first`, `second`, `filtered`, and `rejected`. None constructs callsites
dynamically or registers them manually. The dependency closure above contains
no additional active native emission sites: tracing defines and expands the
macros at their invocation sites; tracing-core provides registration/dispatch;
once_cell and pin-project-lite do not depend on tracing. Dependency source
inspection distinguishes rustdoc examples, macro definitions, and cfg(test)
code from the normal library build. There are no fixture build scripts, proc
macros, dynamic plugins, or dynamically loaded instrumentation. Rust's standard
library does not depend on this third-party tracing instance.

As an additional artifact check, `nm` on both the debug and release binaries
reports four writable `__CALLSITE` symbols, one for each of the inventory entries.
Their four read-only `META` symbols are metadata, not four more callsites. Symbol
inspection corroborates the source/dependency inventory; it is not a portable
standalone proof for stripped or differently optimized binaries. Recheck the
inventory and dependency features whenever source, toolchain, flags or resolution
changes. A downstream application's bound must be established independently.

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

## Component assessment

The following source assessments explain why these candidates were not adopted
on the strict capture path; neither is a dependency of the fixture:

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

The fixture establishes narrow upstream native-output/allocation observations
and an artifact-specific Q1 registration argument. The reusable subscriber,
bounded record and span storage, lifetime handling, rejection/overflow counters,
snapshot API, complete memory bound, product examples, and remaining P8 matrix
are **unimplemented**. Span/dispatch lifetimes and actual producer preparation
still need their own emission-path audit. Publication remains disabled.
