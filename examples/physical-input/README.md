# Timestamped physical input

One sensor supplies a sample acquired at 10 ms.
Its physical action adds a 1 ms delay.

Run from the repository root:

```sh
cargo run -p physical-input
```

The application prints this line and exits:

```text
received 42 at logical 11 ms (physical 11 ms)
```

Package rustdoc describes the build script, source configuration, and worker lifecycle.
