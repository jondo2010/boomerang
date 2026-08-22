# boomerang_rerun

`boomerang_rerun` is the Boomerang-specific adapter from structured `tracing`
spans and events to Rerun recordings. It is deliberately separate from the
runtime, builder, and `boomerang` facade so visualization policy and the heavy
Rerun SDK dependency do not become runtime concerns.

End-user setup and Viewer guidance live in the
[Boomerang book](../book/src/rerun.md). This README documents the adapter's
internal boundary and maintenance contract.

## Crate boundary

The crate depends directly on `boomerang_builder`, `boomerang_runtime`, and
`boomerang_tinymap`. Its `federated` feature enables the builder's federated
lowering output. It does not depend on the `boomerang` facade.

The facade keeps source compatibility by re-exporting this crate as
`boomerang::rerun` behind its `rerun` feature. `boomerang_util` uses that public
path when it installs the layer for runner-based applications.

The modules have distinct responsibilities:

- `session` owns sink validation, `RecordingStream` construction, the default
  blueprint, static runtime registration, bounded lifecycle operations, and
  finalized-file verification.
- `layer` maps Boomerang trace spans and events directly to dense Rerun
  archetypes and performs adapter-local causal correlation.
- `entities` maps already-lowered runtime metadata to timeless Rerun entities,
  ownership graphs, and static propagation relationships.

There is intentionally no generic intermediate trace-record schema. Runtime
crates emit typed tracing annotations; this adapter interprets them and writes
the corresponding Rerun archetype directly.

## Recording contract

Static topology is written with `log_static` and therefore carries no timeline.
Dynamic recording data has exactly two time axes:

- Rerun's implicit `log_time` for physical observation order;
- `logical` for representable Boomerang logical tag times.

Scheduler `StateChange` set/reset records use only `log_time`, so their spans
remain meaningful in wall-clock time. Logical phase and operational scalar
observations carry `logical` and `log_time`, and scalar presentation is
registered as static `SeriesPoints` metadata.

Canonical entity paths remain graph node IDs. Compact labels are presentation
metadata derived from lowered registration identities, include stable-key
suffixes where necessary, and are bounded to avoid oversized legends.

Trace annotations must not add state to runtime hot-path objects such as
`TriggerRes`, scheduler queues, events, contexts, or payload wrappers. Causal
state belongs to the adapter and exists only while its layer is enabled.

## Lifecycle and backpressure

Memory, file, and tee sinks use Rerun 0.36.1's bounded batching pipeline. A
memory sink retains the complete trace. File sinks retain an O(chunks) footer
manifest and construct a finalized footer using O(chunks) memory.

The SDK's gRPC sink is rejected because its blocking behavior cannot be
isolated from scheduler callbacks. Supported sinks can still backpressure a
saturated scheduler; the adapter deliberately adds no second event queue.
Further live-viewer latency work is tracked in
[#106](https://github.com/jondo2010/boomerang/issues/106).

`RerunSession::finish` is the authoritative offline success result. It performs
bounded flush, disconnect, teardown, and RRD footer verification while
preserving earlier observational failures. `Drop` is bounded best-effort only.
The first adapter failure disables the session and refreshes tracing interest so
uninterested runtime callsites become cheap again.

## Verification

The focused adapter suite closes the loop from tracing spans/events through a
finalized RRD and decodes the file again:

```console
cargo test -p boomerang_rerun
```

The facade and federated compatibility paths are covered separately:

```console
cargo test -p boomerang --features federated,rerun --test federated_static
BOOM_RERUN=1 cargo test -p boomerang --features rerun --test enclave_cycle enclave_cycle -- --exact
rerun rrd verify target/boomerang/diagrams/enclave_cycle.rrd
```
