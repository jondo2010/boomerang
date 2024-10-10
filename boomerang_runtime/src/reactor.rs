use std::fmt::{Debug, Display};

use downcast_rs::{impl_downcast, Downcast};

use crate::{data::ParallelData, ReactorData};

tinymap::key_type! { pub ReactorKey }

pub trait BaseReactor: Debug + ParallelData + Downcast {
    /// Get the name of the reactor
    fn name(&self) -> &str;
}

impl_downcast!(BaseReactor);

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Reactor<T: ReactorData> {
    /// The reactor name
    name: String,
    /// The ReactorState
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) state: T,
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
