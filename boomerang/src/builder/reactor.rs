use super::{BuilderError, EnvBuilder, ReactionBuilderState};
use crate::runtime;
use slotmap::SecondaryMap;
use std::sync::{Arc, RwLock};

/// ReactorPart
pub trait ReactorPart: Send + Sync + Clone {}
impl<T> ReactorPart for T where T: Send + Sync + Clone {}

pub trait Reactor<S: runtime::SchedulerPoint>: Send + Sync {
    /// Type containing the Reactors input Ports
    type Inputs: ReactorPart = EmptyPart;
    /// Type containing the Reactors output Ports
    type Outputs: ReactorPart = EmptyPart;
    /// Type containing the Reactors Actions
    type Actions: ReactorPart = EmptyPart;

    /// Build the Reactors' Inputs/Outputs/Actions
    fn build_parts<'b>(
        &'b self,
        env: &'b mut EnvBuilder<S>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError>;
    /// Build a new Reactor with the given instance name
    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder<S>,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError>;
}

#[derive(Clone, Default)]
pub struct EmptyPart;

// ------------------------- //

/// Reactor Prototype
#[derive(Debug)]
pub(super) struct ReactorBuilder {
    /// The instantiated/child name of the Reactor
    pub name: String,
    /// The top-level/class name of the Reactor
    pub type_name: String,
    /// Optional parent reactor key
    pub parent_reactor_key: Option<runtime::ReactorKey>,
    /// Reactions in this ReactorType
    pub reactions: SecondaryMap<runtime::ReactionKey, ()>,
    pub ports: SecondaryMap<runtime::BasePortKey, ()>,
}

impl ReactorBuilder {
    fn new(name: &str, type_name: &str, parent_reactor_key: Option<runtime::ReactorKey>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            parent_reactor_key,
            reactions: SecondaryMap::new(),
            ports: SecondaryMap::new(),
        }
    }
}

/// Builder struct used to facilitate construction of a ReactorType
#[derive(Debug)]
pub struct ReactorBuilderState<'a, S, R>
where
    S: runtime::SchedulerPoint,
    R: Reactor<S>,
{
    reactor_key: runtime::ReactorKey,
    reactor: Arc<RwLock<R>>,
    pub inputs: R::Inputs,
    pub outputs: R::Outputs,
    pub actions: R::Actions,
    env: &'a mut EnvBuilder<S>,
}

impl<'a, S, R> ReactorBuilderState<'a, S, R>
where
    S: runtime::SchedulerPoint,
    R: Reactor<S>,
{
    pub(super) fn new(
        name: &str,
        parent: Option<runtime::ReactorKey>,
        reactor: R,
        env: &'a mut EnvBuilder<S>,
    ) -> Self {
        let reactor_key = env.reactors.insert(ReactorBuilder::new(
            name,
            std::any::type_name::<R>(),
            parent,
        ));

        let (inputs, outputs, actions) = reactor.build_parts(env, reactor_key).unwrap();

        Self {
            reactor_key,
            reactor: Arc::new(RwLock::new(reactor)),
            inputs,
            outputs,
            actions,
            env,
        }
    }

    pub fn add_startup_action(
        &mut self,
        name: &str,
    ) -> Result<runtime::BaseActionKey, BuilderError> {
        self.add_timer(
            name,
            runtime::Duration::from_micros(0),
            runtime::Duration::from_micros(0),
        )
    }

    pub fn add_shutdown_action(
        &mut self,
        _name: &str,
    ) -> Result<runtime::BaseActionKey, BuilderError> {
        todo!()
    }

    pub fn add_timer(
        &mut self,
        name: &str,
        period: runtime::Duration,
        offset: runtime::Duration,
    ) -> Result<runtime::BaseActionKey, BuilderError> {
        self.env.add_timer(name, period, offset, self.reactor_key)
    }

    pub fn add_reaction<F>(&mut self, reaction_fn: F) -> ReactionBuilderState<S>
    where
        F: for<'c> Fn(&'c mut R, &'c S, &'c R::Inputs, &'c R::Outputs, &'c R::Actions)
            + Send
            + Sync
            + 'static,
        R: 'static,
        R::Inputs: 'static,
        R::Outputs: 'static,
        R::Actions: 'static,
    {
        // Priority = number of reactions declared thus far + 1
        let priority = self.env.reactors[self.reactor_key].reactions.len();
        let reactor = self.reactor.clone();
        let inputs = self.inputs.clone();
        let outputs = self.outputs.clone();
        let actions = self.actions.clone();

        ReactionBuilderState::new(
            std::any::type_name::<F>(),
            priority,
            self.reactor_key,
            move |sched: &S| {
                let mut reactor = reactor.write().unwrap();
                reaction_fn(&mut *reactor, sched, &inputs, &outputs, &actions);
            },
            self.env,
        )
    }

    pub fn finish(self) -> Result<(runtime::ReactorKey, R::Inputs, R::Outputs), BuilderError> {
        Ok((self.reactor_key, self.inputs, self.outputs))
    }
}
