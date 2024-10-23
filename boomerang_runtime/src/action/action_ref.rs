use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{Action, ActionKey, ActionStore, BaseActionStore};
use crate::{ReactorData, Tag};

pub trait ActionRefValue<T: ReactorData> {
    /// Access the current Action value at the given Tag using a closure `f`.
    fn get_value_with<F: FnOnce(Option<&T>) -> U, U>(&mut self, tag: Tag, f: F) -> U;
    /// Set the Action value at the given Tag.
    fn set_value(&mut self, value: Option<T>, new_tag: Tag);
    fn get_min_delay(&self) -> Duration;
    fn get_key(&self) -> ActionKey;
}

/// An `ActionRef` is a reference to an `Action` that can be scheduled.
pub struct ActionRef<'a, T: ReactorData = ()> {
    pub(crate) name: &'a str,
    pub(crate) key: ActionKey,
    pub(crate) min_delay: Duration,
    pub(crate) store: &'a mut ActionStore<T>,
}

impl ActionRef<'_, ()> {
    pub fn name(&self) -> &str {
        self.name
    }

    pub fn key(&self) -> ActionKey {
        self.key
    }
}

impl<'a, T: ReactorData> ActionRefValue<T> for ActionRef<'a, T> {
    fn get_value_with<F: FnOnce(Option<&T>) -> U, U>(&mut self, tag: Tag, f: F) -> U {
        let value = self.store.get_current(tag);
        f(value)
    }

    fn set_value(&mut self, value: Option<T>, new_tag: Tag) {
        self.store.push(new_tag, value);
    }

    fn get_min_delay(&self) -> Duration {
        self.min_delay
    }

    fn get_key(&self) -> ActionKey {
        self.key
    }
}

#[derive(Debug)]
pub struct PhysicalActionRef<T: ReactorData = ()> {
    pub(crate) key: ActionKey,
    pub(crate) min_delay: Duration,
    pub(crate) values: Arc<Mutex<dyn BaseActionStore>>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: ReactorData> Clone for PhysicalActionRef<T> {
    fn clone(&self) -> Self {
        Self {
            key: self.key,
            min_delay: self.min_delay,
            values: self.values.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T: ReactorData> ActionRefValue<T> for PhysicalActionRef<T> {
    fn get_value_with<F: FnOnce(Option<&T>) -> U, U>(&mut self, tag: Tag, f: F) -> U {
        let mut store = self.values.lock().expect("Failed to lock action store");
        let store = store
            .downcast_mut::<ActionStore<T>>()
            .expect("Type mismatch on ActionValues!");
        let value = store.get_current(tag);
        f(value)
    }

    fn set_value(&mut self, value: Option<T>, new_tag: Tag) {
        let mut store = self.values.lock().expect("Failed to lock action store");
        let store = store
            .downcast_mut::<ActionStore<T>>()
            .expect("Type mismatch on ActionValues!");
        store.push(new_tag, value);
    }

    fn get_min_delay(&self) -> Duration {
        self.min_delay
    }

    fn get_key(&self) -> ActionKey {
        self.key
    }
}

impl<'a, T: ReactorData> From<&'a mut Action> for ActionRef<'a, T> {
    fn from(value: &'a mut Action) -> Self {
        value
            .as_logical_mut()
            .map(|logical| ActionRef {
                name: logical.name.as_str(),
                key: logical.key,
                min_delay: logical.min_delay,
                store: logical
                    .store
                    .downcast_mut()
                    .expect("Type mismatch on ActionValues!"),
            })
            .expect("Action is not valued")
    }
}

impl<'a, T: ReactorData> From<&'a mut Action> for PhysicalActionRef<T> {
    fn from(value: &'a mut Action) -> Self {
        value
            .as_physical()
            .map(|physical| PhysicalActionRef {
                key: physical.key,
                min_delay: physical.min_delay,
                values: Arc::clone(&physical.store),
                _phantom: std::marker::PhantomData,
            })
            .expect("Action is not valued")
    }
}
