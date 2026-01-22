# A First Reactor

This page introduces a minimal, runnable example that prints "Hello World." on startup and then shuts down.

## Minimal Example

Create a test that builds and runs a reactor with a startup reaction:

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
    boomerang_util::runner::build_and_test_reactor(
        HelloWorld(),
        "hello_world",
        (),
        config,
    )
    .unwrap();
}
```

## Running It

Place the test in `tests/hello_world.rs` in a new crate that depends on Boomerang. Then run:

```
cargo test -p your_crate_name hello_world
```

You should see `Hello World.` in the test output.

## Structure of a Boomerang Project

Boomerang is a Rust library. The typical structure is:

- `Cargo.toml` lists dependencies like `boomerang` and `boomerang_util`.
- `src/lib.rs` or `src/main.rs` can define reactors.
- `tests/` can contain runnable examples using `boomerang_util::runner`.

In this repository, tests like `boomerang/tests/hello_world.rs` show minimal runnable reactors.

## Reactor Anatomy

A reactor is a Rust function annotated with `#[reactor]` and returning `impl Reactor`. The `reaction!` macro defines reactions that run when triggers are present. In the example above, `(startup)` is the startup action automatically scheduled by the runtime.

## Comments and Style

Use standard Rust comments:

```rust
// Single-line comment.
/* Multi-line
   comment. */
```

Avoid pseudo-language syntax; in Boomerang, the Rust code is the source of truth.
