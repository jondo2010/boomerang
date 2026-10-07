# Boomerang Runtime

[![crates.io](https://img.shields.io/crates/v/boomerang_runtime.svg)](https://crates.io/crates/boomerang_runtime)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](./LICENSE)
[![Downloads](https://img.shields.io/crates/d/boomerang_runtime.svg)](https://crates.io/crates/boomerang_runtime)
[![CI](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml)
[![docs](https://docs.rs/boomerang_runtime/badge.svg)](https://docs.rs/boomerang_runtime)
[![codecov](https://codecov.io/github/jondo2010/boomerang/graph/badge.svg?token=PYXF8VSNY9)](https://codecov.io/github/jondo2010/boomerang)

Runtime types and discrete event executor for Boomerang.

This crate is not intended to be used by itself, but rather through the top-level [`boomerang`](https://docs.rs/boomerang) crate.

## Hosted physical clocks

The optional `external-clock` feature lets a compiled, local Federate share one
`physical_clock::ManualClock`. Create it with a `PhysicalClockDomainId`, retain a
clone in the driver, and select it through
`FederateBindings::with_physical_clock(domain, clock)`. Every Enclave receives the
same clock and fresh random `ExecutionEpoch`. A clock is consumed by one run;
domain mismatch, reuse, closure, or a mailbox without a retained wake slot fails
before scheduler startup. `physical_time` contains only integer types and a
minimal read trait; hosted `HostClock` implements that contract for adapters
sharing a Federate's host origin.

`advance_to` accepts forward jumps and repeated timestamps. Regression, `close`,
and `fail` retain the first terminal error and release physical and coordination
waits through the existing Federate abort path. One preallocated deadline slot
per Enclave is reused without steady-state allocation. A jump wakes each due
Enclave once; its event queue still executes every eligible timer in tag order.
This may produce a burst of work. `fast_forward` bypasses pacing independently of
clock selection; reaction physical time and physical actions still use the
selected clock.

Reactions and send contexts expose the selected integer offset through
`CommonContext::physical_time` (`None` for ordinary host-clock execution).
`try_get_physical_time`, `try_schedule_action`, and
`try_schedule_action_async` report checked conversion and terminal errors.
The legacy `get_physical_time` returns a checked, origin-relative `Instant` and
panics on failure; this compatibility value is unsuitable for watchdogs.
Supervision and transport liveness continue to use host `Instant::now()`.

Generated launchers can declare the clock on their one local hosted Federate:

```toml
[deployments.production.federates.host.physical-clock]
domain = 7
binding = "sensor"
entry = "drive_clock"
```

`binding` selects an existing payload package; `entry` is a function path from
that crate's root. It receives a clone of the selected `ManualClock` and returns
`Result<Guard, E>`; the launcher retains `Guard` through execution. The driver
must start its own advancement mechanism or advance before returning. Clock
selection, domain, and driver participate in artifact fingerprints; random epochs
do not. Non-host targets, custom target JSON, non-`std` runtimes, and distributed
clock configuration are rejected. Feature-off builds contain no hosted clock
state or scheduler branches. The future Pico scheduler is not a prerequisite;
its own scheduler exclusion proof remains deferred.

## Hosted physical inputs

With `external-clock`, `FederateBindings::with_physical_inputs` adds an `InputConfig`
sidecar. Declare stable source names, required/optional status, and typed physical
**action** targets using Enclave and payload-binding identities. Targets are owned
exclusively by the adapter: ordinary reaction/driver publication must use separate
actions. Resolution validates types and reads minimum delays before any initializer
or driver starts. The startup callback receives this execution's `InputAdmission`;
it must return promptly and can pass cloned handles to the application's drivers.
Required sources keep an idle Federate alive; shutdown and configured logical
horizons still use the existing coordinator and selected physical clock.

Resolve `source(name)` and `target(source, name)` once. Submit owned decoded
`InputObservation` values with source sequence, clock domain, epoch, and acquisition
time. `submit(batch, progress)` validates the entire batch, publishes at most one
envelope per destination Enclave, then commits exclusive progress. Receipts retain
host arrival separately from mapped tags, which use checked acquisition time plus compiled minimum delay. Diagnostic host arrival
times never affect mapping. Targets cannot silently replace a pending value at the
same tag, even across batches; batch and retained-batch bounds return explicit
overflow. Retention is released when destination tags complete.

`advance(source, F)` promises that future acquisition times are **not less than F**.
An observation at F remains admissible. Required sources gate all finite tags until
they publish a frontier; the minimum predecessor of `F + delay` caps existing
Federate grants. Optional sources never gate progress. Periodic and idle sources
must publish explicitly. Frontiers may exceed clock time; observations may not.
`InputErrorKind` distinguishes malformed, future, late (including in-flight),
duplicate, out-of-order, protocol, overflow, and disconnected outcomes. Required
protocol failures, overflow, or disconnection abort; optional disconnection retains
committed observations. Partial fan-out aborts before a batch can execute.

Generated launchers accept `physical-clock.inputs` with `max_batch_values`,
`max_staged_batches`, and `sources.<name>` containing `required` and
`targets.<name> = { enclave = "sensor", binding = "action/sensor/sample" }`.
The configured driver then takes `(ManualClock, InputAdmission)` and returns
`Result<(), InputError>`. Declarations affect fingerprints and inherit the hosted,
local, host-target restriction. All admission state and grant checks compile out
when `external-clock` is disabled. Transport, device decoding, simulator stepping,
and constrained scheduler integration remain separate capabilities.
