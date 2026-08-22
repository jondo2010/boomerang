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

- `session` owns file-backed `RecordingStream` construction, the default
  timeline-first blueprint, static runtime registration, finalization, and RRD
  footer verification.
- `layer` maps Boomerang trace spans and events directly to dense Rerun
  archetypes.
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
`TriggerRes`, scheduler queues, events, contexts, or payload wrappers. Adapter
state exists only while its layer is enabled.

## File lifecycle

`RerunSession::save(application_id, path)` is the only constructor. It creates a
file-backed recording with the default blueprint. Call `register_runtime` before
execution consumes the lowered assembly, install `layer` in the active tracing
subscriber, and call `finish` after execution.

`finish` is the authoritative success result for the offline recording: it
finalizes the file, verifies that the RRD footer can be decoded, and reports any
earlier recording failure.

## Verification

The existing federation integration test closes the loop from tracing
spans/events through a finalized RRD and decodes the file again:

```console
cargo test -p boomerang --features federated,rerun \
  --test federated_static \
  public_api_runs_static_federation_with_finalized_rrd_trace -- --exact
```

The facade and federated compatibility paths are covered separately:

```console
cargo test -p boomerang --features federated,rerun --test federated_static
BOOM_RERUN=1 cargo test -p boomerang --features rerun --test enclave_cycle enclave_cycle -- --exact
rerun rrd verify target/boomerang/diagrams/enclave_cycle.rrd
```
