# Introduction

Boomerang is built around a Rust-first implementation of the "Reactors" Deterministic Actor Model, described by M. Lohstroh, A. Lee, et al U.C. Berlekely, [Link to paper](https://ptolemy.berkeley.edu/publications/papers/19/LohstrohEtAl_Reactors_DAC_2019.pdf).

## Comparison to Lingua-Franca

The `Lingua-Franca` project (https://github.com/icyphy/lingua-franca/wiki) serves as a point of reference for `Boomerang`.

The `Boomerang` scheduler started out as a direct Rust port of the `Lingua-Franca` Discrete-Event scheduler runtime. Instead of using a distinct "compositional language" like Lingua-Franca, Boomerang leverages the power of Rust derive-macros to directly annotate the Reactor semantics and composition. The resultant DAG is analyzed and used to generate implementation primitives for the Scheduler.

This project is still in the very early stages, but intends to implement as much of the [language specification](https://github.com/icyphy/lingua-franca/wiki/Language-Specification) and features from `Lingua-Franca` as possible.

See also [Reactor Cpp](https://github.com/tud-ccc/reactor-cpp)
