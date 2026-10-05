# Boomerang-Tinymap

[![crates.io](https://img.shields.io/crates/v/boomerang_tinymap.svg)](https://crates.io/crates/boomerang_tinymap)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](./LICENSE)
[![Downloads](https://img.shields.io/crates/d/boomerang_tinymap.svg)](https://crates.io/crates/boomerang_tinymap)
[![CI](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jondo2010/boomerang/actions/workflows/ci.yml)
[![docs](https://docs.rs/boomerang_tinymap/badge.svg)](https://docs.rs/boomerang_tinymap)
[![codecov](https://codecov.io/github/jondo2010/boomerang/graph/badge.svg?token=PYXF8VSNY9)](https://codecov.io/github/jondo2010/boomerang)

A tiny, fast, and simple Slotkey-type map implementation for [`boomerang`](https://docs.rs/boomerang).

The crate provides dense tables and sequences for a build-once, execute-many lifecycle. New
storage-agnostic code uses phase-specific types:

```text
build -> seal -> move to final location -> borrow views -> execute -> drop
```

- `TinyVecBuilder` and `TinyMapBuilder` initialize the table and are the only phases that can
  change its shape.
- `SealedTinyVec` and `SealedTinyMap` own the initialized values with a fixed shape and can be
  moved to their final owning location.
- `TinyVecRef`/`TinyMapRef` expose shared runtime access. `TinyVecMut`/`TinyMapMut` permit value
  mutation without insertion, removal, or replacement of the table shape.
- Whole-image pinning and linking, when stable non-owning handles are required, happen above these
  table types after the sealed ownership root reaches its final location.

The backing is an owning construction detail. Choose inline storage for a compile-time capacity,
borrowed storage for caller-provided `MaybeUninit` slots, or the `alloc`-gated heap constructor for
hosted construction. Borrowed runtime views do not carry the backing type. The backing-storage
trait is intentionally private: downstream code selects a supported concrete adapter instead of
implementing storage invariants itself.

```rust
use boomerang_tinymap::{key_type, InlineStorage, TinyMapBuilder};

key_type!(EntryKey);

let mut builder = TinyMapBuilder::<EntryKey, u16, InlineStorage<u16, 4>>::inline();
let first = builder.try_insert(10)?;
let tail = builder.try_extend_exact([20, 30].into_iter())?;

let sealed = builder.seal();
let view = sealed.as_ref();
assert_eq!(view[first], 10);
assert_eq!(view.get_span(tail).unwrap().values(), [20, 30]);
# Ok::<(), boomerang_tinymap::TinyMapError>(())
```

`TinyMapBuilder` owns a complete dense key domain and generates keys in insertion order. Its
`IndexSpan` results preserve those global keys when resolved through a map view; they are distinct
from `SliceRange`, which addresses anonymous values in a packed relationship slice.

The older heap-backed [`TinyMap`](https://docs.rs/boomerang_tinymap/latest/boomerang_tinymap/struct.TinyMap.html),
[`TinySecondaryMap`](https://docs.rs/boomerang_tinymap/latest/boomerang_tinymap/struct.TinySecondaryMap.html),
and [`KeySet`](https://docs.rs/boomerang_tinymap/latest/boomerang_tinymap/struct.KeySet.html) remain
available with `alloc` for hosted compatibility during migration. They retain their existing
mutable APIs and do not enforce the new sealed lifecycle. New generated-image and runtime
interfaces should use the phase-specific builder, sealed-owner, and borrowed-view types.

The [`key_type!`](https://docs.rs/boomerang_tinymap/latest/boomerang_tinymap/macro.key_type.html)
macro declares transparent `u32` dense keys. Generated keys do not implement Serde traits;
applications that intentionally serialize a contextual key may add those derives explicitly.
