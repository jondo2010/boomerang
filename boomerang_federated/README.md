# boomerang_federated

Portable `WireTag`, `WireDelay`, bounded canonical frames, and payload codecs for
compiled Boomerang coordination. Tags use explicit infinity sentinels, nanosecond
offsets, and architecture-independent microsteps. Tag serialization and canonical wire codecs are
unconditional; the `serde` feature additionally enables `WireDelay` serialization.

`wire` borrows caller-owned buffers and compiled typed member/route tables. Its
session admits an exact closed-world channel profile before dense references and
fails closed on errors. `PostcardCodec` supports sealed allocation-free values,
with compile-time encoded limits and caller scratch for canonical validation.

The crate owns no topology, sockets, queues, RTI grant state, or scheduler.
Hosted I/O and task orchestration belong to `boomerang_central_rti`. The wire
algorithms allocate no storage; making all dependencies `no_std` remains Phase 7.
See [the protocol](../docs/federated-protocol.md) for the wire format and bounds.
