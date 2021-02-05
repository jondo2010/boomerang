use super::{BuilderError, EnvBuilder, ReactionBuilderState};
use crate::runtime::{self};
use slotmap::SecondaryMap;
use std::sync::{Arc, RwLock};

pub trait Reactor: Send + Sync + 'static {
    /// Type containing the Reactor's input Ports
    type Inputs;
    /// Type containing the Reactor's output Ports
    type Outputs;
    /// Type containing the Reactor's Actions
    type Actions;
    /// Build a new Reactor with the given instance name
    fn build(
        self,
        name: &str,
        env: &mut EnvBuilder,
        parent: Option<runtime::ReactorKey>,
    ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError>;
}

pub trait ReactorPart: Send + Sync + Clone + 'static {
    fn build(env: &mut EnvBuilder, reactor_key: runtime::ReactorKey) -> Result<Self, BuilderError>;
}

#[derive(Clone)]
pub struct EmptyPart;
impl ReactorPart for EmptyPart {
    fn build(_: &mut EnvBuilder, _: runtime::ReactorKey) -> Result<Self, BuilderError> {
        Ok(Self)
    }
}

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
pub struct ReactorBuilderState<'a, R: Reactor> {
    reactor_key: runtime::ReactorKey,
    reactor: Arc<RwLock<R>>,
    pub inputs: R::Inputs,
    pub outputs: R::Outputs,
    pub actions: R::Actions,
    env: &'a mut EnvBuilder,
}

impl<'a, R> ReactorBuilderState<'a, R>
where
    R: Reactor,
    R::Inputs: ReactorPart,
    R::Outputs: ReactorPart,
    R::Actions: ReactorPart,
{
    pub(super) fn new(
        name: &str,
        parent: Option<runtime::ReactorKey>,
        reactor: R,
        env: &'a mut EnvBuilder,
    ) -> Self {
        let reactor_key = env.reactors.insert(ReactorBuilder::new(
            name,
            std::any::type_name::<R>(),
            parent,
        ));

        let inputs = R::Inputs::build(env, reactor_key).unwrap();
        let outputs = R::Outputs::build(env, reactor_key).unwrap();
        let actions = R::Actions::build(env, reactor_key).unwrap();

        Self {
            reactor_key,
            reactor: Arc::new(RwLock::new(reactor)),
            inputs,
            outputs,
            actions,
            env,
        }
    }

    pub fn add_startup_timer(
        &mut self,
        name: &str,
    ) -> Result<runtime::BaseActionKey, BuilderError> {
        self.add_timer(
            name,
            runtime::Duration::from_micros(0),
            runtime::Duration::from_micros(0),
        )
    }

    pub fn add_timer(
        &mut self,
        name: &str,
        period: runtime::Duration,
        offset: runtime::Duration,
    ) -> Result<runtime::BaseActionKey, BuilderError> {
        self.env.add_timer(name, period, offset, self.reactor_key)
    }

    pub fn add_reaction<F>(&mut self, reaction_fn: F) -> ReactionBuilderState
    where
        F: Fn(&mut R, &runtime::SchedulerPoint, &R::Inputs, &R::Outputs, &R::Actions)
            + Send
            + Sync
            + 'static,
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
            runtime::ReactionFn::new(move |sched| {
                let mut reactor = reactor.write().unwrap();
                reaction_fn(&mut *reactor, sched, &inputs, &outputs, &actions);
            }),
            self.env,
        )
    }

    pub fn finish(self) -> Result<(runtime::ReactorKey, R::Inputs, R::Outputs), BuilderError> {
        Ok((self.reactor_key, self.inputs, self.outputs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::tests::*;

    #[test]
    fn test_add_input() {
        let mut env_builder = EnvBuilder::new();
        let _builder_state =
            ReactorBuilderState::new("test_reactor", None, TestReactor2, &mut env_builder);
    }
}
