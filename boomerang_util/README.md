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
  capture `boomerang::coordination` through DEBUG independently of `RUST_LOG`,
  then write JSON lines to stderr after execution and worker shutdown.

Changing this setting rebuilds the generated artifacts. There is no runtime
backend switch; `BOOMERANG_TRACE_MODE` is not read. Off builds request neither
subscriber implementation, and bounded builds do not request the hosted
formatter/appender. Application-selected dependencies can still enable Cargo
features independently. These modes do not compile out ordinary `tracing`
instrumentation or prevent an application from installing its own subscriber.
The library features are additive, but generated launchers select one initializer.

Bounded limits are compiled from optional tables. Each omitted field inherits
the deployment value, then the defaults shown here:

```toml
[deployments.production.bounded-tracing]
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

These tables require `tracing = "bounded"`. Values are unsigned 32-bit integers;
all except `records` must be positive, and `spans` must be below `u32::MAX`.
Zero records selects loss-only capture. Analysis rejects unknown fields,
invalid capacities and storage arithmetic overflow without allocating the
requested buffers. Validation uses the build host's layouts; target setup
validates again and may fail to reserve storage. This is not a RAM-budget or
embedded-target qualification guarantee.

Check/build resource reports include resolved `bounded_tracing` capacities for
each Federate and `rti_bounded_tracing` for the RTI. They describe a process-wide
capture budget, separate from Enclave execution storage, not total RAM usage:
scratch buffers, scope copies, metadata and synchronization have additional
target-dependent overhead. Changing limits changes generated source and the
artifact fingerprint, but not coordination or Federate-image compatibility.

The coordination-only DEBUG filter and reference/loss-counter ceilings (1,024)
are fixed. Full rings overwrite the oldest complete record; other capture
failures reject whole events or invalidate context and increment separate
counters. Small budgets may therefore produce incomplete traces without
changing application execution. There are no environment-based limit overrides.

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
must not be treated as complete causal evidence. Hosted runtime-construction
events remain available in streaming mode. Neither this integration nor hosted
tests qualify an embedded target's timing or allocator guarantees.
