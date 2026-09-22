# boomerang_telemetry

Internal, allocation-free telemetry record types and a caller-buffered canonical
Postcard codec. Producers and consumers form a closed system deployed atomically
from the same build. Scheduler records carry `boomerang_runtime::ObservationSnapshot`
directly, including its typed lifecycle and phase. Runtime payload coupling is
intentional; snapshot changes evolve with the deployment, without a shadow schema
or compatibility mapping. The record version identifies the envelope and does not
promise compatibility across independently deployed runtime versions.

The crate uses `no_std` and has no default features, but its required runtime
dependency currently needs `std`. Encoding itself uses no allocation. The crate
owns no runtime observation state, transport, socket, executor, clock, tracing,
replay, or application-value interface.

Each encoded record is bounded by 1,200 bytes. Callers supply output storage for
encoding and scratch storage for canonical decoding; decode scratch must be at
least as long as its input. Stable process, Federate, and Enclave identities are
borrowed from the input or caller.

`TelemetryEncoder` owns one `TelemetryIdentity`, independent scheduler and
exporter-health sequences, and saturating publication-drop and snapshot-miss
counters. Callers supply monotonic sender and observation timestamps, snapshots,
and output storage. A sequence advances only after successful encoding; each
record group stops after its successful `u64::MAX` record without affecting the
other group.
