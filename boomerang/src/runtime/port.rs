use derive_more::Display;
use downcast_rs::{impl_downcast, DowncastSync};

use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    sync::{Arc, RwLock},
};
use tracing::event;

pub use slotmap::DefaultKey as BasePortKey;
#[derive(Clone, Copy, Derivative)]
#[derivative(Debug, Default, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub struct PortKey<T: PortData>(slotmap::KeyData, PhantomData<T>);

impl<T: PortData> From<slotmap::KeyData> for PortKey<T> {
    fn from(key: slotmap::KeyData) -> Self {
        Self(key, PhantomData)
    }
}

impl<T: PortData> slotmap::Key for PortKey<T> {
    fn data(&self) -> slotmap::KeyData {
        self.0
    }
}

pub trait PortData: Debug + Copy + Clone + Send + Sync + 'static {}
impl<T> PortData for T where T: Debug + Copy + Clone + Send + Sync + 'static {}
pub type PortValue<T> = RwLock<Option<T>>;

pub trait BasePort: Debug + Display + Send + Sync + DowncastSync {
    /// Get the transitive set of Reactions that are sensitive to this Port being set.
    // fn get_triggers(&self) -> &Vec<ReactionKey>;
    /// Reset the internal value
    fn cleanup(&self);
}
impl_downcast!(sync BasePort);

#[derive(Debug, Display)]
#[display(fmt = "{}", name)]
pub struct Port<T>
where
    T: PortData,
{
    name: String,
    value: PortValue<T>,
    // triggers: SecondaryMap<ReactionKey, ()>,
}

impl<T> Port<T>
where
    T: PortData,
{
    pub fn new(name: String, value: PortValue<T>) -> Self {
        Self { name, value }
    }

    pub fn get(&self) -> Option<T> {
        *self.value.read().unwrap()
    }

    pub fn set(self: &Arc<Self>, value: Option<T> /* , scheduler: &mut Scheduler */) {
        // assert!(
        // self.inward_binding.borrow().is_none(),
        // "set() may only be called on a ports that do not have an inward binding!"
        // );
        *self.value.write().unwrap() = value;

        // let port: Arc<dyn BasePort> = self.clone();
        // scheduler.set_port(&port);
    }

    // pub fn is_present(&self) -> bool {
    // self.inward_binding
    // .borrow()
    // .as_ref()
    // .map(|port| port.is_present())
    // .unwrap_or(self.value.borrow().is_some())
    // }
}

impl<T> BasePort for Port<T>
where
    T: PortData,
{
    // fn get_triggers(&self) -> &Vec<ReactionKey> {
    // &self.triggers.keys().collect()
    // }

    fn cleanup(&self) {
        event!(tracing::Level::DEBUG, ?self.name, "cleanup()");

        *self.value.write().unwrap() = None;
    }
}

#[test]
fn test_port() {
    let p0: Arc<dyn BasePort> = Arc::new(Port::new("p0".into(), RwLock::new(Some(1.0f64))));
    dbg!(&p0);
    let x = p0.downcast_ref::<Port<f64>>();
    dbg!(&x);
}
