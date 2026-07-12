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
