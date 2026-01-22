# Time and Timers

Timers are actions that the runtime schedules at fixed offsets and periods. Use the `timer!` macro inside a reactor to define them.

```rust
use boomerang::prelude::*;

#[reactor]
fn Ticker() -> impl Reactor {
    timer! { t(0 s, 100 msec) };

    reaction! {
        (t) {
            println!("tick");
        }
    }
}
```

The first argument is the offset (when the timer first fires) and the second is the period. If you need a one-shot timer, omit the period.

Timers are deterministic: given the same initial time and inputs, the same timer tags will be produced.
