# Terminal Snake

Run the Cargo-native application from the repository root:

```sh
cargo run -p snake-app
```

Use the arrow keys to turn. Use **Ctrl-C** to quit.
The game uses a 16×16 board with edge wrapping.
Food increases the movement speed. A body collision ends the game.
Run the application in an interactive terminal. Normal shutdown restores terminal mode.

The standalone keyboard demo prints arrow keys through the retained legacy CLI:

```sh
cargo run -p cargo-boomerang -- boomerang --workspace examples/snake/keyboard run --deployment keyboard
```

The telemetry demo retains the legacy generated-launcher path. It requires [zellij](https://zellij.dev/):

```sh
./examples/snake/telemetry-demo.sh
```

The demo starts the UDP monitor on `127.0.0.1:9000` before Snake.
Use Up/Down to select a source. Use `1` for throughput or `2` for scheduler time.
Use `q` to exit the monitor.
The Cargo-native application does not enable this telemetry exporter.

Read the `snake-app` crate rustdoc for architecture, feature ownership, dependency roles, and execution limits.

Ported from Clément Fournier's [original Snake example](https://github.com/lf-lang/reactor-rust/blob/master/examples/src/Snake.lf).
