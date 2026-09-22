# boomerang_telemetry

Portable, allocation-free telemetry record types and a caller-buffered canonical
Postcard codec. The crate is `no_std`, has no default features, and owns no
runtime observation state, transport, socket, executor, clock, tracing, replay,
or application-value interface.

Each encoded record is bounded by 1,200 bytes. Callers supply output storage for
encoding and scratch storage for canonical decoding; decode scratch must be at
least as long as its input. Stable process, Federate, and Enclave identities are
borrowed from the input or caller. Scheduler lifecycle and phase are
adapter-defined `u8` codes; this crate deliberately assigns no meanings to them.

`TelemetryEncoder` owns one `TelemetryIdentity`, independent scheduler and
exporter-health sequences, and saturating publication-drop and snapshot-miss
counters. Callers supply monotonic sender and observation timestamps, samples,
and output storage. A sequence advances only after successful encoding; each
record group stops after its successful `u64::MAX` record without affecting the
other group.
