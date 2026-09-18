# Bounded diagnostic capture specification

## Status and terminology

This is the normative contract for `tracing-bounded`, currently an unpublished
initial native subscriber with bounded event/span storage and prepared producer
contexts. Application integration and full release qualification remain incomplete.
**MUST**, **MUST NOT**, and **SHOULD** denote requirements and recommendations.
A release MUST disclose unmet requirements and MUST NOT claim full conformance
for a partial implementation.

- **Producer:** an emitting thread, not an async task.
- **Preparation:** installation, allocation and thread-local initialization before
  allocation-sensitive execution.
- **Capture:** supported event/span callbacks, including inherited context,
  lifetime bookkeeping and loss accounting.
- **Record:** a complete event with metadata, fields and captured scopes.
- **Scope:** span metadata/fields, ordered by parent ancestry.
- **Loss:** discarded/overwritten events or unavailable span/context state.
  Intentional filtering is not loss.

## P1. Independence and native interoperability

The crate MUST implement the native `tracing` subscriber interface without
replacement macros, a second event vocabulary, or host-application types.
Production dependencies MUST NOT include Boomerang, Tokio, other executors, transports, the
standard formatting subscriber or its growing registry. Applications may select
an ordinary hosted formatter instead. Installation MUST be explicit; importing
the crate has no global effect.

The packaged crate MUST build and test independently of repository-relative
implementation files, and include public API docs and this specification.
Application integration policy remains outside it.

## P2. Supported deployment profile

The initial `std` profile permits setup allocation and uses one fixed subscriber;
all active dispatch handles refer to it. Dynamic filters, competing/replacement
subscribers and arbitrary layers are outside the profile.

This **closed-callsite profile** requires an application-established, process-wide
upper bound `C` on static native callsites, including dependencies and filtered
sites. This is deployment evidence, not a subscriber-enforced capacity; Q1 defines
the constraints.

All guarantees require **macro-shaped value sets** for events, span creation and
span updates: values MUST use the associated callsite's metadata field set.
Explicit entries MUST reference only its declared fields, at most once each;
the total entry count, including empty entries, MUST NOT exceed the declared
field count. Positional entries MUST correspond exactly to the declared fields.
Sparse or reordered explicit entries are permitted. This is a producer/dependency
precondition, not subscriber-validated input: native macros satisfy the shape,
and hand-built native values MUST also satisfy it. Arbitrary out-of-profile sets
have no bounded-work, rejection or loss-accounting guarantee. P4's unsupported
value rejection still applies within this shape; P5 permits repeated names
across distinct scopes.

Preparation MUST precede reliance on capture guarantees. Producer admission MUST
use reserved bounded storage or fail visibly, without growing capture storage or
aborting the application. Individual event sites require no crate-specific calls.

The guarantee covers supported callbacks and the native emission path, including
first use. Application field expressions are excluded; their construction MUST
independently meet the producer's execution requirements.

Capture MUST NOT allocate/deallocate heap storage, perform output I/O, block on
locks, sleep, or wait for another producer/consumer to make progress. Work MUST
be bounded by configured capacities, supported field sizes and `C`. Q1's finite
registration retries are permitted; subscriber callbacks MUST NOT retry without
a bound. Callback-only guarantees do not satisfy this whole-path requirement.

No cycle-count, scheduling, hard-WCET, `no_std`, allocator-free startup, interrupt
safety or microcontroller/RTOS qualification is implied. Such profiles require
separate guarantees and target evidence, not inference from dependency features.

## P3. Configuration, filtering, and storage

Configuration is immutable. Construction MUST validate size arithmetic and return
typed errors for unsupported limits, never silently clamp or wrap.

Explicit bounds are required for:

| Quantity | Meaning |
| --- | --- |
| Record count | Simultaneously retained events |
| Record bytes / fields | Total copied value bytes / field occurrences, including scopes |
| Live spans | Occupied span slots, including deferred reclamation |
| Span bytes / fields | Copied value bytes / field occurrences per span |
| Producer contexts | Simultaneously admitted contexts |
| Scope depth | Entered-scope bookkeeping and traversed parent depth |

Structural bounds MUST be positive, except record count may be zero (all enabled
events are then lost). Empty strings/slices remain valid. Static metadata and
scalar/slot overhead are additional to copied-byte budgets. Releases MUST document
the complete storage bound: stacks, scratch, reclamation and actual allocator
capacity/overhead. No hidden growing dictionary is permitted.

Filtering uses only a maximum native level and either all targets or an immutable
finite set of exact target strings supplied at setup. It MUST NOT use regexes,
environment parsing, application callbacks, field values, identity resolution or
dynamic allocation. Filtered events/spans MUST NOT invoke formatters or increment
loss counters. Native compile-time filtering remains independent.

## P4. Field contract

Supported values are native booleans, signed/unsigned 64/128-bit integers, f64,
UTF-8 strings and byte slices. Smaller integers retain tracing's canonical value.
Integers MUST NOT narrow or pass through text; floats MUST NOT pass through decimal
formatting (including infinities, NaNs and signed zero). Strings/bytes MUST be
copied before callback return.

Opaque Debug, Display, error, formatted-message and unsupported extension values
MUST reject the entire event without formatting, traversal or application
callbacks. Missing primitive visitor methods MUST NOT fall back to formatting.
Metadata field counts MUST be checked before visitation; excess fields/bytes
reject the entire event, without partial output or truncation.

Field names have no built-in semantics. Application schemas supply fingerprints,
keys, tags and reasons; the crate defines no scheduler, RTI or payload types.
Collection/redaction policy belongs to the application before emission.

## P5. Record and snapshot contract

Records MUST contain static event metadata, all accepted fields, and captured
scope metadata/fields ordered root-to-leaf. Fields remain attached to their event
or scope; duplicate names MUST NOT be flattened, overwritten or conflated.

Records MUST be self-contained: no earlier span-creation record or live/reused
span slot is needed to read them. Explicit-root events have no scope; events with
unavailable required ancestry MUST be rejected, not made root.

Snapshots are owned off-path copies of complete records, oldest-to-newest in
local ring commit order. Rejected events have no sequence position; commit order
does not establish cross-thread/process causality. Readers may allocate and wait,
but MUST NOT prevent slot reuse or make producers wait. Snapshot and counter
observations need not be one atomic view during capture; at quiescence both are
exact unless counters saturated.

Snapshots are an in-process API, not a durable/portable format. Pointers and native
span IDs MUST NOT be advertised as cross-process identities. Export encoding,
label resolution and text formatting occur outside capture.

## P6. Span and producer lifecycle

The subscriber MUST support native parent selection, current-span propagation,
clone/drop, enter/exit and field updates. Context MUST survive entry on another
prepared thread and MUST NOT leak between sibling async tasks entered per poll.

- Handles, active entries and child ancestry references MUST prevent slot reuse.
- Reused slots MUST NOT alias a previous live identity. Generation exhaustion
  retires the slot or fails admission, never wraps.
- Lifetime bookkeeping MUST survive event loss: failed buffer access cannot
  discard clone/drop operations.
- Reclamation MUST be bounded and nonblocking, without unbounded queues or
  recursion through unbounded ancestry.
- Failed span admission MUST propagate invalid context, never a healthy span's
  identity. Invalid handles MUST remain safe to clone, enter, exit, propagate and
  drop without unbounded per-failure slot consumption.
- Failed/unsupported updates invalidate the span; descendants MUST NOT present
  stale fields as successfully updated context.
- Depth overflow or lost enter/exit MUST invalidate affected context. Recovery
  requires a provably balanced known boundary; otherwise invalidity lasts until
  producer teardown. Healthy producers continue.
- Output loss/overwrite MUST NOT change span references or current context.
- Teardown MUST release reusable producer resources and eventually reclaim spans
  with no handles, entries or child references, without invalidating another
  producer's spans. Counter/reference overflow MUST NOT cause premature reuse or
  silent identity corruption.

Updates MUST be atomic to readers: an event sees complete old or new fields, or
rejects unavailable/invalid context, never a partial mixture. Parent traversal
and records obey P3 limits.

`follows_from` requests increment an unsupported-context-operation counter; they
MUST NOT silently appear captured or affect supported parent/child correlation.
No general causal graph, span-duration profiler or arbitrary extension storage is
required.

## P7. Overflow, rejection, and loss status

A successful commit to a full ring replaces exactly its oldest complete record
and increments `overwritten_records`. Failed capture MUST NOT evict prior output.

Every enabled event delivered to the subscriber commits once or increments exactly
one rejection counter:

| Counter | Reason |
| --- | --- |
| `no_record_capacity` | Zero record capacity |
| `contention` | Storage could not be accessed immediately |
| `invalid_context` | Producer/span ancestry cannot be captured correctly |
| `field_limit` | Metadata or inherited total exceeds the field bound |
| `byte_limit` | Copied value bytes exceed their bound |
| `unsupported_value` | Unsupported visited representation |

Validation order MUST be deterministic: capacity, immediately accessible
producer/context state, metadata field counts, then native-order visitation.
Failed storage access takes `contention`; established unavailable context takes
`invalid_context`. During visitation, the first field/byte/unsupported failure
wins; an event MUST NOT count under multiple reasons.

Separate counters cover span admission/update failure, invalid-context transitions
and unsupported context operations. These are not event rejections; a later event
rejected because of a failed span is its own loss. Overwrite is retention loss,
not rejection of the new record.

Loss counters/status MUST be readable without writing the ring or waiting for a
producer, MUST NOT silently wrap, and MUST expose saturation as "at least this
many." Filtering is not rejection. Invalid/stopped producers MUST NOT appear
loss-free: invalid status persists until recovery/teardown, and transition counts
survive teardown.

Capture failure MUST NOT fail application/coordination, panic on capacity
exhaustion, wait for space, log recursively or replace the application's first
failure cause. First-failure records are not protected from later overwrites.

## P8. Conformance evidence

Tests MUST emit native events/spans and assert public snapshot/loss output.
Private hooks may arrange contention, not replace that observation boundary.
Timing averages do not prove nonblocking behavior, nor do concurrency tests
replace lifetime/context inspection.

| Group | Mandatory checks |
| --- | --- |
| Interoperability | Native macros and conforming hand-built value sets; explicit/root/contextual parents; independent consumer |
| Filtering | Exact targets/levels; excluded fields not formatted or counted as loss |
| Values | Integer extrema; non-finite floats; exact strings/bytes; every primitive visitor; duplicate scope field names |
| Formatter rejection | Panic/side-effect sentinels never execute; whole-event rejection |
| Capacity | Exact field/byte limits; zero records; overwrite order; rejection preserves output; saturation |
| Allocation | Fresh-process single-subscriber first/repeated capture after preparation, including failures; no allocation/deallocation |
| Closed callsites | Process-wide `C`; version/feature/registration audit; concurrent first-use output, not timing-based proof |
| Contention | Producers return before held storage is released; visible loss; inspection cannot force producer waiting |
| Context | Cross-thread propagation; nested/repeated entries; sibling async polls; atomic updates; parent/depth loss; invalid handles/recovery |
| Lifecycle | Clone/drop under contention; child outlives parent handle; generation reuse; cross-thread teardown; repeated capacity reclamation |
| Trace off | Compiled-off tracing avoids field evaluation and adds no required subscriber dependency |
| Publication | Independent packaged build/tests/docs; normative docs/licenses; executable examples |

The audit MUST cover native first-use registration, dispatcher access and TLS
initialization. Known allocation or blocking on a supported path fails the
requirement; it is not an implicit exception.

### Q1. Closed-callsite first-use qualification

For audited `tracing 0.1.44` / `tracing-core 0.1.36`, native macros use static
callsites registered once. Strong-CAS list insertion only gains distinct nodes:
each failed attempt implies another insertion. At most `C` callsites therefore
bound one insertion to `C - 1` failures and one success. Same-site competitors
do not wait for registration to finish. This bounds source-level insertion
attempts, not all atomic operations, instructions, hardware retries or time.
See [QUALIFICATION.md](QUALIFICATION.md) for source locations and test evidence.
Development follows workspace dependency versions, not independent exact pins;
the argument remains conditional on the audited resolved versions and features.

The deployment MUST:

- Establish `C` for the entire process, including dependency and filtered sites;
  producer count, observed executions or application-only searches are insufficient.
- Identify the artifact, build features, inventory method and dependency
  instrumentation. A conservative bound is sufficient, but a caller-supplied
  number MUST NOT be represented as a runtime-enforced registration cap.
- Audit application and dependency producers for P2's value-set shape; metadata
  counts alone do not bound arbitrary hand-built value sets.
- Exclude dynamic/leaked callsites, runtime-loaded instrumentation, custom
  registration and manual duplicate registration during qualified execution.
- Install one fixed dispatcher during setup; do not rebuild interest caches,
  construct competing dispatchers or replace subscribers during capture.
- Record resolved dependency versions/features and audit consumer feature
  unification. The initial lane MUST disable `log` and `log-always`; forwarding
  MUST NOT add formatting, allocation or blocking.
- Re-audit assumptions on dependency updates; semver compatibility alone does not
  qualify another version.

No replacement macros, callsite warm-up or upstream fork is required. This proof
does not qualify producer preparation, field visitation, span/dispatch lifetimes
or subscriber callbacks. Without the deployment evidence, no whole-path bound may
be claimed. Callback-only guarantees or mandatory callsite preparation would
require a separately reviewed profile change.

## P9. Compatibility and release claims

Before publication, replace scaffold status with implemented-profile, toolchain,
target, dependency-feature, complete-memory-bound and conformance evidence.
Distinguish setup allocation from capture allocation; hosted results MUST NOT
imply embedded qualification.

Versioning is independent of the host application. Changes to loss/overflow,
field representations, context guarantees or preparation are public-contract
changes even without Rust API changes; release notes MUST identify them and
apply the declared semantic-versioning policy.

No internal layout or wire ABI is frozen. Future serialization, platform backends
or stricter allocation profiles require separate compatibility/conformance rules.
