# External output outcomes

A reaction can write to a device directly. It does not need an output port or a
runtime command queue. Put the device interface in reactor state. Supply it
before execution. See [Reactor initialization](./reactor-initialization.md).

Define the commitment point for each driver binding. For example, it can be a
register write, DMA submission, or acceptance by a device queue. Return
`Committed` only after the operation crosses that point. Commitment does not
prove physical application unless the binding defines it that way.

## Declare external effects

List the named reactions that can perform external operations:

```rust,no_run
use boomerang::prelude::*;
use boomerang::runtime::{OutputFault, OutputOutcome};

#[derive(Debug)]
pub struct MotorState {
    write: fn(u32) -> Result<(), ()>,
}

#[reactor(state = MotorState, external_effects = [Apply, Stop])]
pub fn Motor(#[input] command: u32) -> impl Reactor {
    reaction! { Apply(command) {
        match command.as_ref() {
            None => OutputOutcome::Unchanged,
            Some(value) => match (state.write)(*value) {
                Ok(()) => OutputOutcome::Committed,
                Err(()) => OutputOutcome::Failed(OutputFault::Device),
            },
        }
    } }
    reaction! { Stop(shutdown) {
        OutputOutcome::Unchanged
    } }
}
```

The application supplies `MotorState::write` with the driver's operation and
passes the state to the live builder. For generated execution, add a state
initializer. See [Reactor initialization](./reactor-initialization.md).
In this example, a successful call must mean that the operation crossed the
binding's commitment point. Keep its time and resource use within the execution
profile's bounds. Do not wait in a reaction for simulator advancement or an
unbounded device acknowledgment.

Each listed name must identify a reaction in this reactor. Do not repeat names.
A declared reaction must return `OutputOutcome` on every normal return path.
The live builder and generated payload code enforce this return type.

| Outcome | Meaning |
| --- | --- |
| `Unchanged` | This invocation required no external operation. |
| `Committed` | The operation crossed the binding's commitment point. |
| `Failed(OutputFault::Device)` | The driver failed. A partial external effect is possible. |
| `NotReported` | No external result was reported. This is invalid for a declared effect. |

`Unchanged` does not request a zero value or a default device state.
Ordinary reaction bodies can continue to return `()`. The adapters convert
`()` to `NotReported`. This conversion reports no commitment. An ordinary
reaction must not return an explicit external result without a declaration.
For manual bindings, set the reaction's external-effect metadata as well.

## Keep each reactor instance distinct

A reactor can declare multiple external-effect reactions. Any number of reactor
instances can declare effects. Each instance has its own output boundary, even
when the instances use the same reactor type. A parent's boundary does not
include its children.

Ports, actions, tags, and reaction dependencies keep their existing meaning.
Use those dependencies to order operations. A reactor boundary does not make
several device operations atomic. Use a device guarantee when operations must
apply at the same physical instant.

## Use paced execution

Declared external effects require paced execution in this implementation.
Use the default runtime configuration, with `fast_forward = false`.
The runtime rejects fast-forward execution before it calls a state initializer.
Logical execution permission alone cannot release a physical operation early.

For a Cargo-native application, pass `Config::default()` or explicitly set `with_fast_forward(false)`.
See [Physical clocks](./hosted-physical-clocks.md) for clock selection. Clock
control does not establish output completion or authorize simulator advancement.

## Export an output observer

Enable `hosted-output` on the `boomerang` dependency of a component that exports
an observer. This feature does not enable manual clocks or source admission.
Declare `init_context(name = MotorInit, output_observer = true)` with `state`
and `state_init`. The generated context then has an `output: OutputObserver`
field. Move this handle into the application's driver or worker. See
[Reactor initialization](./reactor-initialization.md).

Output observers require compiled local hosted execution, using either the host
monotonic clock or a manual clock. The live builder rejects an observer
initializer declaration before constructing user state. Live reactions still
support declared external effects and validate their returned outcomes.
Direct compiled bindings use `StateCapabilities::output_observer` and
`StateInitContext::output_observer()`. Each handle identifies the actual reactor
instance and its immutable execution epoch. Clones select the same instance.
A parent handle excludes its children's effects. A successful completion's
`target()` returns its compiled `(EnclaveIndex, ReactorIndex)` identity.

The selected coordination backend must exclude further logical ingress through
an accepted bound. The local backend supports this proof. The current central
RTI backend does not. Both tag and time requests return `UnsupportedBackend`
there. A declared initializer export on that backend fails before state
initializers run. Grants and idle confirmation alone are insufficient.

## Wait outside reactions

Run the scheduler independently. Never block a reaction or initializer on an
output wait. A wait does not run the scheduler. An observer handle does not
keep execution alive. Use `Config::with_keep_alive(true)` for a long-lived driver. Arrange startup requests before execution can
quiesce, and provide a bounded stop or failure path.

This function runs on a driver thread. The caller supplies an initialized
observer and keeps the execution running:

```rust,no_run
use boomerang::runtime::{
    clock::PhysicalInstant, Duration, OutputBound, OutputObserver, OutputWaitError, Tag,
};
use std::time::Duration as HostDuration;

fn wait_for_outputs(observer: &OutputObserver) -> Result<(), OutputWaitError> {
    let tag = Tag::new(Duration::nanoseconds(10), 0);
    let tag_proof = observer.request_tag(tag)?
        .wait_timeout(HostDuration::from_secs(1))?;
    assert_eq!(tag_proof.bound(), OutputBound::ThroughTag(tag));
    assert_eq!(tag_proof.epoch(), observer.epoch());

    let time = PhysicalInstant(10);
    let time_proof = observer.close_time(time)?
        .wait_timeout(HostDuration::from_secs(1))?;
    assert_eq!(time_proof.bound(), OutputBound::ThroughTime(time));
    Ok(())
}
```

`request_tag(Tag::new(t, m))` proves valid direct outcomes and excludes further effects
through that inclusive tag. Later microsteps at `t` remain possible.
`close_time(t)` covers all microsteps at `t` and all earlier times. It can prove
an empty interval. Neither operation requires a reaction at the requested bound.
`wait_through_tag(tag)` combines a tag request with an unbounded host wait.
Prefer `wait_timeout` when the driver needs a finite host-time limit.

A proof includes `Unchanged` as a valid outcome. It does not assert that a device
operation occurred, that a device has settled, or that several operations were
atomic. The proof is for the selected reactor's direct effects. It is not a
Federate-wide output acknowledgment barrier. Existing execution and causal
barriers can still delay the proof.

## Account for time cutoffs

Time closure actively installs durable admission cutoffs at Enclave
computational boundaries and applicable upstream cuts. Input admission beyond
the selected reactor can therefore change. Fresh strict, source, and replay
input must follow their explicit late-input policy. Unrestricted external
physical input is retagged above the cutoff. Authorized internal traffic and
batches admitted before the cutoff must still drain. Do not treat every logical
message as late.

Required input sources must publish truthful exclusive frontiers beyond the
applicable cut. Publish observations before their progress. An output request
cannot invent missing source progress or release physical work before the
selected clock reaches it.

Dropping a wait cancels its request. A host timeout also consumes and cancels
that wait. Neither path undoes an effect or an installed cutoff. Direct backend
rejection installs no local cutoff. Discovery can fail after local or upstream
cutoffs have been installed; those cutoffs remain. A canceled request cannot
provide a reply to a later request.

Closure alone does not permit a simulator to step over pending timers or
delayed work. It supplies no safe next-time negotiation or controlled advancement
service. Graceful shutdown effects above a sealed time still need physical
release. Immediate host abort remains available.

## Handle failure and current limits

The runtime validates each returned outcome in live and compiled execution.
A failed operation, a missing report, or an undeclared report fails execution.
Execution returns an error after failure. A failure does not undo an operation. Do not replay a possibly applied command automatically.
Other reactions in the same parallel level can already have performed effects.

Output requests have bounded capacity. Handle `Capacity` without an unbounded
retry loop. Invalid bounds or targets and unsupported backends reject requests.
`Shutdown`, `ExecutionFailed`, `Canceled`, `Disconnected`, and `StaleExecution`
terminate requests without a proof. `Timeout` limits only the host wait; it does
not stop execution. A proof already delivered remains readable after a later
execution failure. Use the returned target and epoch to associate it with the
correct instance and run.

The bound, outcome, and completion value contracts are portable. The current
observer adapter uses hosted threads and channels. This implementation makes no
Pico support claim. A future constrained adapter must preserve the same proof
contract within its own resource and platform limits.

Deferred operations that commit later need a future bounded pending-operation
API. Return `Committed` for queue acceptance only when the binding defines that
acceptance as commitment. Later device failure then needs a separate reporting
path. Controlled simulation advancement requires a separate design and further
implementation.
