# boomerang_federated

Portable `WireTag` and `WireDelay` primitives for compiled Boomerang coordination.
Tags use explicit infinity sentinels, nanosecond offsets, and architecture-independent
microsteps. Checked delay arithmetic preserves zero-delay microsteps and resets them
for positive delays. The optional `serde` feature enables serialization.

This pure crate owns no topology, session state, payload codec, transport, or scheduler.
Compiled central coordination and its hosted transports live in `boomerang_central_rti`.
