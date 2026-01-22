# Composition and Hierarchy

Composition is how you build larger systems from smaller reactors. Use `add_child_reactor` and connect their ports.

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

In tests like `boomerang/tests/hierarchy.rs`, this pattern is used to build multi-level hierarchies.
