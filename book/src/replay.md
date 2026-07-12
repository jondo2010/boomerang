# Recording and Replay

Recording and replay make nondeterministic or unavailable parts of a system
repeatable. Boomerang treats recording at two complementary boundaries.

## Physical-Boundary Recording

Physical-boundary recording captures inputs from outside the deterministic
reactor graph: sensors, clocks, operators, hardware interrupts, and external
systems. Physical actions are the intended entry point for these inputs.

A recording preserves each value and its complete logical tag. Replaying those
inputs into the same graph should reproduce the same observable logical trace,
subject to the documented runtime and platform assumptions.

## Deployment-Boundary Recording

Deployment-boundary recording captures messages crossing an enclave or
federate interface. It allows CI to replace a partition—such as a sensor ECU or
planning subsystem—with trace-backed endpoints while the rest of the graph
runs live.

For a one-way producer, replay injects the recorded outbound messages at their
original tags. For a bidirectional or feedback interface, a useful recording
contains both directions: replay supplies the replaced partition's outputs and
validates that the live system produces inputs compatible with the recorded
interaction. A static recording cannot respond correctly to novel inputs; that
requires a behavioral model rather than replay.

## Recording Contract

Portable recordings should use stable logical identities rather than runtime
allocation keys. A boundary event needs at least:

- the stable endpoint or action identity;
- direction and payload schema;
- the full logical tag, including microstep; and
- deterministic ordering information for events sharing a tag.

Recordings may also carry a graph or interface fingerprint so incompatible
graphs fail clearly instead of producing misleading results.

## Current Status

Boomerang currently has action recording/replay foundations backed by MCAP.
Stable deployment-independent identities, full boundary recording, partition
substitution, and deployment-equivalence trace comparison are architectural
goals and are not yet complete. In particular, deterministic replay must
preserve microsteps and must not depend on enclave or action keys that can
change when a graph is repartitioned.
