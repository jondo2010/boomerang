//! Macros for creating and registering data types.

pub mod __reexport {
    pub use linkme;
    pub use paste;
    pub use serde_flexitos;

    pub use crate::data::registry::{
        BASE_ACTION_STORE_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE,
        BASE_PORT_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE,
        BASE_REACTOR_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE,
        REACTION_FN_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE,
    };
}

/// Declares a registry for serializable data types and hooks it up with Serialize and Deserialize implementations.
#[macro_export]
macro_rules! declare_registry {
    ($trait_object:ident, $registry:ident, $distributed_slice:ident) => {
        #[linkme::distributed_slice]
        pub static $distributed_slice: [fn(
            &mut serde_flexitos::MapRegistry<
                dyn $trait_object,
                ::serde_flexitos::id::Ident<'static>,
            >,
        )] = [..];

        static $registry: std::sync::LazyLock<
            serde_flexitos::MapRegistry<dyn $trait_object, ::serde_flexitos::id::Ident<'static>>,
        > = std::sync::LazyLock::new(|| {
            let mut registry = serde_flexitos::MapRegistry::<
                dyn $trait_object,
                ::serde_flexitos::id::Ident<'static>,
            >::new(stringify!($trait_object));
            for registry_fn in $distributed_slice {
                registry_fn(&mut registry);
            }
            registry
        });

        impl<'a> serde::Serialize for dyn $trait_object + 'a {
            #[inline]
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                const fn __check_erased_serialize_supertrait<T: ?Sized + $trait_object>() {
                    serde_flexitos::ser::require_erased_serialize_impl::<T>();
                }
                serde_flexitos::serialize_trait_object(
                    serializer,
                    <Self as serde_flexitos::id::IdObj<::serde_flexitos::id::Ident<'static>>>::id(
                        self,
                    ),
                    self,
                )
            }
        }

        impl<'a, 'de> serde::Deserialize<'de> for Box<dyn $trait_object + 'a> {
            #[inline]
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                use serde_flexitos::Registry;
                $registry.deserialize_trait_object(deserializer)
            }
        }
    };
}

#[macro_export]
macro_rules! impl_id {
    ($ty:ty) => {
        impl $crate::data::macros::__reexport::serde_flexitos::id::Id<&'static str> for $ty {
            const ID: &'static str = stringify!($ty);
        }
        impl
            $crate::data::macros::__reexport::serde_flexitos::id::Id<
                $crate::data::macros::__reexport::serde_flexitos::id::Ident<'static>,
            > for $ty
        {
            const ID: $crate::data::macros::__reexport::serde_flexitos::id::Ident<'static> =
                $crate::data::macros::__reexport::serde_flexitos::ident!(
                    <Self as $crate::data::macros::__reexport::serde_flexitos::id::Id<
                        &'static str,
                    >>::ID
                );
        }
    };
}

/// Registers a custom data type for serialization and deserialization.
#[macro_export]
macro_rules! register_type {
    ($arg:ty) => {
        const _: () = {
            $crate::data::macros::impl_id!($arg);
            $crate::data::macros::__reexport::paste::paste! {
                #[$crate::data::macros::__reexport::linkme::distributed_slice(
                    $crate::data::macros::__reexport::BASE_ACTION_STORE_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE)]
                #[inline]
                fn [< __register_action_store_ $arg:snake >](
                    registry: &mut $crate::data::macros::__reexport::serde_flexitos::MapRegistry<dyn $crate::action::store::BaseActionStore,
                        $crate::data::macros::__reexport::serde_flexitos::id::Ident<'static>>
                ) {
                    use $crate::data::macros::__reexport::serde_flexitos::Registry;
                    registry.register_id_type::<$crate::action::store::ActionStore<$arg>>();
                }

                #[$crate::data::macros::__reexport::linkme::distributed_slice(
                    $crate::data::macros::__reexport::BASE_PORT_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE)]
                #[inline]
                fn [< __register_action_data_ $arg:snake >](
                    registry: &mut $crate::data::macros::__reexport::serde_flexitos::MapRegistry<dyn $crate::BasePort,
                        $crate::data::macros::__reexport::serde_flexitos::id::Ident<'static>>
                ) {
                    use $crate::data::macros::__reexport::serde_flexitos::Registry;
                    registry.register_id_type::<$crate::Port<$arg>>();
                }

                #[$crate::data::macros::__reexport::linkme::distributed_slice(
                    $crate::data::macros::__reexport::BASE_REACTOR_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE)]
                #[inline]
                fn [< __register_reactor_data_ $arg:snake >](
                    registry: &mut $crate::data::macros::__reexport::serde_flexitos::MapRegistry<dyn $crate::BaseReactor,
                        $crate::data::macros::__reexport::serde_flexitos::id::Ident<'static>>
                ) {
                    use $crate::data::macros::__reexport::serde_flexitos::Registry;
                    registry.register_id_type::<$crate::Reactor<$arg>>();
                }
            }
        };
    };
}

/// Registers the built-in data types for serialization and deserialization.
#[macro_export]
macro_rules! register_builtins {
    ($registry_slice:ident, $base_type:ty, $type:ty) => {
        paste::paste! {
            #[linkme::distributed_slice($registry_slice)]
            #[inline]
            fn [< __register_ $base_type:snake _ builtins >](
                registry: &mut serde_flexitos::MapRegistry<dyn $base_type, Ident<'static>>,
            ) {
                use serde_flexitos::Registry;
                registry.register_id_type::<$type<()>>();
                registry.register_id_type::<$type<bool>>();
                registry.register_id_type::<$type<char>>();
                registry.register_id_type::<$type<u8>>();
                registry.register_id_type::<$type<u16>>();
                registry.register_id_type::<$type<u32>>();
                registry.register_id_type::<$type<u64>>();
                registry.register_id_type::<$type<u128>>();
                registry.register_id_type::<$type<usize>>();
                registry.register_id_type::<$type<i8>>();
                registry.register_id_type::<$type<i16>>();
                registry.register_id_type::<$type<i32>>();
                registry.register_id_type::<$type<i64>>();
                registry.register_id_type::<$type<i128>>();
                registry.register_id_type::<$type<isize>>();
                registry.register_id_type::<$type<f32>>();
                registry.register_id_type::<$type<f64>>();
                registry.register_id_type::<$type<String>>();
                registry.register_id_type::<$type<PathBuf>>();
            }
        }
    };
}

/// Registers a reaction function for serialization and deserialization.
#[macro_export]
macro_rules! register_reaction_fn {
    (FnWrapper<$reaction_fn:ty>) => {
        $crate::data::macros::__reexport::paste::paste! {
            #[$crate::data::macros::__reexport::linkme::distributed_slice(
                $crate::data::macros::__reexport::REACTION_FN_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE)]
            #[inline]
            #[allow(non_snake_case)]
            fn [< __register_reaction_fn_ $reaction_fn:snake >](
                registry: &mut $crate::data::macros::__reexport::serde_flexitos::MapRegistry<dyn for<'store> $crate::reaction::ReactionFn<'store>,
                    $crate::data::macros::__reexport::serde_flexitos::id::Ident<'static>>
            ) {
                use $crate::data::macros::__reexport::serde_flexitos::Registry;
                registry.register($crate::reaction::BoxedReactionFn::from($reaction_fn).id(), |d| {
                    let _: () = erased_serde::deserialize(d).unwrap();
                    Ok($crate::reaction::BoxedReactionFn::from($reaction_fn))
                });
            }
        }
    };
}

pub use {impl_id, register_reaction_fn, register_type};
