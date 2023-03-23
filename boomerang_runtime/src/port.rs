use downcast_rs::{impl_downcast, DowncastSync};
use std::{
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
};

use crate::{InnerType, LevelReactionKey, PortData};

tinymap::key_type!(pub PortKey);

pub trait BasePort: Debug + Display + Send + Sync + DowncastSync {
    /// Return the downstream Reactions triggered by this Port
    fn get_downstream(&self) -> core::slice::Iter<LevelReactionKey>;

    /// Set the downstream 'triggered' reactions.
    fn set_downstream(&mut self, downstream: Vec<LevelReactionKey>);

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
    value: Option<T>,
    /// Reactions that this Port triggers when set.
    downstream: Vec<LevelReactionKey>,
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
    pub fn new(name: String) -> Self {
        Self {
            name,
            value: None,
            downstream: Vec::new(),
        }
    }

    pub fn get(&self) -> &Option<T> {
        &self.value
    }

    pub fn get_mut(&mut self) -> &mut Option<T> {
        &mut self.value
    }
}

impl<T> BasePort for Port<T>
where
    T: PortData,
{
    fn get_downstream(&self) -> core::slice::Iter<LevelReactionKey> {
        self.downstream.iter()
    }

    fn set_downstream(&mut self, downstream: Vec<LevelReactionKey>) {
        self.downstream = downstream;
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
    let p0: Box<dyn BasePort> = Box::new(Port::<f64>::new("p0".into()));
    dbg!(&p0);
    let x = p0.downcast_ref::<Port<f64>>();
    dbg!(&x);
}
