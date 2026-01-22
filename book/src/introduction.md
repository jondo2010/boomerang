# Introduction

Boomerang is a deterministic middleware framework for robotics and distributed systems. It provides a Rust-first implementation of the Reactors deterministic actor model, with macros that let you describe reactors directly in Rust code.

The goal of Boomerang is to make concurrent, time-sensitive systems easier to reason about by enforcing deterministic execution. Given the same sequence of inputs, Boomerang produces the same sequence of outputs and state transitions.

## Relation to Lingua Franca

Lingua Franca (https://github.com/icyphy/lingua-franca/wiki) is a close point of reference for Boomerang. The Boomerang runtime started as a Rust port of the Lingua Franca discrete-event scheduler. Instead of a separate language, Boomerang uses Rust derive macros to express reactor semantics and composition.

Boomerang aims to implement as much of the Lingua Franca language specification as is practical in a Rust-first system. Unsupported features are called out explicitly in this book.

See also Reactor Cpp: https://github.com/tud-ccc/reactor-cpp
