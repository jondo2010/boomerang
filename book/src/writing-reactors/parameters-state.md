# Parameters and State

Reactor parameters are regular function arguments. State is declared with a struct and attached using `#[reactor(state = State)]`.

```rust
use boomerang::prelude::*;

struct State {
    count: u32,
}

#[reactor(state = State)]
fn Counter(limit: u32) -> impl Reactor {
    timer! { tick(0 s, 1 s) };

    reaction! {
        (tick) {
            state.count += 1;
            if state.count >= limit {
                ctx.schedule_shutdown(None);
            }
        }
    }
}
```

Parameters are copied into the reactor when it is created. State is mutable only inside reactions, and it is private to the reactor.
