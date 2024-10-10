use std::path::PathBuf;

use crate::{declare_registry, ReactorData};

use super::{BasePort, Port};

declare_registry!(
    BasePort,
    BASE_PORT_DESERIALIZE_REGISTRY,
    BASE_PORT_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE
);

impl<T: ReactorData> serde_flexitos::id::Id<serde_flexitos::id::Ident<'static>> for Port<T> {
    const ID: serde_flexitos::id::Ident<'static> =
        serde_flexitos::id::Ident::I1("Port").extend(T::ID);
}

impl<T: ReactorData> From<Port<T>> for Box<dyn BasePort> {
    #[inline]
    fn from(val: Port<T>) -> Self {
        Box::new(val)
    }
}

#[linkme::distributed_slice(BASE_PORT_DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE)]
#[inline]
fn __register_builtins(
    registry: &mut serde_flexitos::MapRegistry<dyn BasePort, serde_flexitos::id::Ident<'static>>,
) {
    use serde_flexitos::Registry;
    registry.register_id_type::<Port<()>>();
    registry.register_id_type::<Port<bool>>();
    registry.register_id_type::<Port<char>>();
    registry.register_id_type::<Port<u8>>();
    registry.register_id_type::<Port<u16>>();
    registry.register_id_type::<Port<u32>>();
    registry.register_id_type::<Port<u64>>();
    registry.register_id_type::<Port<u128>>();
    registry.register_id_type::<Port<usize>>();
    registry.register_id_type::<Port<i8>>();
    registry.register_id_type::<Port<i16>>();
    registry.register_id_type::<Port<i32>>();
    registry.register_id_type::<Port<i64>>();
    registry.register_id_type::<Port<i128>>();
    registry.register_id_type::<Port<isize>>();
    registry.register_id_type::<Port<f32>>();
    registry.register_id_type::<Port<f64>>();
    // registry.register_id_type::<ActionStore<&str>>();
    registry.register_id_type::<Port<String>>();
    registry.register_id_type::<Port<PathBuf>>();
    // registry.register_id_type::<ActionStore<Path>>();
    // registry.register_id_type::<ActionStore<SystemTime>>();
}

#[test]
fn test() {
    let ports = vec![
        Port::<bool>::new("test", super::PortKey::from(0)).boxed(),
        Port::<f32>::new("test2", super::PortKey::from(1)).boxed(),
    ];

    let serialized = serde_json::to_string(&ports).unwrap();
    println!("serialized = {}", serialized);

    let deserialized: Vec<Box<dyn BasePort>> = serde_json::from_str(&serialized).unwrap();
    dbg!(&deserialized);
}
