# tracing-bounded

A bounded, loss-aware subscriber for native Rust `tracing` events and
spans. Intended for applications where diagnostics may lose data but must not
wait for output capacity, allocate during capture, or corrupt execution context.

**Status: unpublished initial implementation.** The public native subscriber
captures primitive events with copied span context, bounded producer admission,
span updates, reference handling and reuse. Initial Boomerang coordination integration
is available; complete target qualification and full conformance remain outstanding. The
package name is provisional; checking registry search results does not reserve it.

[QUALIFICATION.md](QUALIFICATION.md) records a native emission-path
audit with Cargo-managed tests. Its test-only subscriber is not the production
implementation and does not establish full conformance.

## Why a separate crate?

The capture mechanism does not need to know about an application's scheduler,
protocol, identifiers, or event vocabulary. This crate is developed in the
Boomerang workspace but must remain independently usable, without depending on
Boomerang, an executor, a transport, or its configuration format.

Applications keep standard `tracing` macros and spans. They supply bounded native
field values and an immutable target/level filter. The subscriber owns capture
storage, span context, loss accounting, and off-path inspection.

## Use

```rust
use tracing_bounded::{BoundedSubscriber, Config};

let (subscriber, capture) = BoundedSubscriber::new(Config::default())?;
tracing::subscriber::set_global_default(subscriber)?;
let producer = capture.prepare_current_thread()?;
let span = tracing::info_span!("request", id = 42u64);
span.in_scope(|| tracing::info!(done = true));
// Inspection may allocate and wait; keep it outside sensitive execution.
assert_eq!(capture.snapshot()[0].scopes[0].metadata.name(), "request");
drop(producer);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Keep the fixed global subscriber installed throughout execution. Prepare each
emitting thread and retain its non-Send guard until that thread stops emitting.
Libraries starting worker threads can use `prepare_current_thread()` after
installing the inherited dispatcher. It returns an owned guard for a newly
prepared native bounded subscriber, or `None` for other subscribers and a
thread already prepared for the same subscriber. Nested use does not reset
invalid context or take ownership of the outer guard. Setup failure remains
visible through `lifecycle_loss().producer_admission`.
Enter async task spans per poll; a prepared thread is not a task identity.
Failed admission propagates an invalid span, never another span or a healthy root.
Depth overflow or unbalanced exit invalidates that producer until teardown and
fresh preparation. `loss`, `lifecycle_loss` and `current_thread_is_valid` expose
event loss and context health independently of ring contents.

## Intended guarantees

- Fixed limits cover records, fields, bytes, live spans, producer contexts, and
  scope depth—not merely a bounded queue of heap-allocated messages.
- After producer preparation, supported capture performs no allocation, output
  I/O, blocking lock acquisition, or arbitrary formatting callbacks.
- Full rings overwrite the oldest complete record. Other capture failures drop
  whole events and remain visible through out-of-band loss counters/status.
- Retained events include their captured scope fields. Losing earlier output
  does not silently change a later event's identity.
- Invalid context is reported or rejected; it is never replaced with another
  task's or thread's context.

These claims are conditional on the supported profile and target qualification;
passing hosted tests does not qualify every deployment. [SPEC.md](SPEC.md) is the normative
contract and release conformance checklist. It is also rendered as the
`specification` rustdoc module.
The adopted closed-callsite profile also requires an application-established
upper bound `C` covering all static native callsites in the process, including
dependencies. Under the audited upstream registration algorithm, an insertion
needs at most `C` compare-and-swap attempts. No replacement macros or callsite
warm-up are required. This is a conditional source-level work bound, not a timing
guarantee; full release qualification is still pending.

## Fit and limitations

The initial supported profile is a fixed native subscriber installed before
execution, with a bounded set of producer threads. Setup and snapshot inspection
may allocate. Dynamic subscriber composition and replacement are outside the
bounded guarantee.

For `R` records, `S` spans, `P` producers and depth `D`, the owned logical storage
consists of the following fixed allocations (`sizeof` uses the build's layouts):

- Ring table: `R * sizeof(Slot)`; span table: `S * sizeof(SpanSlot)`.
- Record arrays including scratch: `(R + 1) * (F * sizeof(Option<StoredField>)
  + B + D * sizeof(Option<StoredScope>))`.
- Span arrays including update scratch: `(S + 1) * (SF * sizeof(Option<StoredField>)
  + SB)`, with `SF`/`SB` the per-span limits.
- References: `S * sizeof(References)`; producer stacks:
  `P * (sizeof(Producer) + D * sizeof(Entry))`.
- The capture `Shared` and subscriber `State` headers, including both embedded
  scratch-slot headers, mutex/header state and loss counters; two Arc control blocks.

All arrays are boxed at construction. No reclamation queue is allocated. Platform
mutex storage for the `P + 2` warmed mutexes, allocator bookkeeping/rounding,
upstream dispatcher/callsite storage and off-path snapshots are additional.
Subscriber TLS holds only a token/index pair per prepared thread. Callbacks use
fixed stack locals, not capacity-sized stack arrays or recursive traversal.

Each reference admission uses one strong CAS; failure is visible. Owned reference
release uses one atomic subtraction independent of capture locks. A last release
can leave deferred work in its fixed span slot; bounded sweeps reclaim it before
subsequent span admission. Sweeps take at most `S` passes over `S` slots; context
copying takes at most `D²` ancestry steps. Generations never wrap: exhausted slots
retire. Span updates rebuild scratch (at most `SF²` field comparisons), preserving
unmodified values without accumulating obsolete byte storage. Failed updates
invalidate the span and its descendants. These are source-level bounds, not WCET.

Ring, field and byte arrays use fallible reservations that return
`BuildError::Allocation` on reservation failure. `Arc`/shared allocation and
standard platform mutex initialization do not promise recoverable global
allocation failure. Setup may allocate and block; the constructor acquires and
releases its new mutex before returning either backend handle.

The supported-input profile requires macro-shaped value sets: matching metadata,
no repeated or foreign fields, and no more entries (including empty ones) than
declared fields. Positional entries correspond exactly to the declared fields;
explicit entries may be sparse or reordered. Native macros satisfy this shape;
hand-built events/span values must also comply. This is a producer/dependency
precondition, not subscriber validation. Arbitrary value sets have no bounded-work,
rejection or loss-accounting guarantee: upstream can scan extra entries even after
visitor rejection. See P2 and [QUALIFICATION.md](QUALIFICATION.md). Root-event
output tests cover conforming manual inputs. The functional subscriber has native
context/lifecycle tests; deployment-specific qualification remains required.

The process-wide callsite bound is a deployment responsibility, not a capacity
that this subscriber can enforce. Dynamic callsite creation and runtime-loaded
instrumentation are outside the profile. Dependency versions and unified Cargo
features must be audited; the initial qualification configuration disables
tracing's `log` and `log-always` features. See specification Q1 for the exact
constraints and required evidence.

Primitive scalars and bounded strings/bytes are supported by the design. Fields
using `?value`, `%value`, formatted messages, or error formatting are not assumed
safe: the strict subscriber will reject unsupported records without invoking
those formatters. Applications can retain a separate ordinary hosted subscriber
configuration when those diagnostics are wanted.

Allocation-free capture is not allocator-free startup, `no_std` support, interrupt
safety, or a hard real-time qualification. None of those broader claims is made
by this initial profile. In particular, native tracing's own setup requirements
must be accounted for; a `no_std` label alone does not establish no-allocation use.

This is diagnostic capture, not durable logging, payload recording, replay,
delivery acknowledgement, or recovery storage. It does not impose a wire format
or promise that a failure event can never be overwritten.

## Reuse and publication

Existing components must be evaluated against the contract before adoption.
Initial source inspection found that `thingbuf 0.1.6` uses weak-CAS retry/backoff
in its queue push path; the closed-callsite proof does not bound recurring queue
traffic. `tracing-serde-structured 0.4.0` provides stock visitors that permit
Debug formatting, and its owned capture allocates. Neither is selected for the
strict capture path. This is a scoped assessment, not a claim that no ecosystem
components could be reused. Any adoption must preserve explicit loss, bounded
context, and the formatter rule.

Before publication, the implementation must pass the specification's conformance
matrix, provide runnable examples and API documentation, declare its tested Rust
version/platform profile, and pass standalone package verification. The published
archive must include this README, the specification, and both license texts.

## Testing

From this crate's root, including an extracted package, run:

```sh
cargo test -p tracing-bounded
cargo test -p tracing-bounded --release
```

These commands run the backend unit tests, the first-backend-event regression,
and all seven [native-path audit](QUALIFICATION.md) scenarios. The first-event
regression lives directly in `capture` as a unit test. It starts a fresh copy of
the unit-test executable, selecting only itself, so other tests cannot warm up
tracing state. The native-path audit remains a harness-free executable with one
child process per scenario. No shell runner, production-source inclusion,
copying, or isolated build is required. Both share the test-only allocation
observer and workspace dependencies; Cargo's usual lockfile and build cache apply.

The regression accesses private backend state without a public test API. After
dispatch and observer preparation, it measures the first
real macro-generated explicit-root event without preceding capture or snapshot
access, then checks retained fields and zero loss off-path. A deliberate
allocation/deallocation validates the observer. This root-only regression is
not full final-subscriber or cross-platform allocation qualification.

Production builds forbid unsafe code. Test builds deny it except inside the
shared allocation observer, whose `GlobalAlloc` implementation delegates to
`System`. Miri runs the first-event test alone without spawning; see the exact
commands in [QUALIFICATION.md](QUALIFICATION.md).

Package verification remains a separate release check (`cargo package` followed
by tests in the extracted archive). Dependency updates are allowed through the
workspace; resolved versions/features remain part of qualification evidence,
not independently pinned requirements. See Q1 before making boundedness claims
for a new dependency resolution.

## License

Licensed under either [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option.
