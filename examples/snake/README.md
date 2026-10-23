# Terminal Snake

The example root is the Snake application. Its sibling crates are:

- `game/`: game component.
- `keyboard/`: keyboard and arrow display components.
- `keyboard-app/`: standalone keyboard application.

Run the Cargo-native application from the repository root:

```sh
cargo run -p snake-app
```

Use the arrow keys to turn. Use **Ctrl-C** to quit.
The game uses a 16×16 board with edge wrapping.
Food increases the movement speed. A body collision ends the game.
Run the application in an interactive terminal. Normal shutdown restores terminal mode.

The Cargo-native standalone keyboard demo displays arrow keys:

```sh
cargo run -p snake-keyboard-app
```

The native telemetry demo requires [zellij](https://zellij.dev/):

```sh
./examples/snake/telemetry-demo.sh
```

The demo runs Snake with the `telemetry` feature and a UDP monitor on `127.0.0.1:9000`.
Use Up/Down to select a source. Use `1` for throughput or `2` for scheduler time.
Use `q` to exit the monitor.


Read the application rustdoc in `src/main.rs` and `keyboard-app/src/main.rs` for architecture and execution policy.

Ported from Clément Fournier's [original Snake example](https://github.com/lf-lang/reactor-rust/blob/master/examples/src/Snake.lf).
