# boomerang_federated

Protocol, codec, transport, RTI state-machine, and runtime bridge primitives for Boomerang
federations. `RuntimeFederation` owns the compiled RTI topology and a deterministic map of
`RuntimeFederate` values. Each `RuntimeFederate` is an owned pre-execution bundle containing one
compute node's dense Enclave map and protocol bridge. A runner consumes those bundles to start the
Enclave schedulers and protocol clients; the RTI itself receives topology and transport endpoints.
Stable Federate and endpoint IDs remain on the wire while compiled RTI internals use crate-private,
process-local dense keys.

This crate depends on the protocol-neutral scheduler extension points in
`boomerang_runtime`; the core runtime does not depend on this crate or expose a
federation feature.
