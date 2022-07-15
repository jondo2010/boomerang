use downcast_rs::{impl_downcast, DowncastSync};
use std::{
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
};

use crate::{InnerType, PortData};

slotmap::new_key_type! {
    pub struct PortKey;
}

pub trait BasePort: Debug + Display + Send + Sync + DowncastSync {
    /// Get the associated PortKey for this Port
    fn get_key(&self) -> PortKey;

    /// Return true if the port contains a value
    fn is_set(&self) -> bool;

    /// Reset the internal value
    fn cleanup(&mut self);

    /// Get the internal type name str
    fn type_name(&self) -> &'static str;
}
impl_downcast!(sync BasePort);

#[derive(Debug)]
pub struct Port<T: PortData> {
    name: String,
    key: PortKey,
    value: Option<T>,
}

impl<T: PortData> Display for Port<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "Port<{}> \"{}\"",
            std::any::type_name::<T>(),
            self.name
        ))
    }
}

impl<T: PortData> Deref for Port<T> {
    type Target = Option<T>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: PortData> DerefMut for Port<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: PortData> InnerType for Port<T> {
    type Inner = T;
}

impl<T> Port<T>
where
    T: PortData,
{
    pub fn new(name: String, key: PortKey) -> Self {
        Self {
            name,
            key,
            value: None,
        }
    }

    pub fn get(&self) -> &Option<T> {
        &self.value
    }

    pub fn get_mut(&mut self) -> &mut Option<T> {
        // let downstream = ctx.dep_info.triggered_by_port(port_key);
        // ctx.enqueue_now(downstream)
        &mut self.value
    }
}

impl<T> BasePort for Port<T>
where
    T: PortData,
{
    fn get_key(&self) -> PortKey {
        self.key
    }

    fn is_set(&self) -> bool {
        self.value.is_some()
    }

    fn cleanup(&mut self) {
        // event!(tracing::Level::DEBUG, ?self.name, "cleanup()");
        self.value = None;
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }
}

#[test]
fn test_port() {
    let key = PortKey::from(slotmap::KeyData::from_ffi(1));
    let p0: Box<dyn BasePort> = Box::new(Port::<f64>::new("p0".into(), key));
    dbg!(&p0);
    let x = p0.downcast_ref::<Port<f64>>();
    dbg!(&x);
}
