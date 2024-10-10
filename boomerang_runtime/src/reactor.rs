use std::fmt::{Debug, Display};

use crate::ReactorState;

tinymap::key_type! { pub ReactorKey }

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Reactor {
    /// The reactor name
    name: String,
    /// The ReactorState
    pub(crate) state: Box<dyn ReactorState>,
}

impl Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("Reactor");
        debug_struct.field("name", &self.name);
        debug_struct.field("state", &"Box<dyn ReactorState>");
        debug_struct.finish()
    }
}

impl Display for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "runtime::Reactor::new(\"{name}\", Box::new({ty}))",
            name = self.name,
            ty = (*self.state).type_name()
        )
    }
}

impl Reactor {
    pub fn new(name: &str, state: Box<dyn ReactorState>) -> Self {
        Self {
            name: name.to_owned(),
            state,
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
