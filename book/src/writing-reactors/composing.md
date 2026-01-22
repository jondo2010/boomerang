# Composing Reactors

Reactor composition is done by creating child reactors inside another reactor and wiring their ports together.

```rust
use boomerang::prelude::*;

#[reactor]
fn Gain(#[input] inp: u32, #[output] out: u32, gain: u32) -> impl Reactor {
    reaction! {
        (inp) -> out {
            *out = Some(inp.unwrap() * gain);
        }
    }
}

#[reactor]
fn Top(#[input] inp: u32, #[output] out: u32) -> impl Reactor {
    let gain = builder.add_child_reactor(Gain(2), "gain", (), false)?;
    builder.connect_port(inp, gain.inp, None, false)?;
    builder.connect_port(gain.out, out, None, false)?;
}
```

Connections can include an optional delay. See "Time and Timers" and the "Timers and Delays" guide for examples.
