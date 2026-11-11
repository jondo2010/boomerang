# Reactor initialization

Use a state initializer to construct reactor state before logical execution.
An initializer can receive action handles, a source handle, manual clock
control, or an output observer. These resources are called capabilities.
Initialization does not require an input source or a manual clock.

## Declare an initializer context

Use `state` to select the state type and `state_init` to select its constructor.
Add `init_context` to declare the argument passed to that constructor:

```rust,no_run
use boomerang::prelude::*;

#[derive(Debug)]
pub struct KeyboardState;

impl KeyboardState {
    fn new(init: KeyboardInit) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Give init.send and init.key_press to your input worker.
        let _ = init;
        Ok(Self)
    }
}

#[reactor(
    state = KeyboardState,
    state_init = KeyboardState::new,
    init_context(name = KeyboardInit, actions = [key_press])
)]
pub fn Keyboard(#[physical_action] key_press: u32) -> impl Reactor {
    reaction! { receive(key_press) {
        let _ = ctx.get_action_value(&mut key_press);
    } }
}
```

The macro generates the `KeyboardInit` structure. Its `send` field has type
`SendContext`. Its `key_press` field has type `AsyncActionRef<u32>`.
The example declares the context; it does not start a worker.
See [External inputs](./external-inputs.md) for sending and retry instructions.

`init_context` requires both `state` and `state_init`.
Use these constructor signatures:

| Declaration | Constructor signature |
| --- | --- |
| `state_init = path` without `init_context` | `fn() -> State` |
| `state_init = path` with `init_context` | `fn(GeneratedContext) -> Result<State, E>` |

`E` must convert to `Box<dyn std::error::Error + Send + Sync>`.
The context is passed by value. Its handles can be moved into application workers.

For a generic reactor, the generated context has the same generic parameters
and bounds. For example, a reactor `Sensor<T, const N: usize>` with a context
named `SensorInit` passes `SensorInit<T, N>` to its initializer.

## Select context fields

`name` sets the Rust name of the generated structure. The other options select
its fields:

| Option | Generated fields |
| --- | --- |
| Always present | `send: SendContext` |
| `actions = [sample]`, without `source` | `sample: AsyncActionRef<T>`, where `T` is the action payload type |
| `source = "plant"` | `source: InputSourceHandle` |
| `actions = [sample]`, with `source` | `sample: SourceAction<T>` |
| `clock_control = true` | `clock: ManualClock` |
| `output_observer = true` | `output: OutputObserver` |

Each action name must identify a physical action in this reactor declaration.
Do not repeat action names. The names `source`, `clock`, `output`, `send`, and
`__boomerang_marker` are reserved for generated fields, even when the
corresponding option is absent.

A source declaration applies to all listed actions. One context cannot mix
ordinary action handles and source action handles. Use separate components
when their initializers need different input contracts.
A source requires at least one declared action in compiled deployment metadata.

Omit `source` for ordinary sending. Omit `clock_control` when the component
does not advance physical time. A component can control a manual clock without
a source, or use source admission with the host monotonic clock.

## Select an execution path

Ordinary action initialization works with the live builder, the live runner,
and local Cargo-native applications. It requires no hosted runtime feature.
Source admission and manual clock control require compiled local hosted
Federate execution. Enable `hosted-source-input` or `hosted-manual-clock` on
the runtime dependency of each payload package that uses the corresponding API.
These hosted APIs are unsupported on constrained targets such as Pico.

Generated payload bindings call declared context initializers during runtime initialization. With the live
builder, use `Reactor::build_initialized`. With the live runner, use
`boomerang_util::runner::build_and_test_reactor_initialized`.
Methods that accept supplied state do not call the context initializer.

For direct compiled-runtime bindings, use the `boomerang_runtime::initialization`
module. Use `initialization::StateCapabilities`
to declare resources and `initialization::StateInitContext` to obtain their
handles. Capability lookup returns `initialization::InitializationError` when a
requested resource is unavailable or does not match its declaration.
Input submission errors remain separate and use `input::InputError`.
Output configuration errors use `InitializationError::OutputUnavailable` when
`hosted-output` is enabled. Output requests use `OutputWaitError`.

## Export the owning output boundary

This initializer declares an observer without a clock or source. It queues a
startup request before execution and starts a worker. The worker's wait has a
host-time limit:

```rust,no_run
use boomerang::prelude::*;
use boomerang::runtime::{OutputOutcome, OutputWaitError, Tag};
use std::io::Write;

// Application-owned reporting. A failed log write must not panic during teardown.
fn report_output_failure(message: &str) {
    let _ = writeln!(std::io::stderr().lock(), "output worker: {message}");
}

#[derive(Debug)]
pub struct OutputState {
    worker: Option<std::thread::JoinHandle<()>>,
}

impl OutputState {
    fn new(init: OutputInit) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let wait = init.output.request_tag(Tag::ZERO)?;
        let worker = std::thread::spawn(move || {
            match wait.wait_timeout(std::time::Duration::from_secs(1)) {
                Ok(_completion) => {}
                Err(OutputWaitError::Timeout) => report_output_failure("completion timed out"),
                Err(error) => report_output_failure(&format!("completion failed: {error:?}")),
            }
        });
        Ok(Self { worker: Some(worker) })
    }
}

impl Drop for OutputState {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                report_output_failure("worker panicked");
            }
        }
    }
}

#[reactor(
    state = OutputState, state_init = OutputState::new,
    init_context(name = OutputInit, output_observer = true),
    external_effects = [Apply]
)]
pub fn OutputDevice() -> impl Reactor {
    reaction! { Apply(startup) { OutputOutcome::Unchanged } }
}
```

The handle selects only this instance's directly declared effects. It excludes
child effects. Runtime cleanup releases output waiters before it drops state
that can join workers. Use a finite host-time bound for each worker operation.
An observer does not keep execution alive. Use `Config::with_keep_alive(true)`
when a driver must submit requests after startup.
Provide a stop or failure path for that execution. See
[External output outcomes](./output-outcomes.md) for bounds and durable cutoffs.

## Initialize before logical startup

The runtime validates all selected capability declarations and payload types
before it calls any context initializer. This check includes components without
reactions. The clock origin and action endpoints are ready when the initializer
runs. For source input, the source state and coordinator are also ready.

Initialization does not consume a logical tag. You can therefore submit an
observation acquired at epoch coordinate zero. Required source progress still
limits when startup reactions can execute.

Return from the initializer without waiting for logical execution. You can
submit a bounded source batch or start a worker. For ordinary sends during
initialization, use `prepare_action` and nonblocking `try_send`.
Alternatively, send from a worker without waiting for that worker to finish.
A blocking send can prevent scheduler startup if the mailbox becomes full.

## Manage application workers

Keep a guard for each worker in component state. Make the guard cancel and join
its worker when the guard is dropped. If construction fails or panics, cancel any
workers whose guards have not transferred to the returned state.

After the runtime takes ownership of state, runtime failure closes the input
and clock capabilities and scheduler channels before it drops worker guards.
Run completion also closes retained capabilities.

See [External inputs](./external-inputs.md) for source declarations and
[Physical clocks](./hosted-physical-clocks.md) for manual clock selection.
For direct runtime bindings, see the
[initialization API](https://docs.rs/boomerang_runtime/latest/boomerang_runtime/initialization/index.html).
