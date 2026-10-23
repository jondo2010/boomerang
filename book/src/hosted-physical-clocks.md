# Physical clocks

Boomerang uses the host monotonic clock by default. Select a manual clock when
an application or simulator controls physical time. All Enclaves in a compiled
Federate share the selected clock and execution epoch.

Clock selection and input delivery are separate. Ordinary asynchronous events
and timestamped source observations can use either clock.
See [External inputs](./external-inputs.md) to choose an input contract.

## Select and advance a manual clock

For manual time, add this table to the Federate configuration:

```toml
[deployments.production.federates.host.physical-clock]
domain = 7
```

A component that requests `clock_control = true` needs this clock selection.
For a component that only controls the clock, declare
`init_context(name = ClockInit, clock_control = true)`.
No input configuration is needed for that component. Declare `state` and
`state_init` as described in [Reactor initialization](./reactor-initialization.md).
Only one component initializer can request clock control for a Federate.

The initializer receives `clock: ManualClock` in its generated context.
For example, a worker can advance time to 10 ms:

```rust,no_run
use boomerang_runtime::clock::{ManualClock, PhysicalClockError, PhysicalInstant};
use core::time::Duration;

fn advance(clock: &ManualClock) -> Result<(), PhysicalClockError> {
    clock.advance_to(PhysicalInstant::from_duration(Duration::from_millis(10))?)
}
```

Call `advance_to` with a `PhysicalInstant` on the selected timeline.
While the clock is active, an equal timestamp leaves time unchanged.
A later timestamp advances time. A backward change fails the run.
A forward jump preserves all intervening timer tags.

`close` and `fail` terminate the clock and release waiting execution.
`fast_forward` skips physical pacing, but clock readings still use the selected
clock.

For host monotonic time, omit `physical-clock` and `clock_control`.
The toolchain rejects obsolete clock options `entry`, driver `binding`, and
nested `inputs`.

## Read time and identify an execution

`PhysicalInstant` stores unsigned nanoseconds from the execution's physical
origin. The clock domain identifies the timeline. The execution epoch identifies
one execution on that timeline. Compare timestamps only within the same clock
domain and execution epoch.

Sources that use the host clock use domain zero and a fresh execution epoch.
`source.clock()` provides these identities and time readings without manual
clock control.

The `boomerang_runtime::context` module exports `Context`, `SendContext`, and
`CommonContext`. Use `Context` inside reactions. Use `SendContext` in workers
and callbacks. Import `CommonContext` to use their shared methods.

Both context types provide `try_get_physical_time()` through `CommonContext`.
The other methods in this table belong to `Context`:

| Method | Result |
| --- | --- |
| `try_get_physical_time()` | Current time from the selected clock. |
| `get_start_time()` | Epoch coordinate zero. |
| `try_get_logical_time()` | Current logical tag converted to physical coordinates, with range checking. |
| `get_elapsed_logical_time()` | Elapsed logical time with the signed logical range. |
| `get_tag()` | Current logical tag with the signed logical range. |

A context that uses the host clock without an initializer context returns
`NotStarted` until scheduler startup establishes its origin. Start input producers from a startup
reaction if they need that origin before sending.

## Coordinate clock time and source progress

Clock advancement does not report source progress. A required source can hold
back logical execution even when the clock has advanced. Publish observations
before their corresponding progress. See
[Report source progress](./external-inputs.md#report-source-progress).

Use `std::time::Instant` for host watchdogs, connection timeouts, and shutdown
bounds. These measurements continue independently of manual clock advancement.

## Supported deployments and features

Manual clock control requires compiled execution with one local hosted `std`
Federate and one or more Enclaves. Distributed manual clock execution is unsupported.

Enable the required features on the `boomerang_runtime` dependency of each
payload package that uses these APIs:

| Use | Cargo feature |
| --- | --- |
| Host monotonic time with ordinary sending | Neither hosted feature |
| Source admission with host monotonic time | `hosted-source-input` |
| Manual clock control without source admission | `hosted-manual-clock` |
| Source admission with manual clock control | Both features |

Application manifests select each feature explicitly. `external-clock` is a compatibility alias that enables
both features. Neither hosted feature is supported on constrained targets such
as Pico.

The live builder and runner use the host monotonic clock.
See the [clock API](https://docs.rs/boomerang_runtime/latest/boomerang_runtime/clock/index.html)
for type and method details. The
[complete input example](./external-inputs.md#run-the-complete-example)
combines a manual clock with a timestamped source.
