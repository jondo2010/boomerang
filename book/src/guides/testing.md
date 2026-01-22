# Testing Reactors

Tests are the easiest way to run a reactor. Boomerang provides helpers in `boomerang_util::runner` to build and execute a reactor in a test environment.

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

Use `with_fast_forward(true)` to advance logical time without waiting for wall clock time during tests.
