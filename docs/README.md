# Boomerang Internal Developer Documentation

This directory is for internal design and architecture notes for people working
on Boomerang itself. End-user guides belong in the mdBook under `book/src`.

Current notes:

- [Graph partitioning, federation, and replay architecture](./architecture.md)
- [Federated runtime internals](./federated-runtime.md)
- [Static federated protocol](./federated-protocol.md)

Keep these documents focused on repository structure, crate ownership,
invariants, and maintenance guidance. Public APIs and user workflows should be
documented in the book instead.
