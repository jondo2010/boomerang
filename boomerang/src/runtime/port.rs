use derive_more::Display;
use downcast_rs::{impl_downcast, DowncastSync};
use tracing::event;
use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    sync::{Arc, RwLock},
};

use super::ReactionIndex;

#[derive(Display, Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub struct PortIndex(pub usize);

pub trait PortData: Debug + Copy + Clone + Send + Sync + 'static {}
impl<T> PortData for T where T: Debug + Copy + Clone + Send + Sync + 'static {}
pub type PortValue<T> = RwLock<Option<T>>;

pub trait BasePort: Debug + Display + Send + Sync + DowncastSync {
    /// Get the transitive set of Reactions that are sensitive to this Port being set.
    fn get_triggers(&self) -> &BTreeSet<ReactionIndex>;
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
    triggers: BTreeSet<ReactionIndex>,
}

impl<T> Port<T>
where
    T: PortData,
{
    pub fn new(name: String, value: PortValue<T>, triggers: BTreeSet<ReactionIndex>) -> Self {
        Self {
            name,
            value,
            triggers,
        }
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
    fn get_triggers(&self) -> &BTreeSet<ReactionIndex> {
        &self.triggers
    }

    fn cleanup(&self) {
        event!(tracing::Level::DEBUG, ?self.name, "cleanup()");

        *self.value.write().unwrap() = None;
    }
}

#[test]
fn test_port() {
    let p0: Arc<dyn BasePort> = Arc::new(Port::new(
        "p0".into(),
        RwLock::new(Some(1.0f64)),
        BTreeSet::new(),
    ));
    dbg!(&p0);
    let x = p0.downcast_ref::<Port<f64>>();
    dbg!(&x);
}
