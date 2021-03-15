use downcast_rs::{impl_downcast, DowncastSync};
use std::{
    fmt::{Debug, Display},
    sync::RwLock,
};

slotmap::new_key_type! {
    pub struct PortKey;
}

pub trait PortData: Debug + Clone + Send + Sync + Default + 'static {}
impl<T> PortData for T where T: Debug + Clone + Send + Sync + Default + 'static {}

pub trait BasePort: Debug + Display + Send + Sync + DowncastSync {
    /// Reset the internal value
    fn cleanup(&self);
}
impl_downcast!(sync BasePort);

#[derive(Debug)]
pub struct Port<T: PortData> {
    name: String,
    value: RwLock<(T, bool)>,
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

impl<T> Port<T>
where
    T: PortData,
{
    pub fn new(name: String) -> Self {
        Self {
            name,
            value: RwLock::new((T::default(), false)),
        }
    }

    pub fn get_with<F: FnOnce(&T, bool)>(&self, f: F) {
        let value = self.value.read().unwrap();
        f(&value.0, value.1)
    }

    pub fn get_with_mut<F: FnOnce(&mut T, bool) -> bool>(&self, f: F) -> bool {
        let mut value = self.value.write().unwrap();
        let is_set = value.1;
        f(&mut value.0, is_set)
    }
}

impl<T> BasePort for Port<T>
where
    T: PortData,
{
    fn cleanup(&self) {
        // event!(tracing::Level::DEBUG, ?self.name, "cleanup()");
        let mut value = self.value.write().unwrap();
        (*value).1 = false;
    }
}

#[test]
fn test_port() {
    let p0: Box<dyn BasePort> = Box::new(Port::<f64>::new("p0".into()));
    dbg!(&p0);
    let x = p0.downcast_ref::<Port<f64>>();
    dbg!(&x);
}
