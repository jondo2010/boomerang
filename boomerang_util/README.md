# Boomerang Utilities

[![crates.io](https://img.shields.io/crates/v/boomerang_util.svg)](https://crates.io/crates/boomerang_util)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](./LICENSE)
[![Downloads](https://img.shields.io/crates/d/boomerang_util.svg)](https://crates.io/crates/boomerang_util)
[![CI](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml)
[![docs](https://docs.rs/boomerang_util/badge.svg)](https://docs.rs/boomerang_util)
[![codecov](https://codecov.io/github/jondo2010/boomerang/graph/badge.svg?token=PYXF8VSNY9)](https://codecov.io/github/jondo2010/boomerang)

This crate owns optional host-side support that does not belong in the core
runtime. The `launcher` feature is the canonical process-policy layer for
generated launchers. It provides the private execution-summary protocol used
by `cargo-boomerang`, without a subscriber dependency. `hosted-tracing` adds
streaming initialization and formatted retention; `bounded-tracing` independently
adds native bounded capture and shutdown export.

The independent `hosted-telemetry` feature adds `telemetry::TelemetrySource`
and the async `telemetry::run_udp` worker. Create a source from a borrowed portable
`TelemetryIdentity`, install its cloned `observation_handle()` on one scheduler,
then run the worker on a hosted Tokio executor with a fixed source slice, UDP
destination, positive sampling period, and shutdown future. Each source owns the
observation state and its matching monotonic origin. Source identifiers must be
stable logical names supplied by the caller, and the shutdown future should
complete after scheduler finalization.

The worker samples immediately and periodically, skips missed ticks, and makes
one final publication attempt at shutdown. It reuses one 1,200-byte buffer and
uses nonblocking sends with no queue or retry backlog. Each source emits independent
scheduler and exporter-health record groups; send/encoding failures increment
publication drops, and unavailable coherent snapshots increment snapshot misses.
UDP delivery remains best effort. This feature is off by default and does not
install tracing or change scheduler execution policy.

The `runner` feature retains the older convenience API that builds, lowers, and
executes a Reactor in one process. New production applications should use
`cargo-boomerang` and generated launchers. The `test-tracing` feature remains a
compatibility helper for existing tests while delegating subscriber setup to
the launcher support implementation.

## Launcher trace modes

Select the subscriber **at build time** in `Boomerang.toml`. The deployment-wide
choice applies to generated RTI and Federate executables:

```toml
[deployments.production]
tracing = "bounded" # "off", "bounded", or "hosted" (default)
```

- `hosted`: enable `hosted-tracing` and call `launcher::init_tracing()`;
  streaming output is filtered by `RUST_LOG` (off by default).
- `off`: enable only `launcher` and emit no subscriber initialization.
- `bounded`: enable `bounded-tracing` and call `launcher::init_bounded_tracing(config)`;
  by default capture `boomerang::coordination` through DEBUG independently of `RUST_LOG`,
  then write JSON lines to stderr after execution and worker shutdown.

Changing this setting rebuilds the generated artifacts. There is no runtime
backend switch; `BOOMERANG_TRACE_MODE` is not read. Off builds request neither
subscriber implementation, and bounded builds do not request the hosted
formatter/appender. Application-selected dependencies can still enable Cargo
features independently. These modes do not compile out ordinary `tracing`
instrumentation or prevent an application from installing its own subscriber.
The library features are additive, but generated launchers select one initializer.

Bounded limits and filters are compiled from optional tables. Each omitted field inherits
the deployment value, then the defaults shown here:

```toml
[deployments.production.bounded-tracing]
level = "debug"
targets = ["boomerang::coordination"]
records = 1024
fields = 32       # event plus inherited field occurrences per record
bytes = 1024      # copied event plus inherited value bytes per record
spans = 64       # simultaneously live spans, including deferred reclamation
span-fields = 16 # declared fields per span
span-bytes = 256 # copied value bytes per span
producers = 64   # simultaneously prepared emitting threads
depth = 16      # entered stack and parent ancestry depth

[deployments.production.federates.sensor.bounded-tracing]
records = 128   # other fields still inherit the deployment values

[deployments.production.rti.bounded-tracing]
records = 2048
```

These tables require `tracing = "bounded"`. Capacity values are unsigned 32-bit integers;
all except `records` must be positive, and `spans` must be below `u32::MAX`.
Zero records selects loss-only capture. Analysis rejects unknown fields,
invalid capacities and storage arithmetic overflow without allocating the
requested buffers. Validation uses the build host's layouts; target setup
validates again and may fail to reserve storage. This is not a RAM-budget or
embedded-target qualification guarantee.

`level` accepts `off`, `error`, `warn`, `info`, `debug`, or `trace`. `targets` is
an exact-name list, not a prefix match or `RUST_LOG` expression. An override
replaces the entire inherited list; `targets = []` enables all targets at the
selected level. Empty target names and surrounding whitespace are rejected.
Use `level = "off"` to disable capture while retaining the final loss report.

To include keyboard input admission and runtime construction in Snake, keep
`tracing = "bounded"` under `[deployments.snake]` and configure:

```toml
[deployments.snake.bounded-tracing]
targets = ["boomerang::coordination", "boomerang::runtime"]
level = "debug"
records = 1024
```

This rebuilds the generated executable. At orderly shutdown, stderr includes
`runtime.event.admitted` with `kind = "action"`, `origin = "physical"` or
`"logical"`, numeric `enclave`/`action` keys, and `tag_kind`, `tag_offset_ns`,
`tag_microstep`. These describe scheduler admission, not producer submission or
reaction completion. Keys are local to their documented domain: action keys
are Enclave-local; generated scheduler Enclave keys are deployment-global.
No action payload (including the pressed key) is recorded. Internally scheduled
logical actions are not asynchronous mailbox admissions and do not emit this event.
Runtime identity spans, scheduler, queue and barrier events use native fields;
durations use nanoseconds. TRACE additionally includes processed-tag events.
Application/third-party targets may still contain unsupported formatted fields;
selecting them does not make their values bounded-compatible.

Check/build resource reports include resolved `bounded_tracing` capacities and filters for
each Federate and `rti_bounded_tracing` for the RTI. They describe a process-wide
capture budget, separate from Enclave execution storage, not total RAM usage:
scratch buffers, scope copies, metadata and synchronization have additional
target-dependent overhead. Changing limits or filters changes generated source and the
artifact fingerprint, but not coordination or Federate-image compatibility.

Reference/loss-counter ceilings remain fixed at 1,024. Full rings overwrite
the oldest complete record; other capture
failures reject whole events or invalidate context and increment separate
counters. Small budgets may therefore produce incomplete traces without
changing application execution. There are no environment-based limit or filter overrides.

Shutdown JSON has `trace_schema: 1` and a process `pid`. A `kind: "record"`
line contains target, level, native fields, and root-to-leaf copied scopes.
Bytes become arrays and 128-bit integers become decimal strings; non-finite
floats use an `f64_bits` hex object. A final `kind: "loss"` line exports every
event/lifecycle counter with its saturation flag, even if no records survived.
Per-process record order is commit order, not a distributed clock. Export is
off-path and may allocate/block; abrupt process termination cannot flush it.
Export I/O failure reports to stderr and does not replace an execution failure.

The coordination schema and key domains are documented in
[boomerang_central_rti](../boomerang_central_rti/README.md#coordination-tracing).
This is diagnostic capture, not payload recording/replay, and incomplete traces
must not be treated as complete causal evidence. Runtime-construction events
are also available in hosted streaming mode. Neither this integration nor hosted
tests qualify an embedded target's timing or allocator guarantees.
