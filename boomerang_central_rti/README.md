# Boomerang central RTI

`boomerang_central_rti` supplies the Tokio-backed, central RTI executor for
hosted static Boomerang federations.
It owns federate clients, transports, central sessions, and the static runner
that adapts compiled runtime state to asynchronous I/O.
Protocol, topology, payload codecs, and pure RTI coordination remain in
`boomerang_federated`; scheduler execution remains in `boomerang_runtime`.
