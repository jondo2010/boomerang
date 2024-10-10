use downcast_rs::{impl_downcast, Downcast};

use crate::declare_registry;

#[cfg(feature = "serde")]
mod serde_impl {
    pub trait PortData:
        std::fmt::Debug
        + Send
        + Sync
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + serde_flexitos::id::Id
        + 'static
    {
    }

    impl<T> PortData for T where
        T: std::fmt::Debug
            + Send
            + Sync
            + serde::Serialize
            + for<'de> serde::Deserialize<'de>
            + serde_flexitos::id::Id
            + 'static
    {
    }

    pub trait ActionData:
        std::fmt::Debug
        + Send
        + Sync
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + serde_flexitos::id::Id
        + 'static
    {
    }

    impl<T> ActionData for T where
        T: std::fmt::Debug
            + Send
            + Sync
            + serde::Serialize
            + for<'de> serde::Deserialize<'de>
            + serde_flexitos::id::Id
            + 'static
    {
    }
}

#[cfg(not(feature = "serde"))]
mod non_serde_impl {
    pub trait PortData: std::fmt::Debug + Send + Sync + 'static {}
    impl<T> PortData for T where T: std::fmt::Debug + Send + Sync + 'static {}

    pub trait ActionData: std::fmt::Debug + Send + Sync + 'static {}

    impl<T> ActionData for T where T: std::fmt::Debug + Send + Sync + 'static {}
}

#[cfg(feature = "parallel")]
pub trait ReactorState: Downcast + Send + Sync {
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

#[cfg(feature = "parallel")]
impl<T> ReactorState for T where T: Downcast + Send + Sync {}

#[cfg(not(feature = "parallel"))]
pub trait ReactorState: Downcast {
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

#[cfg(not(feature = "parallel"))]
impl<T> ReactorState for T where T: Downcast {}

impl_downcast!(ReactorState);

declare_registry!(
    ReactorState,
    REACTOR_STATE_DESERIALIZE_REGISTRY,
    REACTOR_STATE_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE
);
