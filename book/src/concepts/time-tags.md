# Time and Tags

Boomerang uses a logical time model. A tag is a pair of logical time and microstep that totally orders events. When multiple events occur at the same logical time, microsteps provide a deterministic order.

```rust
use boomerang::prelude::*;

#[reactor]
fn LogTag() -> impl Reactor {
    reaction! {
        (startup) {
            println!("tag: {:?}", ctx.get_tag());
            ctx.schedule_shutdown(None);
        }
    }
}
```

What to remember: tags are how the runtime orders events and reactions. The same input tags produce the same execution order.
