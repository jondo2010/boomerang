# Telemetry Monitor Receiver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a bounded hosted UDP telemetry receiver with deterministic text/JSON snapshots, exposed through feature-gated `cargo boomerang monitor`.

**Architecture:** `boomerang_monitor` owns all receiver semantics: bounded source registration, per-group ordering, scheduler history, counter-rate derivation, snapshots, and presentation. It accepts the closed-system `boomerang_telemetry::TelemetryRecord` directly and documents that intentional coupling normatively in crate Rustdoc. `cargo-boomerang` only parses monitor CLI arguments and calls the crate’s standard-library receive loop; its existing build/run behavior remains unchanged unless its optional `monitor` feature is enabled.

**Tech Stack:** Rust 2021, `std::net::UdpSocket`, `std::time`, `BTreeMap`/`VecDeque`, `boomerang_telemetry`, `serde`, `serde_json`, `thiserror`, `clap`.

**Spec:** `docs/superpowers/specs/2026-09-25-telemetry-monitor-receiver-design.md`

## Global Constraints

- This is hosted-only code; `boomerang_monitor` depends on `std`, `serde`, `serde_json`, and `boomerang_telemetry`, but not on scheduler execution, Tokio, or observation ownership.
- Treat the telemetry format as internal to an atomically built/deployed closed system; decode `TelemetryRecord` directly and do not add a duplicate stable wire schema.
- Bound input at `boomerang_telemetry::MAX_DATAGRAM_BYTES`, sources, copied identity metadata, and scheduler history; accepted input must not create unbounded growth.
- Source identity is the complete record identity: run/artifact IDs, process ID/incarnation, role, optional Federate ID, and optional Enclave ID.
- Sequence is independent for `RecordGroup::Scheduler` and `RecordGroup::ExporterHealth`; stale/reordered records do not update latest values or history.
- Derive rates only from accepted scheduler records and their strictly increasing `observation_monotonic_ns`; never from packet-arrival timestamps. Preserve unavailable rates and discontinuities explicitly.
- Renderers consume one `MonitorSnapshot`, are deterministic, and must not mutate receiver state or calculate rates.
- The default monitor process listens until interrupted. Reject zero `--max-records` and malformed durations before binding; `--idle-timeout` must fail with accepted/rejected counts when the requested accepted-record count was not reached.
- Prefer observable, end-to-end behavior tests. Do not add source-layout, private-path, AST, or substring-only test seams.
- Extend the generated multi-Federate telemetry deployment proof and retain its `sensor received command 42` assertion.

---

## File Structure

- `boomerang_monitor/Cargo.toml`: hosted receiver package dependencies and lint/package metadata.
- `boomerang_monitor/src/lib.rs`: normative crate contract and public re-exports.
- `boomerang_monitor/src/receiver.rs`: bounded receiver state, ingest semantics, snapshots, and focused behavior tests.
- `boomerang_monitor/src/render.rs`: deterministic text and JSON serialization of snapshots.
- `boomerang_monitor/src/serve.rs`: standard-library UDP receive loop, command options, and automation termination semantics.
- `Cargo.toml`: workspace member and workspace-local dependency registration.
- `cargo-boomerang/Cargo.toml`: optional `monitor` feature and dependency.
- `cargo-boomerang/src/main.rs`: feature-gated `monitor` subcommand and argument conversion.
- `cargo-boomerang/tests/toolchain/build.rs`: generated multi-Federate deployment invokes the real monitor and validates its JSON result plus unchanged payload exchange.

### Task 1: Create the monitor crate and its normative public contract

**Files:**
- Create: `boomerang_monitor/Cargo.toml`
- Create: `boomerang_monitor/src/lib.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces `boomerang_monitor` as a workspace package and re-exports `Receiver`, `ReceiverConfig`, `MonitorSnapshot`, `MonitorError`, `MonitorOptions`, and `serve`.
- Produces crate-level Rustdoc that normatively defines internal-format coupling, bounded ownership, source identity, independent group ordering, rate clock, renderer purity, and the #263 boundary.

- [ ] **Step 1: Write the compile-time package smoke test**

Create `boomerang_monitor/src/lib.rs` with a minimal public test that constructs the documented default configuration:

```rust
#[test]
fn default_receiver_configuration_is_bounded() {
    let config = ReceiverConfig::default();
    assert!(config.max_sources > 0);
    assert!(config.history_capacity > 0);
    assert!(config.max_metadata_bytes > 0);
}
```

- [ ] **Step 2: Run the package test to verify it fails**

Run: `cargo test -p boomerang_monitor --locked`

Expected: FAIL because the package does not yet exist.

- [ ] **Step 3: Add the workspace package and crate Rustdoc**

Add `boomerang_monitor` to workspace members and local workspace dependencies. Create a hosted package with `boomerang_telemetry.workspace = true`, `serde = { workspace = true, features = ["derive"] }`, `serde_json.workspace = true`, and `thiserror.workspace = true`.

Start `src/lib.rs` with normative documentation equivalent to:

```rust
//! # Boomerang telemetry monitor
//!
//! This crate is the hosted receiver for Boomerang's internal telemetry records.
//! It deliberately consumes `boomerang_telemetry::TelemetryRecord` directly:
//! producer, receiver, and deployment are built atomically as one closed system,
//! so record layouts may evolve together and are not an independently stable protocol.
//!
//! `Receiver` owns bounded registry, metadata, and history state. A source is the
//! full telemetry identity; ordering is tracked independently for scheduler and
//! exporter-health groups. Scheduler rates use sender observation time, never
//! arrival time. `MonitorSnapshot` is the only rendering input; renderers do not
//! alter state or infer rates. Presentation-only dashboard work belongs above this
//! crate and must reuse these semantics.
```

Declare private `receiver`, `render`, and `serve` modules and the planned re-exports so the crate has one public boundary.

- [ ] **Step 4: Run package formatting and the smoke test**

Run: `cargo fmt --check && cargo test -p boomerang_monitor --locked`

Expected: PASS.

- [ ] **Step 5: Commit the crate boundary**

```bash
git add Cargo.toml boomerang_monitor/Cargo.toml boomerang_monitor/src/lib.rs
git commit -m "feat: add bounded telemetry monitor crate"
```

### Task 2: Implement bounded ingest, snapshots, and rate/discontinuity semantics

**Files:**
- Create: `boomerang_monitor/src/receiver.rs`
- Modify: `boomerang_monitor/src/lib.rs`

**Interfaces:**
- Consumes `TelemetryRecord::decode(input, scratch)`, `TelemetryValue`, `RecordGroup`, `ObservationSnapshot`, and `MAX_DATAGRAM_BYTES`.
- Produces `pub struct ReceiverConfig { pub max_sources: usize, pub history_capacity: usize, pub max_metadata_bytes: usize }`, `pub struct Receiver`, `pub fn Receiver::ingest(&mut self, datagram: &[u8], received_at: Duration) -> IngestOutcome`, and `pub fn Receiver::snapshot(&self, now: Duration) -> MonitorSnapshot`.
- Produces serde-serializable snapshot types with exact counters for accepted, malformed categories, source-limit rejection, stale/reordered input, skipped sequences, discontinuities, and history evictions.

- [ ] **Step 1: Write behavior tests using encoded current records**

In `receiver.rs`, create local record builders using `TelemetryRecord::encode_into` and a fixed source identity. Add focused tests that assert public snapshots, not private fields:

```rust
#[test]
fn scheduler_rate_uses_observation_time_not_receipt_time() {
    let mut receiver = Receiver::new(ReceiverConfig::default());
    receiver.ingest(&scheduler_record(0, 1_000, 10).encode(), Duration::from_secs(90));
    receiver.ingest(&scheduler_record(1, 2_000, 30).encode(), Duration::from_secs(91));
    let snapshot = receiver.snapshot(Duration::from_secs(99));
    assert_eq!(snapshot.sources[0].scheduler.as_ref().unwrap().rates.processed_tags_per_second, Some(20_000_000));
}

#[test]
fn duplicate_scheduler_record_leaves_latest_sample_unchanged() {
    let mut receiver = Receiver::new(ReceiverConfig::default());
    receiver.ingest(&scheduler_record(7, 1_000, 10).encode(), Duration::ZERO);
    receiver.ingest(&scheduler_record(7, 2_000, 99).encode(), Duration::from_secs(1));
    let scheduler = receiver.snapshot(Duration::from_secs(2)).sources[0].scheduler.unwrap();
    assert_eq!(scheduler.history.len(), 1);
    assert_eq!(scheduler.latest.raw.processed_tags, 10);
    assert_eq!(scheduler.sequence.stale_or_reordered, 1);
}

#[test]
fn exporter_health_has_independent_sequence_and_no_scheduler_history() {
    let mut receiver = Receiver::new(ReceiverConfig::default());
    receiver.ingest(&exporter_record(0, 1_000, 3, 4).encode(), Duration::ZERO);
    let source = &receiver.snapshot(Duration::from_secs(1)).sources[0];
    assert!(source.scheduler.is_none());
    assert_eq!(source.exporter_health.as_ref().unwrap().sequence.latest, Some(0));
}

#[test]
fn source_and_metadata_limits_reject_new_sources_without_growth() {
    let mut receiver = Receiver::new(ReceiverConfig { max_sources: 1, history_capacity: 2, max_metadata_bytes: 64 });
    receiver.ingest(&scheduler_record_for("one", 0).encode(), Duration::ZERO);
    receiver.ingest(&scheduler_record_for("two", 0).encode(), Duration::ZERO);
    let snapshot = receiver.snapshot(Duration::from_secs(1));
    assert_eq!(snapshot.sources.len(), 1);
    assert_eq!(snapshot.counters.source_limit_rejected, 1);
}

#[test]
fn counter_reset_saturation_or_nonmonotonic_time_marks_rates_unavailable() {
    let mut receiver = Receiver::new(ReceiverConfig::default());
    receiver.ingest(&scheduler_record(0, 2_000, 20).encode(), Duration::ZERO);
    receiver.ingest(&scheduler_record(1, 1_000, 10).encode(), Duration::from_secs(1));
    let scheduler = receiver.snapshot(Duration::from_secs(2)).sources[0].scheduler.unwrap();
    assert_eq!(scheduler.latest.rates.processed_tags_per_second, None);
    assert_eq!(scheduler.sequence.discontinuities, 1);
}
```

Also cover malformed/oversize input, gapped sequence skipped count, changed process incarnation becoming a separate source, bounded history eviction, and group-specific freshness at a supplied `now`.

- [ ] **Step 2: Run focused tests to verify they fail**

Run: `cargo test -p boomerang_monitor receiver::tests --locked`

Expected: FAIL because `Receiver` and snapshot types are not implemented.

- [ ] **Step 3: Implement the bounded state machine**

Implement these public data shapes (additional fields are permitted only when they expose an approved semantic):

```rust
pub struct Receiver {
    sources: BTreeMap<SourceKey, SourceState>,
    counters: ReceiverCounters,
}
pub struct MonitorSnapshot { pub counters: ReceiverCounters, pub sources: Vec<SourceSnapshot> }
pub struct SourceSnapshot { pub identity: SourceIdentitySnapshot, pub scheduler: Option<SchedulerSnapshot>, pub exporter_health: Option<ExporterHealthSnapshot> }
pub struct SchedulerSnapshot { pub sequence: SequenceSnapshot, pub latest: SchedulerSample, pub history: Vec<SchedulerSample>, pub history_evictions: u64 }
pub struct SequenceSnapshot { pub latest: Option<u64>, pub skipped: u64, pub stale_or_reordered: u64, pub discontinuities: u64, pub fresh_at: Option<Duration> }
```

Copy source strings into `SourceKey` only after successful decode and only if adding that key is within `max_sources` and `max_metadata_bytes`. Classify codec errors into explicit receiver counters. For each group, accept the first sequence, accept exactly-next, accept higher while adding `sequence - prior - 1` to skipped, and reject lower/equal before mutating latest values/history. Retain scheduler samples in `VecDeque` capped at `history_capacity`, incrementing evictions on pop.

For every cumulative scheduler field (`reaction_elapsed_ns`, `framework_elapsed_ns`, `physical_wait_elapsed_ns`, `external_wait_elapsed_ns`, `coordination_wait_elapsed_ns`, `processed_tags`, `processed_reactions`, `processed_events`, `set_ports`, `scheduled_actions`, and `completed_logical_tags`), calculate a per-second rate only when both timestamps and counters strictly/nondecreasing satisfy the spec. Represent unavailable rates as `Option<u64>` and count a discontinuity on a regressed/saturated counter or zero/nonmonotonic observation delta. Keep gauges raw.

- [ ] **Step 4: Run focused behavior tests**

Run: `cargo fmt --check && cargo test -p boomerang_monitor receiver::tests --locked`

Expected: PASS.

- [ ] **Step 5: Commit receiver semantics**

```bash
git add boomerang_monitor/src/lib.rs boomerang_monitor/src/receiver.rs
git commit -m "feat: ingest bounded telemetry monitor records"
```

### Task 3: Render snapshots and serve UDP with bounded automation controls

**Files:**
- Create: `boomerang_monitor/src/render.rs`
- Create: `boomerang_monitor/src/serve.rs`
- Modify: `boomerang_monitor/src/lib.rs`

**Interfaces:**
- Consumes `MonitorSnapshot` and `Receiver` from Task 2.
- Produces `pub fn render_text(snapshot: &MonitorSnapshot) -> String`, `pub fn render_json(snapshot: &MonitorSnapshot) -> Result<String, serde_json::Error>`, `pub struct MonitorOptions`, and `pub fn serve(options: &MonitorOptions) -> Result<MonitorSnapshot, MonitorError>`.

- [ ] **Step 1: Write rendering and UDP-loop behavior tests**

Test identical snapshot serialization and externally observable termination:

```rust
#[test]
fn text_and_json_render_the_same_snapshot_without_mutation() {
    let receiver = receiver_with_one_source();
    let snapshot = receiver.snapshot(Duration::from_secs(5));
    let json: MonitorSnapshot = serde_json::from_str(&render_json(&snapshot).unwrap()).unwrap();
    assert_eq!(json, snapshot);
    assert_eq!(render_text(&snapshot), render_text(&snapshot));
}

#[test]
fn serve_returns_after_requested_accepted_records() {
    let options = local_options(NonZeroUsize::new(1), Some(Duration::from_secs(1)));
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let endpoint = options.listen;
    std::thread::spawn(move || sender.send_to(&scheduler_record(0, 1, 1).encode(), endpoint).unwrap());
    assert_eq!(serve(&options).unwrap().counters.accepted, 1);
}

#[test]
fn serve_idle_timeout_reports_accepted_and_rejected_counts() {
    let options = local_options(NonZeroUsize::new(1), Some(Duration::from_millis(1)));
    let error = serve(&options).unwrap_err();
    assert!(matches!(error, MonitorError::IdleTimeout { accepted: 0, rejected: 0 }));
}
```

- [ ] **Step 2: Run the focused tests to verify they fail**

Run: `cargo test -p boomerang_monitor render::tests serve::tests --locked`

Expected: FAIL because the renderer and receive loop do not exist.

- [ ] **Step 3: Implement pure rendering and the receive loop**

Use the `BTreeMap` source order established by `Receiver` to produce text lines with source identity, group sequence/freshness, counters, raw gauge units, per-second rate units, and `unavailable` where a rate is `None`. Serialize the same `MonitorSnapshot` with `serde_json::to_string_pretty`; neither function may call `ingest` or mutate state.

Define options and errors:

```rust
pub struct MonitorOptions { pub listen: SocketAddr, pub json: bool, pub max_records: Option<NonZeroUsize>, pub idle_timeout: Option<Duration>, pub receiver: ReceiverConfig }
pub enum MonitorError { Bind(std::io::Error), Receive(std::io::Error), IdleTimeout { accepted: u64, rejected: u64 }, Json(serde_json::Error) }
```

Bind one `UdpSocket`, reuse a `[u8; MAX_DATAGRAM_BYTES]` receive buffer and scratch buffer, and record receipt time with one `Instant` origin. Count only accepted records toward `max_records`; for `idle_timeout`, set/read a socket timeout and return `IdleTimeout` if the target has not been reached. Return the final snapshot on a normal finite exit. The caller selects text or JSON after `serve` returns.

- [ ] **Step 4: Run focused renderer and server tests**

Run: `cargo fmt --check && cargo test -p boomerang_monitor render::tests serve::tests --locked`

Expected: PASS.

- [ ] **Step 5: Commit hosted monitor operation**

```bash
git add boomerang_monitor/src/lib.rs boomerang_monitor/src/render.rs boomerang_monitor/src/serve.rs
git commit -m "feat: serve and render telemetry monitor snapshots"
```

### Task 4: Wire the optional cargo subcommand and prove generated deployment behavior

**Files:**
- Modify: `cargo-boomerang/Cargo.toml`
- Modify: `cargo-boomerang/src/main.rs`
- Modify: `cargo-boomerang/tests/toolchain/build.rs`

**Interfaces:**
- Consumes `boomerang_monitor::{serve, render_json, render_text, MonitorOptions, ReceiverConfig}` when the `monitor` feature is enabled.
- Produces `cargo boomerang monitor --listen <socket-address> [--json] [--max-records N] [--idle-timeout <duration>]` only under `--features monitor`.

- [ ] **Step 1: Write CLI feature and end-to-end tests**

Add test coverage that invokes the real binary in both configurations:

```rust
#[test]
fn generated_central_deployment_feeds_monitor_json_without_changing_payload_exchange() {
    // Bind/release an ephemeral loopback port, then start the feature-enabled binary with
    // `monitor --listen <port> --json --max-records 3 --idle-timeout 5s`.
    // Run sensor-telemetry with BOOMERANG_TELEMETRY_ENDPOINT=<port>, parse monitor stdout as
    // MonitorSnapshot, and compare its scheduler source identities to the three expected pairs.
    // The deployment process must also succeed and print `sensor received command 42`.
}
```

Pass a concrete listener port determined by a temporary standard UDP bind/release, start the monitor first, use a finite `--max-records` sufficient for the three generated scheduler sources, and give the child an idle timeout to prevent a hung test. Assert structured parsed snapshot identities rather than generated source text or JSON substrings.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cargo-boomerang --test toolchain generated_central_deployment_feeds_monitor_json_without_changing_payload_exchange --locked`

Expected: FAIL because no monitor subcommand or feature exists.

- [ ] **Step 3: Add feature-gated CLI wiring**

In `cargo-boomerang/Cargo.toml`, add:

```toml
[features]
monitor = ["dep:boomerang_monitor"]

[dependencies]
boomerang_monitor = { workspace = true, optional = true }
```

Gate the `Monitor` `BoomerangCommand` variant and its imports with `#[cfg(feature = "monitor")]`. Parse `SocketAddr` through Clap, parse `--idle-timeout` with `humantime::parse_duration`, and use a nonzero parser for `--max-records`; invalid values must be Clap errors before the receiver binds. Construct `MonitorOptions`, call `serve`, and print exactly one selected rendering to stdout. Do not alter `Build`, `Check`, or `Run`.

Extend the existing generated central-deployment telemetry test to launch this feature-enabled binary/process, consume its parsed JSON result, and retain the tagged-payload assertion. Do not add tests that inspect generated Rust layout or artifact-private paths.

- [ ] **Step 4: Run feature matrix and end-to-end proof**

Run:

```bash
cargo test -p boomerang_monitor --locked
cargo check -p cargo-boomerang --locked
cargo test -p cargo-boomerang --test toolchain generated_central_deployment_feeds_monitor_json_without_changing_payload_exchange --features monitor --locked
cargo test -p cargo-boomerang --test toolchain --features monitor --locked
```

Expected: all commands PASS; the integration test validates three Federate/Enclave sources and the unchanged `sensor received command 42` output.

- [ ] **Step 5: Commit CLI and deployment proof**

```bash
git add cargo-boomerang/Cargo.toml cargo-boomerang/src/main.rs cargo-boomerang/tests/toolchain/build.rs
git commit -m "feat: add feature-gated telemetry monitor command"
```

### Task 5: Verify the whole slice and review crate documentation as the contract

**Files:**
- Modify only if verification identifies a concrete defect: files from Tasks 1–4.

**Interfaces:**
- Consumes all task outputs and the approved design spec.
- Produces verification evidence that the documented semantics, crate feature boundary, and end-to-end generated deployment are coherent.

- [ ] **Step 1: Audit normative Rustdoc against the design**

Read `boomerang_monitor/src/lib.rs` and verify its crate docs explicitly cover: closed-system direct record coupling; no compatibility promise; bounded registry/history/metadata; complete source identity; separate group sequences; observation-clock rates; pure snapshot rendering; and #263’s presentation-only reuse boundary. Add only missing normative statements.

- [ ] **Step 2: Run the full relevant verification set**

Run:

```bash
cargo fmt --check
cargo test -p boomerang_monitor --locked
cargo check -p cargo-boomerang --locked
cargo check -p cargo-boomerang --features monitor --locked
cargo test -p cargo-boomerang --test toolchain --features monitor --locked
cargo clippy -p boomerang_monitor -p cargo-boomerang --features monitor --all-targets --locked -- -D warnings
```

Expected: every command exits zero.

- [ ] **Step 3: Inspect the final diff for scope and test-seam discipline**

Run: `git diff origin/main...HEAD -- boomerang_monitor Cargo.toml cargo-boomerang`

Expected: only the new hosted monitor crate, its feature-gated CLI integration, normative docs, and behavior-focused tests are present; no producer changes, Ratatui dependencies, generated-layout assertions, or private-path test seams.

- [ ] **Step 4: Commit any verification correction**

```bash
git add boomerang_monitor Cargo.toml cargo-boomerang
git commit -m "test: verify telemetry monitor receiver"
```
