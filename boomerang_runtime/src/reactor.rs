use std::fmt::Debug;

use downcast_rs::{impl_downcast, DowncastSync};

use crate::{ActionKey, Context, LevelReactionKey, ReactionSet, ScheduledEvent, Tag};

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
    /// For each Action, a set of Reactions triggered by it.
    pub action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
}

impl Reactor {
    pub fn new(
        name: &str,
        state: Box<dyn ReactorState>,
        action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            state,
            action_triggers,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_state<T: ReactorState>(&mut self) -> Option<&mut T> {
        self.state.downcast_mut()
    }
}
