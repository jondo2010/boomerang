# External inputs

Use physical actions to deliver keyboard events, callback results, or sensor data.
Choose ordinary event delivery when the scheduler can adjust late input.
Choose source admission when acquisition times must determine exact logical tags.
Both contracts work with the host monotonic clock or a manual clock.

Create the action handles in a
[reactor initializer](./reactor-initialization.md).
Clock selection is described in [Physical clocks](./hosted-physical-clocks.md).

## Choose an input contract

Choose ordinary event delivery when an input can run after work that has already
executed. Keyboard events and callback results usually fit this contract.
Use `AsyncActionRef<T>` with `SendContext`.

Choose source admission when an acquisition timestamp determines the exact
logical time of the input. Use `SourceAction<T>` with `InputSourceHandle`.
This contract is useful for timestamped sensor streams, including streams that
use host monotonic time.

| Behavior | Ordinary event delivery | Source admission |
| --- | --- | --- |
| Timestamp | Captured during preparation. | Supplied with the observation. |
| Late input | The scheduler adjusts the tag to permit delivery. | Admission rejects the observation. |
| Values for the same action and time | The scheduler assigns separate microsteps. | A conflicting action/tag reservation returns `Overflow`. |
| Logical progress | The input does not limit execution. | A required source's frontier limits execution. |
| Submission | One event. | A batch of observations with optional progress. |

Both contracts work with the host monotonic clock or a manual clock. A host clock cannot tell the
scheduler whether an input worker still holds unpublished samples.
A required source uses progress reports to supply this information.

An optional source retains the timestamp checks but does not hold back execution.
It can therefore reject late observations. Use ordinary event delivery when you
want the scheduler to adjust late input instead.

`SourceAction<T>` binds a compiled physical action to its input source.
The handle retains the action's minimum delay and accepts values of type `T`.
You cannot pass it to `SendContext::prepare_action` or convert it to an ordinary
send handle. Use separate actions for the two contracts.

## Send keyboard events and callback results

Declare the required physical actions with
`init_context(name = KeyboardInit, actions = [key_press])`.
The [initialization example](./reactor-initialization.md#declare-an-initializer-context)
generates `send: SendContext` and `key_press: AsyncActionRef<u32>`.
Move these handles into an application worker.

Prepare each acquired value once. If the mailbox is full, retry with the
returned pending value:

```rust,no_run
use boomerang_runtime::{context::TrySendError, AsyncActionRef, SendContext};
use std::sync::atomic::{AtomicBool, Ordering};

fn send_key(
    send: &SendContext,
    action: &AsyncActionRef<u32>,
    key: u32,
    cancelled: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut pending = send.prepare_action(action, key, None)?;
    loop {
        if cancelled.load(Ordering::Acquire) { break; }
        match pending.try_send() {
            Ok(()) => break,
            Err(TrySendError::Closed(_)) => break,
            Err(TrySendError::Full(retry)) => pending = retry,
        }
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    Ok(())
}
```

This function runs in an application worker. `key` is the acquired value, and
`cancelled` is the worker's cancellation flag. The function stops retrying when
the worker is cancelled or the mailbox closes.

`prepare_action` reads the selected clock and adds the action's minimum delay
and the requested delay once. A retry retains this timestamp and the payload.
It does not read the clock again or allocate another payload.
Drop the pending value to cancel it.

Clock errors and arithmetic errors occur before publication. `Closed` means
that the scheduler no longer accepts input.

Use `PreparedAction::send` when the worker can wait for mailbox capacity:

```rust,no_run
# use boomerang_runtime::{AsyncActionRef, SendContext};
# fn send_key(send: &SendContext, action: &AsyncActionRef<u32>, key: u32)
#     -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
send.prepare_action(action, key, None)?.send()?;
# Ok(())
# }
```

`send` uses the prepared timestamp and payload. It consumes the payload even if
the mailbox closes. Use `try_send` to retain an unsuccessful send for a retry.
Do not block in an initializer or in a worker that the scheduler must join.

Use `SendContext::try_shutdown` to request shutdown without blocking.
Retry on `TryShutdownError::Full`. Stop on `TryShutdownError::Closed`.

For a callback service, keep the service in component state. A timer reaction
can request work from the service. The service uses the initialized send
context and action handles when a callback arrives.

Ordinary sending requires neither an input policy nor either hosted feature.
A logical startup reaction can still announce keyboard readiness.

## Declare a timestamped input source

Use source admission when you need to preserve acquisition times and report
which input times are complete. Acquisition time is the selected clock time at
which the source obtains an observation.

Declare a source and its physical actions in the component:

```rust,no_run
use boomerang::prelude::*;
# #[derive(Debug)]
# pub struct DriverState;
# impl DriverState {
#     fn new(_init: DriverInit) -> Result<Self, std::io::Error> { Ok(Self) }
# }

#[reactor(
    state = DriverState,
    state_init = DriverState::new,
    init_context(name = DriverInit, source = "plant", actions = [sample]),
    contract = "example.sensor", contract_version = 1,
    bounds(queue_capacity = 8, payload_bytes = 64, state_bytes = 256, scratch_bytes = 64)
)]
pub fn Sensor(#[physical_action(min_delay = 1 msec)] sample: u32) -> impl Reactor {
    // Reactions use sample as an ordinary action reference.
}
```

This declaration requests source admission without clock control. The
configuration below uses the default host monotonic clock. Start the application worker in `DriverState::new`, as described in
[Reactor initialization](./reactor-initialization.md).

Define `DriverState::new(init: DriverInit)` to return `Result<DriverState, E>`.
The error type must convert to `Box<dyn std::error::Error + Send + Sync>`.
The generated argument has these fields:

| Field | Type and use |
| --- | --- |
| `source` | `input::InputSourceHandle` for observations and progress in this execution. |
| `sample` | `input::SourceAction<u32>` for the declared action endpoint. |
| `send` | `SendContext` for ordinary asynchronous sending. |

Use separate actions for source admission and ordinary asynchronous sending.
Without a `source` declaration, generated action fields use `AsyncActionRef<T>`.
For direct runtime initialization, `StateInitContext::source_action()` returns a
`SourceAction<T>`. `StateInitContext::async_action()` returns an `AsyncActionRef<T>`
only for initialization without a source.

### Configure the source

Configure source policies in application runtime code. The
[physical-input example](https://github.com/jondo2010/boomerang/blob/main/examples/physical-input/src/main.rs) installs the
runtime physical-input configuration and manual clock. Its
[build script](https://github.com/jondo2010/boomerang/blob/main/examples/physical-input/build.rs) selects the matching
component-to-source mapping through `BindingConfiguration`.

The compiler obtains action endpoints from the component declaration. Each
source policy identifies its component instance. Reused components require
separate policies for their separate instances.

Set `required = true` when source progress must limit logical execution.
Set `required = false` for an optional source that does not limit execution.

For host monotonic time, keep the default runtime clock. Read acquisition time through `init.source.clock()`.
Enable `hosted-source-input` for this selection. Manual clock support is not needed.

`max_batch_values` limits the number of values in one submission.
`max_staged_batches` limits retained batches until their destination tags
complete. A submission that exceeds a configured capacity returns `Overflow`.

### Submit observations

Read acquisition time from the source's selected clock. Include the clock domain
and execution epoch with each observation:

```rust,no_run
use boomerang_runtime::input::{InputBatch, InputObservation, InputSourceHandle, InputValue, SourceAction};
use core::time::Duration;

fn submit_sample(
    source: &InputSourceHandle,
    sample: &SourceAction<u32>,
    sequence: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let selected = source.clock();
    let acquired = selected.now()?;
    let batch = InputBatch::new(vec![InputObservation {
        source: source.clone(), sequence,
        domain: selected.domain(), epoch: selected.epoch(), acquired,
        values: vec![InputValue::new(sample, 42)],
    }]);
    source.submit(batch.with_progress(vec![(source.clone(), acquired.checked_add(Duration::from_nanos(1))?)]))?;
    Ok(())
}
```

Use `core::time::Duration` or `std::time::Duration` with
`PhysicalInstant::checked_add`. The runtime's `Duration` type is a signed
logical interval and is a different type.

`InputBatch::new` allocates the delivery and receipt buffers before admission.
It does not validate or publish the observations. `submit` consumes the batch,
moves each delivery buffer to its scheduler, and returns the mapped tags in
`InputReceipt.tags`. Payloads are not cloned.

`submit` waits for admission before it returns a receipt. It can run during
initialization. Reactions start only after all initializers succeed. Shutdown
rejects new submissions and wakes waiting callers. An admitted receipt remains
available if the execution ends before its caller receives it.

Admission rejects an observation at or before a tag that has final permission
to execute. An earlier scheduling grant alone does not make input late.

Admission reuses validation and retention storage allocated at initialization.
Batch preparation still allocates. Hosted coordinator notifications and error
diagnostics can also allocate. Consumed observation storage is released during
submission. This API does not provide an allocation-free runtime.

This submission publishes one value before it reports the associated progress.
The frontier is one nanosecond after the acquisition time. Publish this frontier
only after all observations at that acquisition time are complete.

Increase the sequence number for each observation from a source. Acquisition
times must not decrease or exceed current selected physical time.
All source and action handles in a submission must belong to the same admission
service and execution.

A shared driver can collect handles from several component initializers.
It can submit one batch for several sources, including actions in different
Enclaves. This operation does not require manual clock control.
You can clone action handles without cloning payloads. Admission moves the
submitted payloads without cloning them.

The acquisition time plus the action's minimum delay determines the logical tag.
For example, acquisition at 10 ms with a 1 ms action delay maps to 11 ms.
Host arrival time does not affect this mapping.

A destination rejects a tag that it has completed or reserved for execution.
Source admission does not move a late observation to a later tag.
Ordinary asynchronous sending retains its timestamp and microstep adjustment
behavior.

### Report source progress

Call `source.advance(F)` to declare that future acquisitions have times at or
after `F`. This value is the source's exclusive frontier.
An observation at `F` remains admissible if it passes the other admission checks.
Do not reduce the frontier.

You can also include progress in `submit(batch.with_progress(progress))`, as shown above.
The runtime publishes the observations before the associated progress.
Progress can exceed current physical time. Acquisition time cannot.

A required source holds back logical execution until its frontier permits
progress. It also keeps an idle Federate alive. Continue to report frontiers
when the source is idle, even if it has no new observations.
An optional source does not hold back logical execution.

Clock advancement does not report source progress. If execution waits for a
required source, advancing the manual clock alone does not release that wait.

## Handle input errors

Source admission reports errors through `InputErrorKind`:

| Error kind | Meaning |
| --- | --- |
| `Malformed` | A declaration, identity, payload, or time mapping is invalid. |
| `Future` | Acquisition time exceeds selected physical time. |
| `Late` | The destination has completed or reserved the mapped tag for execution. |
| `Duplicate` | A sequence number repeats the last accepted number or a number checked earlier in the batch. |
| `OutOfOrder` | A sequence number or acquisition time is less than the preceding accepted value. |
| `Protocol` | An observation contradicts a frontier, or a frontier decreases. |
| `Overflow` | A batch, staging, mailbox, or action capacity limit has been reached. |
| `Disconnected` | The source, destination, clock, or execution is disconnected. |

A required source's protocol error, overflow, or disconnection aborts execution.

## Supported deployments

Ordinary sending works with the live builder and runner, and with generated
local Cargo-native applications. It needs neither hosted runtime feature.

Source admission requires compiled execution with one local hosted `std`
Federate. Enable `hosted-source-input` on the runtime dependency of each payload
package that uses source APIs. Application manifests select these features explicitly.
Source admission does not require `hosted-manual-clock`.
This hosted source configuration supports local execution. It is not supported on Pico.

## Run the complete example

The [physical-input example](https://github.com/jondo2010/boomerang/tree/main/examples/physical-input)
contains a component, deployment, and worker guard.
Run this command from the repository root:

```sh
cargo run -p physical-input
```

Expected output:

```text
received 42 at logical 11 ms (physical 11 ms)
```

This example combines source admission with manual time to produce repeatable
output. See the [input API](https://docs.rs/boomerang_runtime/latest/boomerang_runtime/input/index.html)
and [context API](https://docs.rs/boomerang_runtime/latest/boomerang_runtime/context/index.html)
for method details.
