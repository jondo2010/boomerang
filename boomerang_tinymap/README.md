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
build -> seal -> move to final location -> pin -> link -> execute -> drop
```

- `TinyVecBuilder`, `TinyMapBuilder`, and `TinySecondaryMapBuilder` initialize tables and are the
  only phases that can change their shape or sparse presence.
- Their sealed owners hold initialized values with a fixed shape and can move to their final owning
  location.
- Storage-erased `Ref` views expose shared runtime access. `Mut` views permit value mutation
  without insertion, removal, or replacement of the table shape.
- Whole-image pinning and linking, when stable non-owning handles are required, happen after the
  sealed ownership root reaches its final location. The root contains pre-sized `HandleTable`
  fields and exposes linking only through `Pin<&mut Root>`. The resulting `LinkedHandleTable`
  guard is shared-only and is the sole source of `Handle` values, tying them to the pinned root.

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

For hosted construction, the heap aliases make the concrete backing explicit. A sealed heap map
may also consume its values in dense order when ownership must pass to execution:

```rust
use boomerang_tinymap::{key_type, HeapSealedTinyMap, HeapTinyMapBuilder};

key_type!(EntryKey);

let mut builder = HeapTinyMapBuilder::<EntryKey, u16>::heap();
let first = builder.try_insert(10)?;
let second = builder.try_insert(20)?;
*builder.get_mut(second).unwrap() = 21;
let sealed: HeapSealedTinyMap<EntryKey, u16> = builder.seal();
let view = sealed.as_ref();
assert_eq!(view.get(first), Some(&10));
assert_eq!(view.get(second), Some(&21));
assert_eq!(sealed.into_values().collect::<Vec<_>>(), [10, 21]);
# Ok::<(), boomerang_tinymap::TinyMapError>(())
```

`TinyMapRef::from_slice` is a const adapter for static or code-generated images whose array order
is already the authoritative dense key domain. It does not allocate runtime keys or establish a
second runtime key domain.

`TinySecondaryMapBuilder` owns a sparse subset of an existing dense key domain. Its capacity is the
number of addressable parent-key slots, including absent slots, rather than the number of present
values. Insertion and replacement happen only before sealing; runtime mutation can change a present
value but cannot add or remove one. Sparse span views preserve absent slots and global keys.

```rust
use boomerang_tinymap::{
    key_type, IndexSpan, InlineSecondaryStorage, InlineStorage, TinyMapBuilder,
    TinySecondaryMapBuilder,
};

key_type!(EntryKey);

let mut owner = TinyMapBuilder::<EntryKey, (), InlineStorage<(), 4>>::inline();
let _first = owner.try_insert(())?;
let second = owner.try_insert(())?;
let third = owner.try_insert(())?;
let fourth = owner.try_insert(())?;

let mut builder = TinySecondaryMapBuilder::<
    EntryKey,
    u16,
    InlineSecondaryStorage<u16, 4>,
>::inline();
builder.try_insert(second, 10)?;
builder.try_insert(fourth, 30)?;

let sealed = builder.seal();
let view = sealed.as_ref();
let span = view.get_span(IndexSpan::new(1, 3)).unwrap();
assert_eq!(span.get(second), Some(&10));
assert_eq!(span.get(third), None);
# Ok::<(), boomerang_tinymap::TinyMapError>(())
```

The older heap-backed [`TinyMap`](https://docs.rs/boomerang_tinymap/latest/boomerang_tinymap/struct.TinyMap.html),
[`TinySecondaryMap`](https://docs.rs/boomerang_tinymap/latest/boomerang_tinymap/struct.TinySecondaryMap.html),
and [`KeySet`](https://docs.rs/boomerang_tinymap/latest/boomerang_tinymap/struct.KeySet.html) remain
available with `alloc` for hosted compatibility during migration. They retain their existing
mutable APIs and do not enforce the new sealed lifecycle. New generated-image and runtime
interfaces should use the phase-specific builder, sealed-owner, and borrowed-view types.

Generated bounded images keep heterogeneous ownership explicit: concrete runtime objects remain
fields, arrays, fixed storage, or generated enums in the pinned root. `TinyMap` does not become a
heterogeneous owner. Linking only records stable, non-owning pointers in already-sized private
handle tables; it neither allocates nor changes table shape. Handles perform no destruction, and
the linked guard must be dropped before the root. Code that needs ordered shutdown should provide
an explicit shutdown phase while the guard is live instead of relying on value destructor order.

The [`key_type!`](https://docs.rs/boomerang_tinymap/latest/boomerang_tinymap/macro.key_type.html)
macro declares transparent `u32` dense keys. Generated keys do not implement Serde traits;
applications that intentionally serialize a contextual key may add those derives explicitly.
