# Reactors

A reactor is a component that owns state, actions, and reactions. In Boomerang, a reactor is declared as a Rust function annotated with `#[reactor]` and returning `impl Reactor`.

```rust
use boomerang::prelude::*;

#[reactor]
fn Greeter() -> impl Reactor {
    reaction! {
        (startup) {
            println!("hello");
            ctx.schedule_shutdown(None);
        }
    }
}
```

What to remember: a reactor is the unit of composition and encapsulation. It is the place where reactions live and where state is owned.
