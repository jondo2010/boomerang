use super::{runtime, Reactor, ReactorBuilderState};
use std::time::Duration;

#[derive(Debug)]
pub struct TimerArgs {
    pub offset: Option<Duration>,
    pub period: Option<Duration>,
}

#[derive(Debug, Default)]
pub enum ActionAttrPolicy {
    #[default]
    Defer,
    Drop,
}

#[derive(Debug)]
pub struct ActionArgs {
    pub physical: bool,
    pub min_delay: Option<Duration>,
    pub mit: Option<Duration>,
    pub policy: Option<ActionAttrPolicy>,
}

#[derive(Debug)]
pub struct ChildArgs<S: runtime::ReactorState> {
    /// An expression resulting in a Reactor
    pub state: S,
}

/// Attribute on a Reaction field in a ReactorBuilder struct
pub struct ReactionArgs<'a, R: Reactor>
{
    pub function: Box<dyn FnOnce(&str, &R, &mut ReactorBuilderState<R::State>) + 'static>,
}