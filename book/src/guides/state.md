# State and Lifecycle

State is private to a reactor and is only mutated inside reactions. Lifecycle triggers like `startup` and `shutdown` let you initialize and clean up.

```rust
use boomerang::prelude::*;

struct State {
    success: bool,
}

#[reactor(state = State)]
fn HelloWorld() -> impl Reactor {
    reaction! {
        (startup) {
            println!("startup");
            state.success = true;
        }
    }

    reaction! {
        (shutdown) {
            assert!(state.success);
            println!("shutdown");
        }
    }
}
```

Use `ctx.schedule_shutdown(None)` to trigger shutdown after completing a scenario.
