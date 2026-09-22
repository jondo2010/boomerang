# boomerang_telemetry

Portable, allocation-free telemetry record types and a caller-buffered canonical
Postcard codec. The crate is `no_std`, has no default features, and owns no
runtime observation state, transport, socket, executor, clock, tracing, replay,
or application-value interface.

Each encoded record is bounded by 1,200 bytes. Callers supply output storage for
encoding and scratch storage for canonical decoding; stable process, Federate,
and Enclave identities are borrowed from the input or caller.
