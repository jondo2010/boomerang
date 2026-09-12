# Boomerang central RTI

`boomerang_central_rti` supplies the Tokio-backed, central RTI executor for
hosted static Boomerang federations.
It owns federate clients, transports, central sessions, and the static runner
that adapts compiled runtime state to asynchronous I/O.
Legacy protocol, topology, and RTI coordination remain in `boomerang_federated`;
compiled image-backed coordination lives here and scheduler execution remains in
`boomerang_runtime`.

## Compiled execution

`compiled::CompiledRti` borrows the immutable central-RTI projection and materializes
only mutable member state. It uses precomputed dependencies; canonical federation
analysis remains in the compiler. Stable member and boundary identities are bound
once to their distinct runtime key domains. `CoordinationIdentity` is the shared
artifact digest supplied at admission; artifact generation supplies it in the next
slice.

`compiled::CentralRtiClient` connects prepared compiled boundary adapters to ordered
request/reply interfaces. Fresh local mailbox fences protect externally published
lower bounds and aggregate completion. Payloads precede grants on the reply stream. Grants are
retained horizons; payload validation uses delay-adjusted bounds, including the
microstep collapse caused by positive delay. Local idle stays reversible until all
members explicitly participate in terminal quiescence. Admission and stop waits
are bounded; local failure aborts the session. Graceful stop currently requires
global quiescence; an early local stop fails the session.

The compiled path contains no in-memory transport. Its channel wiring is isolated
in `boomerang/tests/compiled_reference/central_rti_transport.rs`, solely for tests,
and is explicitly outside the intended architecture hot path. The production I/O
owner must enforce ordered delivery, report transport loss through `abort`, and
own connection deadlines. Generated RTI/Federate artifacts, production transport,
and child-process supervision belong to the final #131 slice. Existing legacy
clients and transports remain separate pending #132; the transitional `federated`
feature gate does not select the compiled protocol or its semantic model.
