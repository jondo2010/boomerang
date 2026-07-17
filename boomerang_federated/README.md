# boomerang_federated

Protocol, codec, transport, RTI state-machine, and runtime bridge primitives for
Boomerang federations. `RuntimeFederation` owns the dense runtime Enclave map and independent RTI
topology data. Each `RuntimeFederate` records the Enclave keys and protocol bridge for one compute
node.

This crate depends on the protocol-neutral scheduler extension points in
`boomerang_runtime`; the core runtime does not depend on this crate or expose a
federation feature.
