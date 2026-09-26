# Compiled Snake example

A terminal Snake game for Unix-like systems. Run these commands from the repository root:

```sh
cargo run -p cargo-boomerang -- boomerang --workspace examples/snake run --deployment snake
```

Use the arrow keys to turn and **Ctrl-C** to quit. The game starts on a 16×16 board,
wraps at the edges, and speeds up as the snake eats food. Reversing directly into
the snake is ignored; colliding with its body ends the game and prints the score.
Run it in an interactive terminal. Raw mode is restored on normal shutdown.

## Telemetry snapshot demo

Snake's checked-in deployment enables the hosted telemetry exporter. With
[`zellij`](https://zellij.dev/) installed, launch the two-pane demo from the
repository root:

```sh
./examples/snake/telemetry-demo.sh
```

The left pane runs Snake. The right pane runs the optional monitor client,
collects two startup telemetry records over UDP on `127.0.0.1:9000`, and prints
one JSON snapshot before waiting for Enter. This is intentionally a completed
snapshot demonstration: the monitor is not a live dashboard yet. The later
dashboard slice will replace this finite output with live presentation.

The keyboard-only demo prints each arrow key:

```sh
cargo run -p cargo-boomerang -- boomerang --workspace examples/snake/keyboard run --deployment keyboard
```

Replace `run` with `check` to validate a deployment, or `build` to publish its
generated executable without starting the terminal session. Both deployments use
wall-clock pacing and remain alive while waiting for physical keyboard input.

The examples compose the same keyboard component with different consumers:

```text
Snake:         Keyboard.key   ──► Snake.key
               Keyboard.ready ──► Snake.ready

Keyboard demo: Keyboard.key   ──► ArrowDisplay.key
```

The `snake-keyboard` crate owns raw terminal mode, the polling thread, the
physical action, and Ctrl-C handling. Its implementation lives under
[`keyboard/src/keyboard`](keyboard/src/keyboard), alongside the named component
declaration in [`keyboard/src/lib.rs`](keyboard/src/lib.rs). It exposes
`key: crossterm::event::KeyEvent` for
arrow presses and emits `ready: ()` after enabling raw mode. Snake starts rendering
and its game clock when `ready` arrives. `ArrowDisplay` only prints received keys.

The [`snake` topology library](src/lib.rs) wires these typed ports through the
public `snake-keyboard` and `snake-game` Cargo dependencies. Each component crate
owns all of its source and exposes named modules: `snake_keyboard::keyboard`,
`snake_keyboard::display`, and `snake_game::game`.

The deployment manifests select those modules with each binding's `component`
path. The Snake manifest binds `snake-game::game` and
`snake-keyboard::keyboard`; the keyboard demo keeps its own manifest because the
current schema has one topology entry per manifest, and binds
`snake-keyboard::{keyboard, display}`. Each composition shares one enclave, so
Ctrl-C and game-over shut down both components and restore terminal settings.

Each `component!` declaration generates a host-side `definition()` constructor.
The topology library composes those definitions with typed port handles:

```rust
let mut app = TopologyBuilder::new("application/keyboard/snake")?;
let enclave = app.enclave("keyboard")?;
let keyboard = app.component("keyboard", keyboard::definition(), &enclave)?;
let snake = app.component("snake", game::definition(), &enclave)?;
app.connect(&keyboard.key, &snake.key)?;
app.connect(&keyboard.ready, &snake.ready)?;
app.finish()
```

The constructors declare actions, ports, reactions, and timing in the compiler
topology. Runtime initialization occurs when the generated executable starts.
The shared enclave is explicit, and connections require matching payload types
and output-to-input direction.

Compile the component crates and both deployments without starting the game:

```sh
cargo check -p snake -p snake-game -p snake-keyboard
cargo run -p cargo-boomerang -- boomerang --workspace examples/snake build --deployment snake
cargo run -p cargo-boomerang -- boomerang --workspace examples/snake/keyboard build --deployment keyboard
```

The keyboard connection provides the boundary for future recording and replay.
A replay harness could feed recorded inputs to the game and verify its final
state. Reproducible execution requires a serializable keyboard event contract,
terminal readiness independent of physical input, and a recorded food RNG seed.

Ported from Clément Fournier's [original Snake example](https://github.com/lf-lang/reactor-rust/blob/master/examples/src/Snake.lf).
