use std::fmt::Debug;

use downcast_rs::{impl_downcast, DowncastSync};

use crate::{ActionKey, LevelReactionKey};

tinymap::key_type! { pub ReactorKey }

pub trait ReactorState: DowncastSync + Send {}
impl<T> ReactorState for T where T: DowncastSync {}
impl_downcast!(sync ReactorState);

pub struct Reactor {
    /// The reactor name
    pub(crate) name: String,
    /// The ReactorState
    pub(crate) state: Box<dyn ReactorState>,
    /// For each Action, a set of Reactions triggered by it.
    pub action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
}

impl Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reactor")
            .field("name", &self.name)
            .field("state", &"Box<dyn ReactorState>")
            .field("action_triggers", &self.action_triggers)
            .finish()
    }
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

    pub fn get_state<T: ReactorState>(&self) -> Option<&T> {
        self.state.downcast_ref()
    }

    pub fn get_state_mut<T: ReactorState>(&mut self) -> Option<&mut T> {
        self.state.downcast_mut()
    }
}
