use std::path::PathBuf;

use crate::{
    action::store::{ActionStore, BaseActionStore},
    declare_registry, ReactorData,
};

// declare_registry!(
//    ActionData,
//    ACTION_DATA_DESERIALIZE_REGISTRY,
//    ACTION_DATA_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE
//);

declare_registry!(
    BaseActionStore,
    BASE_ACTION_STORE_DESERIALIZE_REGISTRY,
    BASE_ACTION_STORE_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE
);

#[macro_export]
macro_rules! register_action_data {
    ($arg:ty) => {
        #[linkme::distributed_slice(BASE_ACTION_STORE_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE)]
        #[inline]
        fn [< __register_action_store_ $arg:snake >](
            registry: &mut serde_flexitos::MapRegistry<dyn BaseActionStore,
            ::serde_flexitos::id::Ident<'static>>
        ) {
            use serde_flexitos::Registry;
            registry.register_id_type::<ActionStore<$arg>>();
        }

        #[linkme::distributed_slice(ACTION_DATA_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE)]
        #[inline]
        fn [< __register_action_data_ $arg:snake >](
            registry: &mut serde_flexitos::MapRegistry<dyn ActionData,
            ::serde_flexitos::id::Ident<'static>>
        ) {
            use serde_flexitos::Registry;
            registry.register_id_type::<$arg>();
        }
    };
}

impl<T: ReactorData> serde_flexitos::id::Id<serde_flexitos::id::Ident<'static>> for ActionStore<T> {
    const ID: serde_flexitos::id::Ident<'static> =
        serde_flexitos::id::Ident::I1("ActionStore").extend(T::ID);
}

impl<T: ReactorData> From<ActionStore<T>> for Box<dyn BaseActionStore> {
    #[inline]
    fn from(val: ActionStore<T>) -> Self {
        Box::new(val)
    }
}

#[linkme::distributed_slice(BASE_ACTION_STORE_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE)]
#[inline]
fn __register_builtins(
    registry: &mut serde_flexitos::MapRegistry<
        dyn BaseActionStore,
        serde_flexitos::id::Ident<'static>,
    >,
) {
    use serde_flexitos::Registry;
    registry.register_id_type::<ActionStore<()>>();
    registry.register_id_type::<ActionStore<bool>>();
    registry.register_id_type::<ActionStore<char>>();
    registry.register_id_type::<ActionStore<u8>>();
    registry.register_id_type::<ActionStore<u16>>();
    registry.register_id_type::<ActionStore<u32>>();
    registry.register_id_type::<ActionStore<u64>>();
    registry.register_id_type::<ActionStore<u128>>();
    registry.register_id_type::<ActionStore<usize>>();
    registry.register_id_type::<ActionStore<i8>>();
    registry.register_id_type::<ActionStore<i16>>();
    registry.register_id_type::<ActionStore<i32>>();
    registry.register_id_type::<ActionStore<i64>>();
    registry.register_id_type::<ActionStore<i128>>();
    registry.register_id_type::<ActionStore<isize>>();
    registry.register_id_type::<ActionStore<f32>>();
    registry.register_id_type::<ActionStore<f64>>();
    // registry.register_id_type::<ActionStore<&str>>();
    registry.register_id_type::<ActionStore<String>>();
    registry.register_id_type::<ActionStore<PathBuf>>();
    // registry.register_id_type::<ActionStore<Path>>();
    // registry.register_id_type::<ActionStore<SystemTime>>();
}
