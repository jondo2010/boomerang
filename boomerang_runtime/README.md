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
