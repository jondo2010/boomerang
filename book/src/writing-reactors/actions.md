# Actions

Actions are events that can carry data. Boomerang distinguishes between logical actions (internal, deterministic) and physical actions (boundary with the outside world).

Logical actions are scheduled by the runtime or reactors. Physical actions are scheduled by external inputs and are the entry point for nondeterministic data.

A physical action can be created with the builder API:

```rust
use boomerang::prelude::*;

#[reactor]
fn WithPhysical() -> impl Reactor {
    let act = builder.add_physical_action::<u32>("act", None)?;

    builder
        .add_reaction(Some("OnAct"))
        .with_trigger(act)
        .with_reaction_fn(|ctx, _state, (mut act,)| {
            let value = ctx.get_action_value(&mut act).unwrap();
            println!("Received {}", value);
        })
        .finish()?;
}
```

Physical actions are also the boundary for recording and replay. See the "Recording and Replay" page for details.
