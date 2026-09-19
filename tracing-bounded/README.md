# tracing-bounded

A proposed bounded, loss-aware subscriber for native Rust `tracing` events and
spans. Intended for applications where diagnostics may lose data but must not
wait for output capacity, allocate during capture, or corrupt execution context.

**Status: unpublished partial implementation.** A tested internal backend now
captures explicit-root native primitive events into fixed record/field/byte
storage. There is still no public subscriber, producer/span context, complete
allocation qualification, or conformance claim. Publication is disabled. The
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

These are requirements for the complete implementation, **not guarantees
established by the current partial backend**. [SPEC.md](SPEC.md) is the normative
contract and release conformance checklist. It is also rendered as the
`specification` rustdoc module.
The adopted closed-callsite profile also requires an application-established
upper bound `C` covering all static native callsites in the process, including
dependencies. Under the audited upstream registration algorithm, an insertion
needs at most `C` compare-and-swap attempts. No replacement macros or callsite
warm-up are required. This is a conditional source-level work bound, not a timing
guarantee; implementation and full release qualification are still pending.

## Fit and limitations

The initial supported profile is a fixed native subscriber installed before
execution, with a bounded set of producer threads. Setup and snapshot inspection
may allocate. Dynamic subscriber composition and replacement are outside the
bounded guarantee.

For configured record, field and copied-byte limits `R`, `F` and `B`, the current
internal backend reserves `R + 1` slots (the ring plus scratch). Its logical data
bound is `(R + 1) * (size_of::<Slot>() + F *
size_of::<Option<StoredField>>() + B)`, plus the shared allocation and `Arc`,
`Mutex`/`Storage` header (ring pointer/length and indices), atomic loss counters
and fixed callback stack state. The ring uses a `Box<[Slot]>` of length `R`, so
those `R` slot headers are the outer table in the formula; scratch is one
embedded `Slot`. Every slot owns boxed field and byte arrays of exactly `F` and
`B` logical elements. Some targets also allocate platform mutex storage during
constructor initialization; this is separate from the logical slot/`Shared`
struct bound. Allocator bookkeeping, alignment and size-class rounding
are implementation-dependent and are not exact requested bytes. Snapshot-owned
`Vec`, `String` and byte storage, along with upstream callsite/dispatcher/TLS
storage, is separate. Span, producer, depth and reclamation storage is not
implemented, so this is not yet a complete P3 bound.

Ring, field and byte arrays use fallible reservations that return
`BuildError::Allocation` on reservation failure. `Arc`/shared allocation and
standard platform mutex initialization do not promise recoverable global
allocation failure. Setup may allocate and block; the constructor acquires and
releases its new mutex before returning either backend handle.

The private backend's visitation work bound currently assumes macro-generated
events with matching metadata/value sets and a bounded number of visited entries.
Manually constructed native events are not yet qualified: tracing-core 0.1.36
permits explicit value sets with repeated fields independently of metadata count,
and continues visiting entries after the visitor has latched rejection. Resolving
this native-input qualification gap is a gate for the future public Subscriber,
alongside producer preparation, span/context and lifecycle qualification. The
normative P1/P2 contract is unchanged.

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
