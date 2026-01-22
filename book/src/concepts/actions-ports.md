# Actions and Ports

Ports are typed connections between reactors, declared as inputs and outputs. Actions are event sources within a reactor, including timers, logical actions, and physical actions.

```rust
use boomerang::prelude::*;

#[reactor]
fn Scale(#[input] x: u32, #[output] y: u32, factor: u32) -> impl Reactor {
    reaction! {
        (x) -> y {
            *y = Some(factor * x.unwrap());
        }
    }
}
```

Values arrive as `Option<T>`. If a value is present at the current tag, the option is `Some(value)`. If not, it is `None`.

What to remember: ports move data between reactors, actions produce events inside a reactor, and presence is represented with `Option<T>`.
