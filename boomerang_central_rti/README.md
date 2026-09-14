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
artifact digest supplied at admission and embedded by artifact generation.

`compiled::RtiClientBindings` groups the validated RTI projection, member, and artifact
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
