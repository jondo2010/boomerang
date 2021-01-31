use crate::runtime::{self};
use slotmap::SecondaryMap;
use std::sync::Arc;
use tracing::event;

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
    action_key: runtime::BaseActionKey,
    /// The ReactorType that contains this ActionBuilder
    reactor_key: runtime::ReactorKey,
    /// Out-going Reactions that this action triggers
    pub triggers: SecondaryMap<runtime::ReactionKey, ()>,
    /// TODO?
    pub schedulers: SecondaryMap<runtime::ReactionKey, ()>,
    inner: ActionBuilderInner,
}

impl ActionBuilder {
    /// Create a new Timer Action
    ///     On startup() - schedule the action with possible offset
    ///     On cleanup() - reschedule if the duration is non-zero
    pub fn new_timer_action(
        name: &str,
        action_key: runtime::BaseActionKey,
        reactor_key: runtime::ReactorKey,
        offset: runtime::Duration,
        period: runtime::Duration,
    ) -> Self {
        Self {
            name: name.into(),
            action_key,
            reactor_key,
            triggers: SecondaryMap::new(),
            schedulers: SecondaryMap::new(),
            inner: ActionBuilderInner::Timer { offset, period },
        }
    }

    pub fn get_action_key(&self) -> runtime::BaseActionKey {
        self.action_key
    }

    pub fn get_reactor_key(&self) -> runtime::ReactorKey {
        self.reactor_key
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
                self.action_key,
                offset,
                period,
                self.triggers.clone(),
            ),
            ActionBuilderInner::StartupAction => {
                runtime::Timer::new_zero(&self.name, self.action_key, self.triggers.clone())
            }
            ActionBuilderInner::ShutdownAction => unimplemented!(),
        })
    }
}
