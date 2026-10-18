# Timestamped physical input

One source, one physical action, and one reaction, using a generated hosted
launcher. Run from the repository root:

```sh
cargo run -p cargo-boomerang -- boomerang --workspace examples/physical-input run --deployment physical-input
```

The application prints this line and exits:

```text
received 42 at logical 11 ms (physical 11 ms)
```

[`Boomerang.toml`](Boomerang.toml) binds the required `plant` source to the
sensor component. Its `init_context(name = DriverInit, source = "plant",
actions = [sample], clock_control = true)` declaration selects the existing
physical action. The compiler injects named typed handles into
[`DriverState::new`](src/lib.rs); the initializer performs no name lookup.

`init.sample` is a `SourceAction<u32>`. It belongs to `init.source` and uses
timestamp-preserving admission. Ordinary keyboard or callback delivery uses
`AsyncActionRef<T>` with `SendContext` on a separate action. That path can adjust
late timestamps. Source admission rejects late observations.

Source admission also works with host monotonic time. Such a deployment omits
`clock_control` and `physical-clock`, reads time through `source.clock()`, and
retains the source policy and progress reports. This example selects manual time
to make its acquisition and output times repeatable.

The driver advances its manual clock to 10 ms and submits value `42`, acquired
at 10 ms. It publishes an exclusive acquisition frontier of 10 ms + 1 ns with
the sample: no future sample can have an earlier acquisition time. The action's
1 ms minimum delay maps the observation to logical time 11 ms. Advancing the
clock to 11 ms then permits physical pacing to reach that tag. Both clock time
and source progress are needed; advancing the clock alone does not release a
required source's progress constraint.

The initializer starts a finite one-shot worker and returns its join guard before
the scheduler starts. State owns the worker and joins it during teardown.
Closed capabilities are normal cancellation; other worker errors are reported. The reaction prints
the value and times, then requests shutdown so the required source does not keep
the example waiting for further progress. A continuous driver also needs a cancellation-aware guard. Initializer-local
workers must be cancelled and joined if initialization fails before returning
state; the runtime closes capabilities before dropping successfully returned
states.

See [External inputs](../../book/src/external-inputs.md) for source configuration
and input errors. See [Physical clocks](../../book/src/hosted-physical-clocks.md)
for manual clock selection.
