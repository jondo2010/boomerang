use std::fmt::{Debug, Display};

use downcast_rs::{impl_downcast, Downcast};

use crate::ReactorData;

tinymap::key_type! { pub ReactorKey }

pub trait BaseReactor: Debug + Downcast + Send + Sync {
    /// Get the name of the reactor
    fn name(&self) -> &str;
}

impl_downcast!(BaseReactor);

impl dyn BaseReactor {
    pub fn get_state<T: ReactorData>(&self) -> Option<&T> {
        self.downcast_ref::<Reactor<T>>().map(|r| &r.state)
    }
}

pub struct Reactor<T: ReactorData> {
    /// The reactor name
    name: String,
    /// The ReactorState
    pub(crate) state: T,
}

impl<T: ReactorData> Debug for Reactor<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reactor")
            .field("name", &self.name)
            .field("state", &std::any::type_name::<T>())
            .finish()
    }
}

impl<T: ReactorData> Display for Reactor<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Reactor<{ty}>(\"{name}\")",
            name = self.name,
            ty = std::any::type_name::<T>()
        )
    }
}

impl<T: ReactorData> Reactor<T> {
    pub fn new(name: &str, state: T) -> Self {
        Self {
            name: name.to_owned(),
            state,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn boxed(self) -> Box<dyn BaseReactor> {
        Box::new(self)
    }
}

impl<T: ReactorData> BaseReactor for Reactor<T> {
    fn name(&self) -> &str {
        &self.name
    }
}
