use std::fmt::Debug;

use downcast_rs::{impl_downcast, DowncastSync};

use crate::{
    Action, ActionKey, Context, LevelReactionKey, LogicalAction, ReactionSet, ScheduledEvent, Tag,
};

tinymap::key_type! { pub ReactorKey }

pub trait ReactorState: DowncastSync + Send {}
impl<T> ReactorState for T where T: DowncastSync {}
impl_downcast!(sync ReactorState);

pub(crate) trait ReactorElement {
    fn startup(&self, _ctx: &mut Context, _key: ActionKey) {}
    fn shutdown(&self, _reaction_set: &mut ReactionSet) {}
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
    pub actions: tinymap::TinyMap<ActionKey, Action>,
    /// For each Action, a set of Reactions triggered by it.
    pub action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
}

impl Reactor {
    pub fn new(
        name: &str,
        state: Box<dyn ReactorState>,
        actions: tinymap::TinyMap<ActionKey, Action>,
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

    /// Return an `Iterator` of reactions sensitive to `Startup` actions.
    pub fn iter_startup_events(&self) -> impl Iterator<Item = &[LevelReactionKey]> {
        self.actions.iter().filter_map(|(action_key, action)| {
            if let Action::Startup = action {
                Some(self.action_triggers[action_key].as_slice())
            } else {
                None
            }
        })
    }

    pub fn iter_shutdown_events(&self) -> impl Iterator<Item = &[LevelReactionKey]> {
        self.actions.iter().filter_map(|(action_key, action)| {
            if let Action::Shutdown { .. } = action {
                Some(self.action_triggers[action_key].as_slice())
            } else {
                None
            }
        })
    }

    pub fn cleanup(&mut self, current_tag: Tag) {
        for action in self.actions.values_mut() {
            if let Action::Logical(LogicalAction { values, .. }) = action {
                // Clear action values at the current tag
                values.remove(current_tag);
            }
        }
    }
}
