# Project Goals and Status

Boomerang's goal is to let teams describe deterministic application behavior
once and preserve that behavior while changing how the system is tested and
deployed.

## Intended Users

Boomerang is aimed at engineers building robotics and embedded systems where:

- independently developed subsystems must compose through explicit interfaces;
- the same product may span several cores, processes, and ECUs;
- CI must exercise meaningful production behavior on a single host;
- recorded sensors or subsystems must be reusable in regression tests; and
- timing, ordering, and unsupported semantics must be explicit.

Reactors should be reusable without embedding assumptions about their eventual
host, transport, or partition. Deployment configuration assigns a completed
reactor graph to enclaves, federates, processes, and machines.

## Design Goals

- **Deterministic behavior:** preserve logical tags, ordering, delays, mode
  transitions, and shutdown behavior across supported deployments.
- **Late-bound deployment:** separate graph composition from placement across
  threads, cores, processes, and ECUs.
- **Component reuse:** make typed reactors independently testable and
  hierarchically composable across products.
- **Local-to-target continuity:** run the same graph monolithically, with its
  production partitioning on one CI host, and on distributed targets.
- **Recording and replay:** capture nondeterminism at physical boundaries and
  substitute partitions using deployment-boundary recordings.
- **Team-scale integration:** use stable identities, explicit schemas, and
  early graph validation to support parallel development.
- **Explicit failure semantics:** reject unsupported distributed behavior
  rather than silently weakening determinism.
- **Evidence-producing execution:** compare stable logical traces across
  deployment and replay configurations.

## Available Today

Boomerang currently provides:

- a deterministic logical-time scheduler;
- typed reactor composition in Rust;
- local enclave execution;
- modal reactors;
- recording and replay foundations; and
- experimental static in-memory and single-process TCP federation.

The current federation implementation is intentionally conservative. See
[Static Federation](./static-federation.md) for its supported subset.

## Long-Term Direction

The project intends to add deployment-independent graph partitioning,
boundary-layer recording and substitution, multi-process and multi-ECU
execution, and platform backends suitable for mixed-criticality embedded
products.

The desired platform range includes `std` environments such as embedded Linux
and QNX, targets based on an RTOS such as Zephyr, and, where feasible, a
portable `no_std` runtime core. Host-side graph construction and validation do
not need to become `no_std`; constrained targets may instead execute a
statically lowered runtime plan with platform-provided clock, synchronization,
execution, storage, and transport services.

Mixed-criticality and safety-critical use are long-term goals. Boomerang does
not currently claim hard real-time bounds, temporal or memory isolation, WCET
or schedulability analysis, ASIL/SIL compliance, a qualified toolchain, or
safety certification. Future claims must be backed by explicit scheduling and
resource policies, fault-containment behavior, platform assumptions, tests,
and reviewable evidence.
