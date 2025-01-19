use crate::{event::AsyncEvent, Context, Duration, SendContext, Tag};

use super::{Action, ActionCommon, ActionKey, BaseAction, ReactorData};

/// [`ActionRef`] is the type received by a user Reaction when they want to interact with an Action. It is the
/// synchronous version of [`AsyncActionRef`].
pub struct ActionRef<'a, T: ReactorData = ()>(&'a mut Action<T>);

impl<'a, T: ReactorData> From<&'a mut dyn BaseAction> for ActionRef<'a, T> {
    fn from(value: &'a mut dyn BaseAction) -> Self {
        Self(value.downcast_mut().expect("Type mismatch on ActionRef2"))
    }
}

impl<'a, T: ReactorData> ActionRef<'a, T> {
    /// Return true if the action is present at the current tag
    pub fn is_present(&mut self, context: &Context) -> bool {
        self.0.store.get_current(context.tag).is_some()
    }

    /// Get the current value for this action
    pub fn get_value(&mut self, context: &Context) -> Option<&T> {
        self.0.store.get_current(context.tag)
    }

    /// Schedule a new value for this action
    pub fn schedule(&mut self, context: &mut Context, value: T, delay: Option<Duration>) {
        let action = &mut self.0;

        let tag_delay = action.min_delay.unwrap_or_default() + delay.unwrap_or_default();

        let new_tag = if action.is_logical {
            // Logical actions are scheduled at the current logical time + tag_delay
            context.tag.delay(tag_delay)
        } else {
            // Physical actions are scheduled at the current physical time + tag_delay
            Tag::from_physical_time(context.start_time, std::time::Instant::now()).delay(tag_delay)
        };

        // Push the new value into the store
        action.store.push(new_tag, value);

        // Schedule the action to trigger at the new tag
        context
            .trigger_res
            .scheduled_actions
            .push((action.key, new_tag));
    }
}

impl<'a, T: ReactorData> ActionCommon for ActionRef<'a, T> {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn key(&self) -> ActionKey {
        self.0.key()
    }

    fn min_delay(&self) -> Duration {
        self.0.min_delay.unwrap_or_default()
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

impl<'a, T: ReactorData> From<&'a mut dyn BaseAction> for AsyncActionRef<T> {
    fn from(value: &'a mut dyn BaseAction) -> Self {
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

impl<T: ReactorData> AsyncActionRef<T> {
    /// Schedule a new value for this action
    pub fn schedule(&self, context: &SendContext, value: T, delay: Option<Duration>) {
        let tag_delay = self.min_delay.unwrap_or_default() + delay.unwrap_or_default();
        let value = Box::new(value) as Box<dyn ReactorData>;

        let event = if self.is_logical {
            // Logical actions are scheduled at the current logical time + tag_delay
            tracing::info!(tag_delay = ?tag_delay, key = ?self.key, "Scheduling Async LogicalAction");
            AsyncEvent::logical(self.key, tag_delay, value)
        } else {
            // Physical actions are scheduled at the current physical time + tag_delay
            let new_tag = Tag::from_physical_time(context.start_time, std::time::Instant::now())
                .delay(tag_delay);
            tracing::info!(new_tag = %new_tag, key = ?self.key, "Scheduling Async PhysicalAction");
            AsyncEvent::physical(self.key, new_tag, value)
        };

        context
            .async_tx
            .send(event)
            .expect("Failed to send async event");
    }
}

impl<T: ReactorData> ActionCommon for AsyncActionRef<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn key(&self) -> ActionKey {
        self.key
    }

    fn min_delay(&self) -> Duration {
        self.min_delay.unwrap_or_default()
    }
}
