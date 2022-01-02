use std::fmt::Debug;

use downcast_rs::{impl_downcast, DowncastSync};

use crate::{ActionKey, Context, DepInfo, ReactionSet, ScheduledEvent, Tag};

slotmap::new_key_type! {
    pub struct ReactorKey;
}

pub trait ReactorState: Send + Sync + DowncastSync {}
impl<T> ReactorState for T where T: Send + Sync + DowncastSync {}
impl_downcast!(sync ReactorState);

pub(crate) trait ReactorElement {
    fn startup(&self, _ctx: &mut Context, _key: ActionKey) {}
    fn shutdown(&self, dep_info: &DepInfo, reaction_set: &mut ReactionSet) {}
    fn cleanup(&self, dep_info: &DepInfo, current_tag: Tag) -> Option<ScheduledEvent> {
        None
    }
}
