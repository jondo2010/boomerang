# Trace Visualization with Rerun

Boomerang can record its structured scheduler trace into a finalized
[Rerun](https://rerun.io/) recording. The trace is intended for debugging
information flow, scheduler parallelism, logical-time behavior, ownership, and
propagation across a lowered runtime graph.

The adapter uses Rerun `0.36.1`. It composes as a `tracing_subscriber` layer and
does not install a global subscriber.

## Runner integration

Programs using `boomerang_util::runner` can enable recording with the `rerun`
feature and either the command-line flag or environment variable:

```console
cargo run --features boomerang_util/rerun -- --rerun
BOOM_RERUN=1 cargo test <test-name>
```

The runner writes `target/boomerang/diagrams/<reactor>.rrd`, alongside the
PlantUML diagram output convention.

## Manual integration

Declare Boomerang with the `rerun` feature. Manual subscriber composition also
names `tracing` and `tracing-subscriber` APIs, so they must be direct
dependencies:

```toml
[dependencies]
boomerang = { version = "0.3", features = ["rerun"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

Add `federated` to the Boomerang feature list for a federated model. A direct
Rerun dependency is needed only when application code names SDK types or builds
a custom blueprint:

```toml
rerun = { version = "=0.36.1", default-features = false, features = ["sdk"] }
```

Register the lowered `RuntimeAssembly` before execution consumes it:

```rust
# #[cfg(feature = "rerun")]
# mod example {
use std::{error::Error, path::PathBuf};

use boomerang::builder::RuntimeAssembly;
use boomerang::rerun::{RerunSessionBuilder, SinkConfig};
use boomerang::runtime;
use tracing_subscriber::prelude::*;

fn run_local(
    parts: RuntimeAssembly,
    config: runtime::Config,
) -> Result<(), Box<dyn Error>> {
    let session = RerunSessionBuilder::new("my-boomerang-model")
        .sink(SinkConfig::File(PathBuf::from("recording.rrd")))
        .build()?;
    session.register_runtime(&parts);

    let subscriber = tracing_subscriber::registry().with(session.layer());
    tracing::subscriber::with_default(subscriber, || {
        runtime::execute_enclaves(parts.into_local()?.into_iter(), config)?;
        Ok::<_, Box<dyn Error>>(())
    })?;

    session.finish()?;
    Ok(())
}
# }
```

For a federated runtime, registration is identical; enable both features and
consume the assembly with `execute_federation_in_memory` or
`execute_federation_over_tcp` inside the subscriber scope.

## Sinks and finalization

`SinkConfig::Memory`, `SinkConfig::File(path)`, and nested non-empty
`SinkConfig::Tee` configurations are supported. Lexically duplicate file paths
are rejected. Avoid symlink or hard-link aliases to the same destination,
because validation intentionally does not canonicalize paths.

Treat `session.finish()?` as the success signal for an offline file. It reports
sink, lifecycle, footer-verification, and earlier observational failures.
Dropping a session performs only bounded best-effort cleanup and cannot report
failure.

The gRPC sink is currently rejected as `UnsupportedGrpc`: Rerun 0.36.1 exposes
blocking behavior that cannot safely be isolated from scheduler callbacks.
Memory, file, and tee sinks still use the SDK's bounded batching and can apply
backpressure when saturated.

## Opening and checking recordings

Open a finalized file in the Viewer:

```console
rerun recording.rrd
```

Validate it without launching the Viewer:

```console
rerun rrd verify recording.rrd
```

An optimized copy can improve random access:

```console
rerun rrd optimize recording.rrd -o recording-optimized.rrd
```

## Default views and time axes

The default blueprint provides:

- **Scheduler phase spans (wall clock)** for entered scheduler activity;
- **Logical phases and measures** for discrete logical observations;
- **Event records** for inspecting dense trace archetypes;
- **Ownership and propagation** for the static hierarchy and causal paths;
- **Diagnostics** for malformed annotations or adapter failures.

A recording exposes exactly two axes:

- `log_time`, added by Rerun, represents physical observation time;
- `logical` represents the time component of a Boomerang tag.

The blueprint selects `logical` initially. This compresses physical idle gaps
without changing the distance between logical tag times. Scheduler state spans
are wall-clock-only and therefore appear when `log_time` is selected; logical
phase and scalar samples appear as discrete points on `logical`. Events at the
same logical time share an x-coordinate; use `boomerang.trace.microstep` to
order superdense-time events.

Logical values above `i64::MAX` remain available as raw
`boomerang.trace.logical_ns` components but cannot become Rerun timeline
coordinates.

## Topology and propagation

Registration data is timeless. It recursively records federates, enclaves,
reactors, reactions, actions, and ports. Actions and ports are owned by their
reactors; canonical paths remain stable graph IDs while compact labels keep the
Viewer readable.

Static topology describes possible trigger and propagation relationships.
Dynamic propagation describes paths actually exercised. When duplicate
candidates make the exact source ambiguous, the record remains under
`/propagation/unresolved` rather than receiving a fabricated causal edge.

## Recording privacy

Application payload values are not recorded. Recordings can contain type names,
value sizes, entity names, errors, and timing, so treat them as diagnostic
metadata and review them before sharing.
