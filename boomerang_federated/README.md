# boomerang_federated

Protocol, topology, payload codec, and RTI state-machine primitives for
Boomerang federations.

This pure crate owns `RtiState`; Tokio transport and execution belong to
`boomerang_central_rti`.

This crate is intentionally separate from `boomerang_runtime`. It does not
start sockets, processes, or schedulers in the Milestone 2 slice.
