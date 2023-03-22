use std::fmt::Debug;

use downcast_rs::{impl_downcast, DowncastSync};

use crate::{
    ActionKey, Context, Duration, InternalAction, LevelReactionKey, ReactionSet, ScheduledEvent,
    Tag, ValuedAction,
};

tinymap::key_type! { pub ReactorKey }

pub trait ReactorState: DowncastSync {}
impl<T> ReactorState for T where T: DowncastSync {}
impl_downcast!(sync ReactorState);

pub(crate) trait ReactorElement {
    fn startup(&self, _ctx: &mut Context, _key: ActionKey) {}
    fn shutdown(&self, _reaction_sett: &mut ReactionSet) {}
    fn cleanup(&self, _current_tag: Tag) -> Option<ScheduledEvent> {
        None
    }
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Reactor {
    /// The reactor name
    pub(crate) name: String,
    /// The ReactorState
    #[derivative(Debug = "ignore")]
    pub(crate) state: Box<dyn ReactorState>,
    /// Map of Actions for this Reactor
    pub(crate) actions: tinymap::TinyMap<ActionKey, InternalAction>,
    /// For each Action, a set of Reactions triggered by it.
    pub(crate) action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
}

impl Reactor {
    pub fn new(
        name: &str,
        state: Box<dyn ReactorState>,
        actions: tinymap::TinyMap<ActionKey, InternalAction>,
        action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            state,
            actions,
            action_triggers,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_state<T: ReactorState>(&mut self) -> Option<&mut T> {
        self.state.downcast_mut()
    }

    /// Return an `Iterator` of startup-triggered reactions and their timing offset.
    pub fn iter_startup_events(&self) -> impl Iterator<Item = (&Duration, &[LevelReactionKey])> {
        self.actions.iter().filter_map(|(action_key, action)| {
            if let InternalAction::Timer { offset, .. } = action {
                Some((offset, self.action_triggers[action_key].as_slice()))
            } else {
                None
            }
        })
    }

    pub fn iter_cleanup_events(&self) -> impl Iterator<Item = (&Duration, &[LevelReactionKey])> {
        self.actions.iter().filter_map(|(action_key, action)| {
            match action {
                InternalAction::Timer { period, .. } if !period.is_zero() => {
                    // schedule a periodic timer again
                    Some((period, self.action_triggers[action_key].as_slice()))
                }
                _ => None,
            }
        })
    }

    pub fn iter_shutdown_events(&self) -> impl Iterator<Item = &[LevelReactionKey]> {
        self.actions.iter().filter_map(|(action_key, action)| {
            if let InternalAction::Shutdown { .. } = action {
                Some(self.action_triggers[action_key].as_slice())
            } else {
                None
            }
        })
    }

    pub fn cleanup(&mut self, current_tag: Tag) {
        for action in self.actions.values_mut() {
            if let InternalAction::Valued(ValuedAction { values, .. }) = action {
                // Clear action values at the current tag
                values.remove(current_tag);
            }
        }
    }
}
