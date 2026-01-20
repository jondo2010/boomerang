use crate::{Context, Duration, Tag};

use super::{Action, ActionKey, BaseAction, ReactorData};
use crate::refs_extract::ReactionRefsError;

/// Wrapper for dynamic mutable action references to support fallible conversions.
pub struct DynActionRefMut<'a>(pub &'a mut dyn BaseAction);

/// Wrapper for dynamic immutable action references to support fallible conversions.
pub struct DynActionRef<'a>(pub &'a dyn BaseAction);

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

impl<'a, T: ReactorData> TryFrom<DynActionRefMut<'a>> for ActionRef<'a, T> {
    type Error = ReactionRefsError;

    fn try_from(value: DynActionRefMut<'a>) -> Result<Self, Self::Error> {
        let found = value.0.type_name();

        value
            .0
            .downcast_mut::<Action<T>>()
            .map(ActionRef)
            .ok_or_else(|| ReactionRefsError::type_mismatch("action", std::any::type_name::<T>(), found))
    }
}

impl<T: ReactorData> ActionRef<'_, T> {
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

    /// Determine the next tag for the given base tag, advancing the microstep
    /// to avoid overwriting values scheduled at the same offset.
    pub fn next_tag_for_offset(&mut self, base: Tag) -> Tag {
        let offset = base.offset();
        let microstep = self
            .0
            .store
            .next_microstep_for_offset(offset, base.microstep());

        Tag::new(offset, microstep)
    }

    /// Convert this [`ActionRef`] to an [`AsyncActionRef`]
    pub fn to_async(self) -> AsyncActionRef<T> {
        AsyncActionRef::try_from(DynActionRef(self.0 as &dyn BaseAction))
            .expect("Type mismatch on ActionRef2")
    }
}

impl<T: ReactorData> ActionCommon<T> for ActionRef<'_, T> {
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
/// It is the asynchronous version of [`ActionRef`].
#[derive(Clone)]
pub struct AsyncActionRef<T: ReactorData = ()> {
    name: String,
    key: ActionKey,
    min_delay: Option<Duration>,
    is_logical: bool,
    _phantom: std::marker::PhantomData<fn() -> T>,
}

impl<'a, T: ReactorData> TryFrom<DynActionRef<'a>> for AsyncActionRef<T> {
    type Error = ReactionRefsError;

    fn try_from(value: DynActionRef<'a>) -> Result<Self, Self::Error> {
        let found = value.0.type_name();

        value
            .0
            .downcast_ref::<Action<T>>()
            .map(|action| Self {
                name: action.name().into(),
                key: action.key(),
                min_delay: action.min_delay(),
                is_logical: action.is_logical(),
                _phantom: Default::default(),
            })
            .ok_or_else(|| ReactionRefsError::type_mismatch("action", std::any::type_name::<T>(), found))
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
