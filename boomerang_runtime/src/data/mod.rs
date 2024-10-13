//! Trait definitions for user data types that can be used in Reactor items [`crate::Port`],
//! [`crate::Action`], and [`crate::Reactor`].
//!
//! # Serialization (`serde` cargo feature)
//!
//! To use a custom data type (i.e. not in the built-in list below) with Boomerang when serialization is enabled, it is
//! required to register the type with the runtime using the [`crate::data::macros::register_type!`] macro. This is
//! necessary to ensure that the type can be correctly serialized and deserialized.
//!
//! The concrete type must also naturally implement the [`serde::Serialize`] and [`serde::Deserialize`] traits.
//!
//! ## Example
//!
//! ```rust
//! use boomerang::prelude::*;
//!
//! #[derive(Debug, serde::Serialize, serde::Deserialize)]
//! struct MyData {
//!    value: u32,
//! }
//!
//! boomerang::runtime::register_type!(MyData);
//! ```
//!
//! ## Built-in types that don't require registration
//!
//! - [()] (unit tuple)
//! - [`bool`]
//! - [`char`]
//! - [`u8`]
//! - [`u16`]
//! - [`u32`]
//! - [`u64`]
//! - [`u128`]
//! - [`usize`]
//! - [`i8`]
//! - [`i16`]
//! - [`i32`]
//! - [`i64`]
//! - [`i128`]
//! - [`isize`]
//! - [`f32`]
//! - [`f64`]
//! - [`String`]
//! - [`std::path::PathBuf`]

use std::fmt::Debug;

#[cfg(feature = "serde")]
pub mod macros;

#[cfg(feature = "serde")]
pub mod registry;

#[cfg(feature = "serde")]
pub use macros::{register_reaction_fn, register_type};

#[cfg(feature = "serde")]
mod serde_impl {
    pub trait SerdeData:
        serde::Serialize + for<'de> serde::Deserialize<'de> + serde_flexitos::id::Id
    {
    }

    impl<T> SerdeData for T where
        T: serde::Serialize + for<'de> serde::Deserialize<'de> + serde_flexitos::id::Id
    {
    }

    pub trait SerdeDataObj: erased_serde::Serialize + serde_flexitos::id::IdObj {}
    impl<T> SerdeDataObj for T where T: erased_serde::Serialize + serde_flexitos::id::IdObj {}
}

#[cfg(not(feature = "serde"))]
mod serde_impl {
    pub trait SerdeData {}
    impl<T> SerdeData for T {}

    pub trait SerdeDataObj {}
    impl<T> SerdeDataObj for T {}
}

//#[cfg(feature = "parallel")]
mod parallel_impl {
    pub trait ParallelData: Send + Sync {}
    impl<T> ParallelData for T where T: Send + Sync {}
}

//#[cfg(not(feature = "parallel"))]
//mod parallel_impl {
//    pub trait ParallelData {}
//    impl<T> ParallelData for T {}
//}

pub use parallel_impl::*;
pub use serde_impl::*;

/// Types implementing this trait can be used as data in ports, actions, and reactors.
pub trait ReactorData: Debug + SerdeData + ParallelData + 'static {}

impl<T> ReactorData for T where T: Debug + SerdeData + ParallelData + 'static {}
