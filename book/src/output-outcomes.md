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
The runtime rejects fast-forward before it calls an external-effect reaction.
Logical execution permission alone cannot release a physical operation early.

For a Cargo-native application, pass `Config::default()` or explicitly set `with_fast_forward(false)`.
See [Physical clocks](./hosted-physical-clocks.md) for clock selection. Clock
control does not establish output completion or authorize simulator advancement.

## Handle failure and current limits

The runtime validates each returned outcome in live and compiled execution.
A failed operation, a missing report, or an undeclared report fails execution.
A failed generated run does not write a success summary. A failure does not
undo an operation. Do not replay a possibly applied command automatically.
Other reactions in the same parallel level can already have performed effects.

This implementation provides direct outcomes and declaration checks. It does
not yet provide reactor completion observers, tag completion, or time closure.
A successful reaction does not prove that all output work for a reactor is done.
Completion proofs must also account for empty intervals, later microsteps,
modes, lifecycle work, and dependencies across Enclaves.

Deferred operations that commit later need a future bounded pending-operation
API. Return `Committed` for queue acceptance only when the binding defines that
acceptance as commitment. Later device failure then needs a separate reporting
path. Controlled simulation advancement requires a separate design and further
implementation.
