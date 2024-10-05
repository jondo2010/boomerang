# Changelog

All notable changes to this project will be documented in this file.

## [0.3.0] - 2024-09-29

Major refactorings and partial rewrites of nearly every crate and subsystem.

- `PhysicalAction` rewritten to support record/replay.
- Rewritten ActionStore using BinaryHeap instead of Hash, improved semantics and remove possibility of mem-overflow
- Rewrote the derive crate and interface. Reactions are now defined as structs.
- It's now possible to get the Reactor state after the scheduler has run
- Improved speed and correctness. Improved scheduler speed (as measured by the ping_pong benchmark) by several magnitudes.
- Removed essentially all allocations from the scheduler hot loop.
- Fully support generics in Reactors and Reactions
- Fully support arrays of ports and actions (banks/multiports)
- Corrected graph issue for causality loops, fixed loopback connections, fix loop in multiple_contained

### ⚙️ Miscellaneous Tasks

- Add codecov configuration file (#19)

<!-- generated by git-cliff -->