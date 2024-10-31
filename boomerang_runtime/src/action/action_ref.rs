use crate::{Context, Duration, Tag};

use super::{Action, ActionKey, BaseAction, ReactorData};

/*: From<&'a mut dyn BaseAction> */
pub trait ActionCommon<T: ReactorData> {
    fn name(&self) -> &str;
    fn key(&self) -> ActionKey;
    fn min_delay(&self) -> Duration;
    fn is_logical(&self) -> bool;
}

/// [`ActionRef`] is the type received by a user Reaction when they want to interact with an Action. It is the
/// synchronous version of [`AsyncActionRef`].
pub struct ActionRef<'a, T: ReactorData = ()>(&'a mut Action<T>);

impl<'a, T: ReactorData> From<&'a mut dyn BaseAction> for ActionRef<'a, T> {
    fn from(value: &'a mut dyn BaseAction) -> Self {
        Self(value.downcast_mut().expect("Type mismatch on ActionRefMut"))
    }
}

impl<'a, T: ReactorData> ActionRef<'a, T> {
    /// Return true if the action is present at the current tag
    pub fn is_present(&mut self, context: &Context) -> bool {
        self.0.store.get_current(context.tag).is_some()
    }

    /// Get the current value for this action at the tag
    pub fn get_value_at(&mut self, tag: Tag) -> Option<&T> {
        self.0.store.get_current(tag)
    }

    /// Set the value for this action at the tag
    pub fn set_value(&mut self, tag: Tag, value: T) {
        self.0.store.push(tag, value);
    }
}

impl<'a, T: ReactorData> ActionCommon<T> for ActionRef<'a, T> {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn key(&self) -> ActionKey {
        self.0.key()
    }

    fn min_delay(&self) -> Duration {
        self.0.min_delay.unwrap_or_default()
    }

    fn is_logical(&self) -> bool {
        self.0.is_logical()
    }
}

/// [`AsyncActionRef`] is the type received by a user Reaction when they want to interact with an Action asynchronously.
/// It is the asynchronous version of [`ActionRefMut`].
#[derive(Clone)]
pub struct AsyncActionRef<T: ReactorData = ()> {
    name: String,
    key: ActionKey,
    min_delay: Option<Duration>,
    is_logical: bool,
    _phantom: std::marker::PhantomData<fn() -> T>,
}

impl<'a, T: ReactorData> From<&'a dyn BaseAction> for AsyncActionRef<T> {
    fn from(value: &'a dyn BaseAction) -> Self {
        let action: &Action<T> = value.downcast_ref().expect("Type mismatch on ActionRef2");
        Self {
            name: action.name().into(),
            key: action.key(),
            min_delay: action.min_delay(),
            is_logical: action.is_logical(),
            _phantom: Default::default(),
        }
    }
}

//NOTE: The following is implemented to satisty PartitionMut in the generated code
impl<'a, T: ReactorData> From<&'a mut dyn BaseAction> for AsyncActionRef<T> {
    fn from(value: &'a mut dyn BaseAction) -> Self {
        (&*value).into()
    }
}

impl<T: ReactorData> ActionCommon<T> for AsyncActionRef<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn key(&self) -> ActionKey {
        self.key
    }

    fn min_delay(&self) -> Duration {
        self.min_delay.unwrap_or_default()
    }

    fn is_logical(&self) -> bool {
        self.is_logical
    }
}
