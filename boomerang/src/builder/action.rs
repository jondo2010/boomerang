use super::runtime;
use slotmap::SecondaryMap;

#[derive(Debug)]
pub enum ActionType {
    Timer {
        period: runtime::Duration,
        offset: runtime::Duration,
    },
    Logical {
        min_delay: Option<runtime::Duration>,
    },
    Shutdown,
}

#[derive(Debug)]
pub struct ActionBuilder {
    /// Name of the Action
    name: String,
    /// Logical type of the action
    ty: ActionType,
    /// The key of this action in the EnvBuilder
    action_key: runtime::ActionKey,
    /// The parent Reactor that owns this Action
    reactor_key: runtime::ReactorKey,
    /// Out-going Reactions that this action triggers
    pub triggers: SecondaryMap<runtime::ReactionKey, ()>,
    /// List of Reactions that may schedule this action
    pub schedulers: SecondaryMap<runtime::ReactionKey, ()>,
}

impl ActionBuilder {
    pub fn new(
        name: &str,
        ty: ActionType,
        action_key: runtime::ActionKey,
        reactor_key: runtime::ReactorKey,
    ) -> Self {
        Self {
            name: name.to_owned(),
            ty,
            action_key,
            reactor_key,
            triggers: SecondaryMap::new(),
            schedulers: SecondaryMap::new(),
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_type(&self) -> &ActionType {
        &self.ty
    }

    pub fn get_action_key(&self) -> runtime::ActionKey {
        self.action_key
    }

    pub fn get_reactor_key(&self) -> runtime::ReactorKey {
        self.reactor_key
    }

    // pub fn build(&self) -> Arc<dyn runtime::BaseAction> {
    // event!(
    // tracing::Level::DEBUG,
    // "Building Action: {}, triggers: {:?}",
    // self.name,
    // self.triggers
    // );
    //
    // match self.inner {
    // ActionBuilderInner::Timer { offset, period } => {
    // Arc::new(runtime::Timer::new(&self.name, self.action_key, offset, period))
    // }
    // ActionBuilderInner::StartupAction => {
    // Arc::new(runtime::Timer::new_zero(&self.name, self.action_key))
    // }
    // ActionBuilderInner::ShutdownAction => unimplemented!(),
    // ActionBuilderInner::Action { logical, min_delay } => {
    // Arc::new(runtime::Action::new(&self.name, logical, min_delay))
    // }
    // }
    // }
}
