# Inputs and Outputs

Inputs and outputs are declared on a reactor's function signature using `#[input]` and `#[output]` attributes. They are typed and accessed inside reactions using the generated bindings.

```rust
use boomerang::prelude::*;

#[reactor]
fn Scale(#[input] x: u32, #[output] y: u32, scale: u32) -> impl Reactor {
    reaction! {
        (x) -> y {
            *y = Some(scale * x.unwrap());
        }
    }
}
```

To connect ports between reactors, use the builder:

```rust
#[reactor]
fn Test(#[input] x: u32) -> impl Reactor {
    reaction! {
        (x) {
            println!("Received value: {:?}", *x);
        }
    }
}

#[reactor]
fn Gain() -> impl Reactor {
    let g = builder.add_child_reactor(Scale(2), "g", (), false)?;
    let t = builder.add_child_reactor(Test(), "t", (), false)?;
    builder.connect_port(g.y, t.x, None, false)?;
}
```

Ports carry `Option<T>` values inside reactions. Use `is_present` to check whether a port or action is present before reading values when appropriate.
