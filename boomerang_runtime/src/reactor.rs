use std::fmt::Debug;

use downcast_rs::{impl_downcast, Downcast};

tinymap::key_type! { pub ReactorKey }

#[cfg(feature = "parallel")]
pub trait ReactorState: Downcast + Send + Sync {}

#[cfg(feature = "parallel")]
impl<T> ReactorState for T where T: Downcast + Send + Sync {}

#[cfg(not(feature = "parallel"))]
pub trait ReactorState: Downcast {}

#[cfg(not(feature = "parallel"))]
impl<T> ReactorState for T where T: Downcast {}

impl_downcast!(ReactorState);

pub struct Reactor {
    /// The reactor name
    pub(crate) name: String,
    /// The ReactorState
    pub(crate) state: Box<dyn ReactorState>,
}

impl Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reactor")
            .field("name", &self.name)
            .field("state", &"Box<dyn ReactorState>")
            .finish()
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
