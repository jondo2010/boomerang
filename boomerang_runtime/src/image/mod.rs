//! Borrowed compiled scheduler-image tables.
//!
//! # Representation contract
//!
//! Runtime entities with independent identity live in complete dense tables addressed by distinct
//! typed keys. Host builders issue those keys while constructing table shape, and complete host
//! tables seal before publication. This module exposes immutable tables to runtime and schema
//! consumers through `TinyMapRef<'a, K, V>`. Keys are never reconstructed across key domains.
//!
//! Contiguous ownership within a dense table is represented by `IndexSpan<K>`. Anonymous ordered
//! relationships are instead stored in typed backing slices and addressed by `SliceRange<T>`.
//! These representations are not interchangeable: an entity belongs in a keyed table, while a
//! range element has meaning only through the image row that owns its range.
//!
//! Stable textual identities use borrowed typed string wrappers. They are not stored in packed
//! byte blobs, and `SliceRange` is never an identity-text coordinate. Rust code generators may
//! emit them as ordinary string literals and rely on the compiler and linker for static placement.
//!
//! Consumers validate the complete concrete image before execution. Validation checks table
//! bounds, typed references, ownership spans, relationship ranges, canonical ordering, and
//! cross-table invariants; constructing a borrowed view is not itself a lowering step.

mod policy;
mod rti;
mod schema;
mod view;

pub use policy::*;
pub use rti::*;
pub use schema::*;
pub use view::*;
