# boomerang_federated

Protocol, codec, transport, RTI state-machine, and runtime bridge primitives for
Boomerang federations. `RuntimeFederation` contains independent RTI topology
data and self-contained `RuntimeFederate` compute nodes, each owning one or more
scheduler Enclaves.

This crate depends on the protocol-neutral scheduler extension points in
`boomerang_runtime`; the core runtime does not depend on this crate or expose a
federation feature.
