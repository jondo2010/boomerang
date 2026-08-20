# Boomerang 🪃

[![crates.io](https://img.shields.io/crates/v/boomerang.svg)](https://crates.io/crates/boomerang)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](./LICENSE)
[![Downloads](https://img.shields.io/crates/d/boomerang.svg)](https://crates.io/crates/boomerang)
[![CI](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml)
[![docs](https://docs.rs/boomerang/badge.svg)](https://docs.rs/boomerang)
[![codecov](https://codecov.io/github/jondo2010/boomerang/graph/badge.svg?token=PYXF8VSNY9)](https://codecov.io/github/jondo2010/boomerang)

Boomerang is a Rust runtime and composition framework for deterministic reactive
systems. Build reusable reactor graphs once, then run them locally or partition
them across cores, processes, and ECUs—with recording and replay at physical and
deployment boundaries.

Boomerang is early-stage. It currently provides deterministic logical-time
execution, local enclaves, modal reactors, recording/replay foundations, and
experimental static federation. Mixed-criticality and `no_std` embedded
deployment are long-term goals.

## Getting Started

```rust
use boomerang::prelude::*;

#[reactor]
fn HelloWorld() -> impl Reactor {
    timer! { t(1 s) };
}
```

## Rerun trace visualization

Enable the optional `rerun` feature to record the scheduler's structured trace
into [Rerun](https://rerun.io/). The adapter uses Rerun `0.36.1` and composes as
a `tracing_subscriber` layer; it never installs a global subscriber.

The examples below name APIs from `tracing` and `tracing-subscriber`, so a
consumer must declare them directly:

```toml
[dependencies]
boomerang = { version = "0.3", features = ["rerun"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

Add `federated` to the `boomerang` feature list for the federated example. A
direct Rerun dependency is needed only when consumer code names Rerun SDK types
or constructs a custom blueprint; the local example leaves its snapshot type
inferred. Use the same SDK configuration as the adapter:

```toml
rerun = { version = "=0.36.1", default-features = false, features = ["sdk"] }
```

Register the lowered `RuntimeAssembly` before execution consumes it. For a
local assembly, the complete flow is:

```rust
# #[cfg(feature = "rerun")]
# mod rerun_local_example {
use std::error::Error;

use boomerang::builder::RuntimeAssembly;
use boomerang::rerun::{RerunSessionBuilder, SinkConfig};
use boomerang::runtime;
use tracing_subscriber::prelude::*;

fn run_local(
    parts: RuntimeAssembly,
    config: runtime::Config,
) -> Result<(), Box<dyn Error>> {
    let session = RerunSessionBuilder::new("my-boomerang-model")
        .sink(SinkConfig::Memory)
        .build()?;
    session.register_runtime(&parts);

    let subscriber = tracing_subscriber::registry().with(session.layer());
    tracing::subscriber::with_default(subscriber, || {
        let enclaves = parts.into_local()?;
        runtime::execute_enclaves(enclaves.into_iter(), config)?;
        Ok::<_, Box<dyn Error>>(())
    })?;

    // This bounded call flushes, returns, and clears the memory recording.
    let _messages = session.take_memory_snapshot_bounded().unwrap_or_default();
    Ok(())
}
# }
```

With both `federated` and `rerun` enabled, only the final execution call changes:

```rust
# #[cfg(all(feature = "federated", feature = "rerun"))]
# mod rerun_federated_example {
use std::error::Error;

use boomerang::builder::RuntimeAssembly;
use boomerang::rerun::RerunSessionBuilder;
use boomerang::{execute_federation_in_memory, runtime};
use tracing_subscriber::prelude::*;

fn run_federated(
    parts: RuntimeAssembly,
    config: runtime::Config,
) -> Result<(), Box<dyn Error>> {
    let session = RerunSessionBuilder::new("my-federation").build()?;
    session.register_runtime(&parts);

    let subscriber = tracing_subscriber::registry().with(session.layer());
    tracing::subscriber::with_default(subscriber, || {
        let federation = parts.into_federation()?;
        execute_federation_in_memory(federation, config)?;
        Ok::<_, Box<dyn Error>>(())
    })?;
    session.flush(); // bounded by the builder's flush_timeout
    Ok(())
}
# }
```

`SinkConfig::Memory`, `SinkConfig::File(path)`, and arbitrary nested
`SinkConfig::Tee` combinations are supported. A file sink produces a
sequentially readable `.rrd`; its footer is deliberately omitted to keep
long-running memory use bounded. Run `rerun rrd optimize recording.rrd` before
using random access or `LazyStore`. A memory sink retains the full trace and
therefore grows with the recording. `take_memory_snapshot_bounded` applies only
to configurations containing a memory sink. Explicit `flush`, snapshot, and
drop use `flush_timeout` (five seconds by default); if an SDK operation never
returns, the adapter may detach its single lifecycle worker together with its
sinks.

`SinkConfig::Grpc` is intentionally rejected as `UnsupportedGrpc`. The pinned
SDK exposes a fixed blocking channel for that sink, which could otherwise hang
a disconnected scheduler callback. The supported sinks still use Rerun's
bounded batching pipeline: saturation can backpressure the scheduler, and
Boomerang adds no second event queue.

### Reading a recording

The default blueprint opens scheduler and event timelines, ownership and
propagation graphs, selected records, diagnostics, and operational measures.
Each dynamic record carries independent axes:

- `elapsed`: monotonic time since session creation;
- `wall_clock`: Unix wall-clock time for correlation with external systems;
- `logical`: the reactor tag's time component, when the event has a tag.

Selecting `logical` removes unrelated wall-clock idle time and therefore
compresses those gaps. It does not change distances between logical tag times.
Events at the same logical time share an x-coordinate; inspect the
`boomerang.trace.microstep` component to order superdense-time events.

Static registration is timeless. It recursively records federates, enclaves,
reactors, reactions, actions, and ports, with actions and ports owned by their
reactor. Entity paths use names plus lowered stable keys where needed to remain
unambiguous. These keys identify one lowered runtime graph; do not treat them as
portable IDs across changed builds. Static topology shows possible trigger and
propagation relationships, while dynamic records show the paths actually
exercised. If duplicate candidates make an exact dynamic propagation source
ambiguous, the record stays under `/propagation/unresolved` rather than gaining
a fabricated causal edge.

Application payload values are never recorded. Traces may include type names,
value sizes, entity names, errors, and timing, so treat recordings as diagnostic
metadata and review those fields before sharing them. Configuration and sink
construction failures return `RerunSessionBuildError`; no session or counters
exist yet. After a session is built, its first runtime-registration, logging,
snapshot, flush, or teardown failure disables it, increments its error counter,
emits one internal warning, and rebuilds tracing's interest cache. When no other
layer is interested, future Boomerang trace callsites are filtered before
reaching the adapter. `skipped_count` covers only attempts that still enter the
adapter after disabling, such as a racing callback or a later explicit
runtime-registration attempt. Another composed layer may keep those callsites
globally enabled, but the disabled Rerun layer's own filter still rejects its
callbacks.

Without an interested layer, trace annotations perform no metadata work. The
adapter adds no fields to `TriggerRes`, `Context`, `Scheduler`, queues, events,
or payload wrappers, so their layouts and the disabled hot path are unchanged.

The workspace currently builds on stable Rust `1.97`. Rust `1.95` is not a
supported promise: pre-existing workspace code uses `core::range::Range`, which
does not compile on that toolchain.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
