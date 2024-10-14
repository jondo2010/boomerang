//! This module contains the registry for serializable data types.

use std::path::PathBuf;

use serde_flexitos::id::{Id, Ident};

use crate::{
    action::store::{ActionStore, BaseActionStore},
    declare_registry, register_builtins, BasePort, BaseReactor, BoxedReactionFn, Port, ReactionFn,
    Reactor,
};

use super::ReactorData;

// *** Registry declarations ***

declare_registry!(
    BaseActionStore,
    BASE_ACTION_STORE_DESERIALIZE_REGISTRY,
    BASE_ACTION_STORE_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE
);

declare_registry!(
    BasePort,
    BASE_PORT_DESERIALIZE_REGISTRY,
    BASE_PORT_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE
);

declare_registry!(
    BaseReactor,
    BASE_REACTOR_DESERIALIZE_REGISTRY,
    BASE_REACTOR_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE
);

type ReactionFunctionMapRegistry = serde_flexitos::MapRegistry<
    dyn for<'store> ReactionFn<'store>,
    serde_flexitos::id::Ident<'static>,
>;

#[linkme::distributed_slice]
pub static REACTION_FN_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE: [fn(
    &mut ReactionFunctionMapRegistry,
)];

static REACTION_FN_DESERIALIZE_REGISTRY: std::sync::LazyLock<
    serde_flexitos::MapRegistry<
        dyn for<'store> ReactionFn<'store>,
        serde_flexitos::id::Ident<'static>,
    >,
> = std::sync::LazyLock::new(|| {
    let mut registry = ReactionFunctionMapRegistry::new(stringify!(ReactionFn));
    for registry_fn in REACTION_FN_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE {
        registry_fn(&mut registry);
    }
    registry
});

impl<'a> serde::Serialize for dyn for<'store> ReactionFn<'store> + 'a {
    #[inline]
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        const fn __check_erased_serialize_supertrait<T: ?Sized + for<'store> ReactionFn<'store>>() {
            serde_flexitos::ser::require_erased_serialize_impl::<T>();
        }
        serde_flexitos::serialize_trait_object(
            serializer,
            <Self as serde_flexitos::id::IdObj<::serde_flexitos::id::Ident<'static>>>::id(self),
            self,
        )
    }
}

impl<'de> serde::Deserialize<'de> for BoxedReactionFn {
    #[inline]
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde_flexitos::Registry;
        REACTION_FN_DESERIALIZE_REGISTRY.deserialize_trait_object(deserializer)
    }
}

// *** Id implementations ***

impl<T: ReactorData> Id<Ident<'static>> for ActionStore<T> {
    const ID: Ident<'static> = Ident::I1("ActionStore").extend(T::ID);
}

impl<T: ReactorData> Id<Ident<'static>> for Port<T> {
    const ID: Ident<'static> = Ident::I1("Port").extend(T::ID);
}

impl<T: ReactorData> Id<Ident<'static>> for Reactor<T> {
    const ID: Ident<'static> = Ident::I1("Reactor").extend(T::ID);
}

impl<T: ReactorData> From<ActionStore<T>> for Box<dyn BaseActionStore> {
    #[inline]
    fn from(val: ActionStore<T>) -> Self {
        Box::new(val)
    }
}

impl<T: ReactorData> From<Port<T>> for Box<dyn BasePort> {
    #[inline]
    fn from(val: Port<T>) -> Self {
        Box::new(val)
    }
}

impl<T: ReactorData> From<Reactor<T>> for Box<dyn BaseReactor> {
    #[inline]
    fn from(val: Reactor<T>) -> Self {
        Box::new(val)
    }
}

register_builtins!(
    BASE_PORT_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE,
    BasePort,
    Port
);

register_builtins!(
    BASE_ACTION_STORE_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE,
    BaseActionStore,
    ActionStore
);

register_builtins!(
    BASE_REACTOR_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE,
    BaseReactor,
    Reactor
);

#[cfg(test)]
mod tests {
    use crate::PortKey;

    use super::*;
    #[test]
    fn test_ports_roundtrip() {
        let ports = vec![
            Port::<bool>::new("test", PortKey::from(0)).boxed(),
            Port::<f32>::new("test2", PortKey::from(1)).boxed(),
        ];

        let serialized = serde_json::to_string(&ports).unwrap();
        println!("serialized = {}", serialized);

        let deserialized: Vec<Box<dyn BasePort>> = serde_json::from_str(&serialized).unwrap();
        dbg!(&deserialized);
    }
}
