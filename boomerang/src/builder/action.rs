use tracing::event;

use super::ReactorTypeIndex;
use crate::runtime::{self, ActionIndex};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Debug)]
enum ActionBuilderInner {
    Timer {
        offset: runtime::Duration,
        period: runtime::Duration,
    },
    StartupAction,
    ShutdownAction,
}

#[derive(Debug)]
pub struct ActionBuilder {
    name: String,
    /// The index of this action
    action_idx: ActionIndex,
    /// The ReactorType that contains this ActionBuilder
    reactor_type_idx: ReactorTypeIndex,
    /// Out-going Reactions that this action triggers
    pub triggers: BTreeSet<runtime::ReactionIndex>,
    /// TODO?
    pub schedulers: Vec<runtime::ReactionIndex>,
    inner: ActionBuilderInner,
}

impl ActionBuilder {
    /// Create a new Timer Action
    ///     On startup() - schedule the action with possible offset
    ///     On cleanup() - reschedule if the duration is non-zero
    pub fn new_timer_action(
        name: &str,
        action_idx: ActionIndex,
        reactor_type_idx: ReactorTypeIndex,
        offset: runtime::Duration,
        period: runtime::Duration,
    ) -> Self {
        Self {
            name: name.into(),
            action_idx,
            reactor_type_idx,
            triggers: BTreeSet::new(),
            schedulers: Vec::new(),
            inner: ActionBuilderInner::Timer { offset, period },
        }
    }

    pub fn get_action_idx(&self) -> ActionIndex {
        self.action_idx
    }

    pub fn get_reactor_type_idx(&self) -> ReactorTypeIndex {
        self.reactor_type_idx
    }

    pub fn build(&self) -> Arc<dyn runtime::BaseAction> {
        event!(
            tracing::Level::DEBUG,
            "Building Action: {}, triggers: {:?}",
            self.name,
            self.triggers
        );

        Arc::new(match self.inner {
            ActionBuilderInner::Timer { offset, period } => runtime::Timer::new(
                &self.name,
                self.action_idx,
                offset,
                period,
                self.triggers.clone(),
            ),
            ActionBuilderInner::StartupAction => {
                runtime::Timer::new_zero(&self.name, self.action_idx, self.triggers.clone())
            }
            ActionBuilderInner::ShutdownAction => unimplemented!(),
        })
    }
}
