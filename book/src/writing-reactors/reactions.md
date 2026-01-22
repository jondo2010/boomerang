# Reactions

Reactions are the executable units inside a reactor. A reaction is triggered when one of its triggers is present at a given tag. It can read trigger values, mutate state, and write to effects.

```rust
use boomerang::prelude::*;

#[reactor]
fn HelloWorld() -> impl Reactor {
    reaction! {
        (startup) {
            println!("Hello World.");
            ctx.schedule_shutdown(None);
        }
    }
}
```

Reactions can also write to outputs:

```rust
#[reactor]
fn Double(#[input] x: u32, #[output] y: u32) -> impl Reactor {
    reaction! {
        (x) -> y {
            *y = Some(2 * x.unwrap());
        }
    }
}
```

If you need more control, the builder API lets you attach triggers, effects, and a reaction function explicitly. This is useful for advanced cases or for building reactions dynamically.
