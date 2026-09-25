# Bounded Telemetry Monitor Receiver Design

## Status

Approved design for the first implementation slice of #262. This slice consumes
the internal v1 telemetry records produced by merged #265 and deliberately does
not add terminal widgets, producer changes, RTI metrics, or OS probes.

## Goal

Provide a hosted, bounded UDP receiver that turns `TelemetryRecord` datagrams
into deterministic text or JSON snapshots. The receiver establishes the source,
freshness, sequence, discontinuity, and rate semantics that the later Ratatui
dashboard in #263 will reuse.

## Scope

- Create the hosted `boomerang_monitor` workspace crate.
- Decode bounded `boomerang_telemetry::TelemetryRecord` datagrams.
- Register a bounded set of stable sources and retain bounded scheduler history.
- Track malformed input, source-limit rejection, independent group sequences,
  source freshness, and counter discontinuities.
- Derive scheduler counter rates from successive `observation_monotonic_ns`
  values, never packet arrival time.
- Expose one deterministic snapshot model with default text and `--json`
  renderings.
- Add `cargo boomerang monitor --listen <socket-address>` behind the optional
  `cargo-boomerang` `monitor` feature.
- Keep the default CLI listening until interrupted. `--max-records <N>` exits
  after N accepted datagrams; `--idle-timeout <duration>` produces a bounded,
  actionable no-input failure for automation.
- Use the existing generated multi-Federate deployment vector as the primary
  proof: a real listener accepts records while the deployment still proves
  `sensor received command 42`.

## Non-goals

- Ratatui widgets, charts, interactive source selection, or manual dashboard
  walkthroughs (#263).
- Central-RTI producer telemetry, CPU/RSS probes, or direct-launch parity
  remaining in #261.
- Application values, retransmission, reliable audit, replay, tracing, or
  coordination transport.
- Cross-source utilization percentages and aggregates. Independent source
  clocks remain separate in this slice.

## Crate and feature boundary

`boomerang_monitor` is a new hosted workspace crate. It depends on
`boomerang_telemetry`, `std`, `serde`, and `serde_json`; it has no dependency on
`boomerang_runtime` observation ownership, scheduler execution, or Tokio.

`cargo-boomerang` declares an optional `monitor` feature that enables its
optional `boomerang_monitor` dependency. The monitor subcommand is compiled
only with that feature. Builds without it acquire neither receiver code nor its
hosted dependencies. The existing generated launcher/exporter feature boundary
is unchanged.

## Receiver state model

`ReceiverConfig` owns explicit hosted limits:

- `max_sources`: maximum registered source identities.
- `history_capacity`: scheduler samples retained per source.
- `max_metadata_bytes`: aggregate copied identity text accepted by the registry.

`Receiver` owns a `BTreeMap<SourceKey, SourceState>` and aggregate input
counters. `SourceKey` copies the entire stable record identity: run ID, artifact
ID, process ID and incarnation, source role, Federate ID, and Enclave ID. No
runtime dense keys, process ordinal inference, or string re-parsing is used.

Each `SourceState` owns independent sequence trackers for `Scheduler` and
`ExporterHealth`, the latest values for each group, and a fixed-capacity ring of
scheduler samples. A sample stores the record's observation time, lifecycle,
phase, raw scheduler snapshot, and derived rates. When history is full, the
oldest sample is evicted and the eviction count is exposed in the snapshot.

New sources that exceed either source count or metadata-byte limits are rejected
and counted. Accepted sources never cause unbounded registry or history growth.

## Datagram, ordering, and discontinuity semantics

`Receiver::ingest(datagram, received_at)` first bounds the input to
`MAX_DATAGRAM_BYTES`, then calls `TelemetryRecord::decode` with caller-owned
scratch storage. Invalid, noncanonical, trailing, unsupported-version, and
group-mismatched records are counted by category and do not register a source.

For each source and group:

- The first sequence is accepted.
- A strictly next sequence is continuous.
- A higher sequence is accepted and records the skipped count.
- A repeated or lower sequence is rejected as stale/reordered and leaves latest
  values and history unchanged.
- A changed source identity creates a distinct source. A changed process
  incarnation is consequently a fresh source and never creates a false rate.

Scheduler counter rates are only available when two accepted scheduler records
have strictly increasing `observation_monotonic_ns` and nondecreasing counter
values. Counter regression, a saturated `u64::MAX` value, or a zero/nonmonotonic
time delta records a discontinuity and yields unavailable rates for that sample.
Gauge values remain sampled values and are never transformed into rates. Sender
observation time is reported independently from local receipt freshness.

Exporter-health records update only exporter-health state. They never create
scheduler history or imply fresh scheduler observations.

## Snapshot and rendering

`Receiver::snapshot(now)` returns serializable `MonitorSnapshot`. It contains:

- receiver counters by accepted/rejected input category;
- source identity and group-specific sequence/freshness/discontinuity state;
- latest scheduler lifecycle, phase, raw gauges, cumulative counters, and
  derived per-second rates with explicit availability;
- latest exporter-health values; and
- bounded-history/registry overflow counters.

The text renderer has a deterministic source ordering from `BTreeMap` and
labels all values with units or availability. JSON serializes the exact same
snapshot model. Neither renderer calculates new rates or changes receiver state.

## CLI

With `cargo-boomerang` built using `--features monitor`:

```text
cargo boomerang monitor --listen 127.0.0.1:9000 [--json]
                       [--max-records N] [--idle-timeout 2s]
```

The command binds a standard-library UDP socket, receives datagrams into one
`MAX_DATAGRAM_BYTES` buffer, supplies monotonic receipt times to `Receiver`,
and renders a snapshot at normal exit. It rejects invalid socket addresses,
zero `--max-records`, and invalid durations before binding. Hitting the idle
timeout before the requested accepted record count is an error that includes
accepted and rejected counts. Interrupt cleanup and interactive terminal state
belong to #263 because this slice does not enter raw terminal mode.

## Verification

Focused receiver tests use encoded current telemetry records, not golden-wire
compatibility fixtures. They cover bounded source/history metadata limits,
malformed input, duplicate/reordered/gapped sequences, source incarnation,
counter reset/saturation/nonmonotonic time discontinuities, and independent
scheduler/exporter-health freshness.

The primary integration test extends the existing generated central-deployment
fixture. It starts a real local monitor listener with a finite accepted-record
limit, launches the existing hosted telemetry deployment, reads the resulting
JSON snapshot, and proves the expected three Federate/Enclave sources plus the
unchanged tagged payload result. It does not inspect generated launcher source.

Feature-gate tests compile and invoke existing build/run commands without the
monitor feature, then compile the monitor command with the feature enabled.

## Follow-up boundary

#263 consumes `MonitorSnapshot` and receiver history for Ratatui presentation.
It must not decode records, infer identity, calculate rates, or define sequence
semantics independently. Remaining #261 producer work adds new records only by
extending the telemetry core and corresponding receiver transformations.
