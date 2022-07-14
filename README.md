# Boomerang

[![crates.io](https://img.shields.io/crates/v/boomerang.svg)](https://crates.io/crates/boomerang)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](./LICENSE)
[![Crates.io](https://img.shields.io/crates/d/boomerang.svg)](https://crates.io/crates/boomerang)
[![Rust](https://github.com/jondo2010/boomerang/workflows/CI/badge.svg)](https://github.com/jondo2010/boomerang/actions)
[![API](https://docs.rs/boomerang/badge.svg)](https://docs.rs/boomerang)

Rust implementation of the "Reactors" Deterministic Actor Model, described by M. Lohstroh, A. Lee, et al U.C. Berlekely, [Link to paper](https://ptolemy.berkeley.edu/publications/papers/19/LohstrohEtAl_Reactors_DAC_2019.pdf).

## Comparison to Lingua-Franca

The `Lingua-Franca` project (https://github.com/icyphy/lingua-franca/wiki) serves as a point of reference for `Boomerang`.

The `Boomerang` scheduler started out as a direct Rust port of the `Lingua-Franca` Discrete-Event scheduler runtime. Instead of using a distinct "compositional language" like Lingua-Franca, Boomerang leverages the power of Rust derive-macros to directly annotate the Reactor semantics and composition. The resultant DAG is analyzed and used to generate implementation primitives for the Scheduler.

This project is still in the very early stages, but intends to implement as much of the [language specification](https://github.com/icyphy/lingua-franca/wiki/Language-Specification) and features from `Lingua-Franca` as possible.

See also [Reactor Cpp](https://github.com/tud-ccc/reactor-cpp)

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