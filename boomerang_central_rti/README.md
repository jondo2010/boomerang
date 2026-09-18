# Boomerang central RTI

`boomerang_central_rti` supplies compiled central coordination for hosted Boomerang
federations. It owns image-backed RTI state, ordered protocol clients, and hosted TCP
transport. Pure wire tag and delay primitives remain in `boomerang_federated`;
scheduler execution remains in `boomerang_runtime`.

## Compiled execution

`compiled::CompiledRti` borrows the immutable central-RTI projection and materializes
only mutable member state. It uses precomputed dependencies; canonical federation
analysis remains in the compiler. Stable member and boundary identities are bound
once to their distinct runtime key domains. `CoordinationIdentity` is the shared
compiler-issued coordination fingerprint supplied at admission, distinct from
each Federate's image fingerprint and executable artifact digest.

`compiled::RtiClientBindings` groups the validated RTI projection, member, and coordination
identity during preflight. It resolves outbound boundaries and maps all incoming RTI
routes to prepared Enclave-local adapters before admission. `CentralRtiClient` then
verifies coordination identity before scheduler startup. Normal payload exchange uses
`RtiRouteIndex` exclusively: no stable string IDs, string cloning, or name lookup.
Member and route domains remain distinct; every request checks route existence and
source ownership. Artifact generation must bind the supplied identity to the exact image.

`compiled::CentralRtiClient` connects these prepared adapters to ordered request/reply
interfaces. Fresh local mailbox fences protect externally published
lower bounds and aggregate completion. Payloads precede grants on the reply stream. Grants are
retained horizons; payload validation uses delay-adjusted bounds, including the
microstep collapse caused by positive delay. Local idle stays reversible until all
members explicitly participate in terminal quiescence. Admission and stop waits
are bounded; local failure aborts the session. Graceful stop currently requires
global quiescence; an early local stop fails the session.

`compiled::in_memory` provides reusable channel adapters for testing and reference
execution, explicitly outside the intended production hot path. Fixture-specific
worker setup and fault injection remain in the integration tests. The production I/O
owner must enforce ordered delivery, report transport loss through `abort`, and
own connection deadlines.

`compiled::hosted` provides bounded framed TCP connections and generated RTI/Federate
artifact entry points. Generated launcher child-process supervision belongs to
`cargo-boomerang`. Compiled runtime and boundary APIs are available unconditionally;
selecting the central backend adds this crate only to deployments that need it.

## Coordination tracing

The `boomerang::coordination` target follows reactions, encoding, transport,
RTI decisions/grants/accounting, receiver admission, and shutdown/failure.
These are ordinary `tracing` events, not replay records: application payloads
and arbitrary error strings are never captured. The returned execution error
retains the first cause; the trace identifies where it first became terminal.

Fields use native values, without producer-side Debug/Display formatting:

- `coordination`: the 32 canonical coordination-fingerprint bytes.
- `federate` and `destination`: `FederateIndex`; `enclave`: deployment-global
  `EnclaveIndex`; `route`: `RtiRouteIndex`.
- `local_route`, `port`, and `reaction`: the emitting Enclave's `RouteIndex`,
  `PortIndex`, and `ReactionIndex`. Keys are unsigned numbers; field names and
  inherited Federate/Enclave scopes preserve their distinct domains.
- `tag_kind`: `finite`, `never`, or `forever`; finite tags include signed
  128-bit `tag_offset_ns` and the full `tag_microstep`. The same suffixes apply
  to `source_tag`, `next_event`, and `dnet`. An absent publication uses
  `next_event_kind = "none"`. Non-finite numeric components, if present, are
  representation details and must not be interpreted as finite time.

Formatting is subscriber policy: hosted JSON currently renders event fingerprint
bytes as hex strings, span bytes as arrays, and 128-bit offsets as decimal strings.
Native bounded records preserve bytes and integers without that conversion.

For custom executables, enable `bounded-tracing`, install a
`tracing_bounded::BoundedSubscriber`, and hold a prepared producer guard on every
application-owned emitting thread, including the thread serving the RTI.
Prepare before creating or entering `RtiClientBindings::execution_span()`.
Runtime-owned coordination/scheduler workers and hosted client I/O workers prepare
themselves before entering inherited spans. Setup failure counts as diagnostic
loss; it does not fail scheduling or transport. No subscriber is installed by
these libraries. Without the feature, they have no `tracing-bounded` dependency.

Filter bounded capture to `boomerang::coordination`; other targets, including
hosted runtime-construction diagnostics, are not yet native-field qualified.
See the [launcher modes](../boomerang_util/README.md#launcher-trace-modes) for
generated executables. Bounded output can be incomplete under contention or
capacity pressure; always inspect both event and lifecycle loss counters.
