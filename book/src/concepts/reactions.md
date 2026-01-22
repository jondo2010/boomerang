# Reactions

A reaction is code that runs when its triggers are present at a given tag. Reactions read trigger values, update state, and produce outputs or schedule actions.

```rust
use boomerang::prelude::*;

#[reactor]
fn Echo(#[input] x: u32, #[output] y: u32) -> impl Reactor {
    reaction! {
        (x) -> y {
            *y = *x;
        }
    }
}
```

What to remember: reactions are the scheduling unit in the runtime, and they run deterministically based on triggers and tags.
