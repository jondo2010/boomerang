//! Trait definitions for user data types that can be used in Reactor items such as ports, actions,
//! and reactors.
//!
//! - [`crate::Reactor`] has user defined data in its `state` field, which is of type [`Box<dyn
//!   ReactorData>`].
//! - [`crate::Port<T: ReactorData>`] and it's corresponding [`crate::BasePort`] have user defined
//!   data in its `value` field, which is of type [`Option<T>`].
//! - [`crate::action::store::ActionStore<T: ActionData>`] and it's corresponding
//!   [`crate::action::store::BaseActionStore`]

use std::fmt::Debug;

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
    impl SerdeData for () {}

    pub trait SerdeDataObj {}
    impl SerdeDataObj for () {}
}

#[cfg(feature = "parallel")]
mod parallel_impl {
    pub trait ParallelData: Send + Sync {}
    impl<T> ParallelData for T where T: Send + Sync {}
}

#[cfg(not(feature = "parallel"))]
mod parallel_impl {
    pub trait ParallelData {}
    impl<T> ParallelData for T {}
}

pub use parallel_impl::*;
pub use serde_impl::*;

/// Types implementing this trait can be used as data in ports, actions, and reactors.
pub trait ReactorData: Debug + SerdeData + ParallelData + 'static {}

impl<T> ReactorData for T where T: Debug + SerdeData + ParallelData + 'static {}

// declare_registry!(
//    ReactorData,
//    REACTOR_DATA_DESERIALIZE_REGISTRY,
//    REACTOR_DATA_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE
//);
