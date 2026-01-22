# Quickstart

This is a minimal, runnable Hello World example using Boomerang's test runner. It mirrors the "A First Reactor" example and uses `cargo test` to execute the reactor.

## 1. Create a crate

```
cargo new hello-boomerang --lib
cd hello-boomerang
```

## 2. Add dependencies

```
cargo add boomerang
cargo add --dev boomerang_util tracing-subscriber
```

## 3. Add the test

Create `tests/hello_world.rs`:

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

#[test]
fn hello_world() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    boomerang_util::runner::build_and_test_reactor(HelloWorld(), "hello_world", (), config)
        .unwrap();
}
```

## 4. Run it

```
cargo test hello_world
```

You should see `Hello World.` in the output.

If you prefer to run reactors in a standalone binary, see the guides for composition, timing, and testing patterns used by the runtime.
