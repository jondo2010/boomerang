use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{Action, ActionData, ActionKey, ActionValues, BaseActionValues};
use crate::{InnerType, Tag};

pub trait ActionRefValue<T: ActionData> {
    fn get_value(&self, tag: Tag) -> Option<T>;
    fn set_value(&mut self, value: Option<T>, new_tag: Tag);
    fn get_min_delay(&self) -> Duration;
    fn get_key(&self) -> ActionKey;
}

/// An `ActionRef` is a reference to an `Action` that can be scheduled.
pub struct ActionRef<'a, T: ActionData = ()> {
    pub(crate) key: ActionKey,
    pub(crate) min_delay: Duration,
    pub(crate) values: &'a mut ActionValues<T>,
}

impl<'a, T: ActionData> ActionRefValue<T> for ActionRef<'a, T> {
    fn get_value(&self, tag: Tag) -> Option<T> {
        self.values.get_value(tag).cloned()
    }

    fn set_value(&mut self, value: Option<T>, new_tag: Tag) {
        self.values.set_value(value, new_tag)
    }

    fn get_min_delay(&self) -> Duration {
        self.min_delay
    }

    fn get_key(&self) -> ActionKey {
        self.key
    }
}

impl<'a, T: ActionData> InnerType for ActionRef<'a, T> {
    type Inner = T;
}

#[derive(Debug, Clone)]
pub struct PhysicalActionRef<T: ActionData = ()> {
    pub(crate) key: ActionKey,
    pub(crate) min_delay: Duration,
    pub(crate) values: Arc<Mutex<dyn BaseActionValues>>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: ActionData> ActionRefValue<T> for PhysicalActionRef<T> {
    fn get_value(&self, tag: Tag) -> Option<T> {
        self.values
            .lock()
            .expect("Failed to lock action values")
            .downcast_mut::<ActionValues<T>>()
            .expect("Type mismatch on ActionValues!")
            .get_value(tag)
            .cloned()
    }

    fn set_value(&mut self, value: Option<T>, new_tag: Tag) {
        self.values
            .lock()
            .expect("Failed to lock action values")
            .downcast_mut::<ActionValues<T>>()
            .expect("Type mismatch on ActionValues!")
            .set_value(value, new_tag)
    }

    fn get_min_delay(&self) -> Duration {
        self.min_delay
    }

    fn get_key(&self) -> ActionKey {
        self.key
    }
}

impl<T: ActionData> InnerType for PhysicalActionRef<T> {
    type Inner = T;
}

impl<'a, T: ActionData> From<&'a mut Action> for ActionRef<'a, T> {
    fn from(value: &'a mut Action) -> Self {
        value
            .as_valued_mut()
            .map(|logical| ActionRef {
                key: logical.key,
                min_delay: logical.min_delay,
                values: logical
                    .values
                    .downcast_mut()
                    .expect("Type mismatch on ActionValues!"),
            })
            .expect("Action is not valued")
    }
}

impl<'a, T: ActionData> From<&'a mut Action> for PhysicalActionRef<T> {
    fn from(value: &'a mut Action) -> Self {
        value
            .as_physical()
            .map(|physical| PhysicalActionRef {
                key: physical.key,
                min_delay: physical.min_delay,
                values: Arc::clone(&physical.values),
                _phantom: std::marker::PhantomData,
            })
            .expect("Action is not valued")
    }
}
