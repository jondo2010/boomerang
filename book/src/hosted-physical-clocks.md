# Hosted Physical Clocks

Use an externally advanced clock when a simulator or another driver supplies
physical time for a local deployment. Without a clock selection, Boomerang uses
the host's monotonic clock. Every Enclave in the selected Federate observes the
same externally supplied time.

## Deployment configuration

Add a `physical-clock` table to the hosted Federate in your `Boomerang.toml`:

```toml
[deployments.production.federates.host.physical-clock]
domain = 7
binding = "sensor"
entry = "drive_clock"
```

This fragment extends an existing deployment. The `host` Federate must use the
`std` runtime, and `sensor` must already name a payload binding under
`deployments.production.bindings`.

- `domain` identifies the physical clock domain shared by the driver and its data.
- `binding` selects the payload package containing the driver.
- `entry` names the driver function relative to that package's crate root.

Generated launchers enable the runtime's `external-clock` feature. A payload
package that directly uses clock APIs must also enable that feature on its
`boomerang_runtime` dependency.

The clock-only driver receives a `boomerang_runtime::clock::ManualClock` and
returns `Result<Guard, E>`. The launcher retains the returned guard throughout
execution. The driver must return after advancing time or starting its own
advancement mechanism; blocking inside the entry function prevents scheduler
startup. Keep a clone of the clock in the driver to advance it during execution.

Clock selection, domain, and driver configuration participate in generated
artifact fingerprints. The runtime creates a fresh execution identity for each
run; that identity does not change the artifact fingerprint.

## Driving time

`PhysicalInstant` is a point on the selected physical clock's timeline, much like
`std::time::Instant` is a point on the host monotonic clock. Its coordinate is
nanoseconds since this execution's physical epoch; zero is the epoch origin.
Compare values only within the same clock domain and execution epoch. Adapters
carry those identities separately; the numeric timestamp cannot validate them.
Use `Duration` for elapsed intervals and delays, and `PhysicalInstant::checked_add`
to compute a later time without advancing the clock.

Reaction and send contexts expose the selected timeline through
`try_get_physical_time() -> Result<PhysicalInstant, PhysicalClockError>`.
Physical events and asynchronous action scheduling use the same coordinates;
manual time does not need to fit the host's `std::time::Instant` representation.
A native send context created before execution returns `NotStarted` until its
scheduler starts. Start external producers from a startup reaction, or call
`Scheduler::startup()` before signaling a producer that the scheduler is ready.
Use `try_schedule_action_async` when the producer needs to handle this error.

`Context::get_start_time()` returns the epoch coordinate, zero.
`try_get_logical_time()` projects the current logical offset into a
`PhysicalInstant`, returning an error if it is outside the physical range.
`get_elapsed_logical_time()` and `get_tag()` retain the full signed logical range.
Use `std::time::Instant` independently for host watchdogs and elapsed run timing.

Call `advance_to(PhysicalInstant(n))` with unsigned nanoseconds since the start
of this execution. Equal timestamps are accepted, and forward jumps are allowed.
Timers due within a jump still execute in logical-tag order, so a large jump can
produce a burst of work. Moving time backwards fails the run. `close` and `fail`
terminate the clock and release waiting execution.

`fast_forward` bypasses waiting for physical time. Reactions and physical actions
still read the selected clock, so enable it only when that separation is intended.
Use the host monotonic clock for driver watchdogs and connection timeouts.

Physical events arriving behind completed logical work move to a later
microstep. Multiple physical values for an action at the same offset retain
separate microsteps. Time or microstep overflow produces an error.

## Supported deployments

External clock selection currently supports one local hosted Federate, which can
contain multiple Enclaves. Non-host targets, custom target JSON, non-`std`
runtimes, and distributed clock configuration are rejected. Simulator stepping,
device decoding, and transport belong to the driver or adapter.

See the [clock API documentation](https://docs.rs/boomerang_runtime/latest/boomerang_runtime/clock/index.html)
for programmatic binding, checked context APIs, and runtime implementation details.

## Timestamped physical inputs

When a driver supplies observations as well as time, declare the sources and
physical-action targets beneath the same `physical-clock` table:

```toml
[deployments.production.federates.host.physical-clock.inputs]
max_batch_values = 16
max_staged_batches = 8

[deployments.production.federates.host.physical-clock.inputs.sources.plant]
required = true

[deployments.production.federates.host.physical-clock.inputs.sources.plant.targets.sample]
enclave = "sensor"
binding = "action/sensor/sample"
```

The target must identify an existing physical action with the matching payload
type. Reserve it for this adapter; ordinary reactions and other drivers must use
separate actions. `max_batch_values` bounds values in one submission, and
`max_staged_batches` bounds batches retained until their destination tags finish.
Exceeding these bounds returns an explicit error.

With `inputs` configured, the driver entry takes `(ManualClock, InputAdmission)`
and returns `Result<(), InputError>`. It must return promptly and can hand cloned
handles to application-managed drivers. Resolve source and target names once,
then submit decoded observations with a source sequence, domain, execution epoch,
and acquisition time. Acquisition time, plus the action's configured minimum
delay, determines the logical tag; host arrival time is diagnostic only.

Use `submit(batch, progress)` to publish observations before their progress.
`advance(source, F)` promises that future acquisition times will be at least F;
an observation exactly at F is still allowed. Required sources hold back logical
execution until they report progress. Optional sources do not hold it back.
Periodic and idle required sources must publish progress explicitly, even when
there are no new observations. Progress can exceed current clock time, but an
observation cannot be acquired in the future.

Clock advancement and input progress are separate: advancing the clock alone
cannot release execution past unresolved required input. Required sources keep
an idle Federate alive; explicit shutdown and configured logical horizons still
apply. Malformed, future, late, duplicate, out-of-order, protocol, overflow, and
disconnected submissions have distinct errors. Required protocol failures,
overflow, or disconnection abort execution; optional disconnection retains
already committed observations.

Input declarations participate in artifact fingerprints and have the same local
hosted deployment restrictions as the clock. See the
[physical-input API documentation](https://docs.rs/boomerang_runtime/latest/boomerang_runtime/physical_input/index.html)
for admission, reservation, and progress implementation details.
