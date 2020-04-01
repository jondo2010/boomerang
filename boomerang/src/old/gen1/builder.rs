use crate::{Duration, Rc, RefCell, Sched};
use std::collections::HashMap;

// #[derive(Debug)]
// pub struct ReactorBuilder {
// state: Rc<ReactorStateBuilder>,
// children: HashMap<syn::Ident, Rc<ChildBuilder>>,
// timers: HashMap<syn::Ident, Rc<TimerBuilder>>,
// inputs: HashMap<syn::Ident, Rc<PortBuilder>>,
// outputs: HashMap<syn::Ident, Rc<PortBuilder>>,
// reactions: Vec<Rc<ReactionBuilder>>,
// connections: HashMap<Rc<PortBuilder>, Vec<Rc<PortBuilder>>>,
// }

#[derive(Clone)]
pub struct TimerBuilder {
    pub offset: Option<Duration>,
    pub period: Option<Duration>,
}

pub struct InputBuilder {}

pub struct OutputBuilder {}

pub struct ReactionBuilder<S> {
    pub reaction: Box<RefCell<dyn FnMut(&mut S)>>,
    pub depends_on_timers: Vec<Rc<TimerBuilder>>,
    pub depends_on_inputs: Vec<Rc<InputBuilder>>,
    pub provides_outputs: Vec<Rc<OutputBuilder>>,
}

#[derive(Default)]
pub struct ReactorBuilder<S> {
    pub timers: HashMap<String, Rc<TimerBuilder>>,
    pub inputs: HashMap<String, Rc<InputBuilder>>,
    pub outputs: HashMap<String, Rc<OutputBuilder>>,
    pub children: HashMap<String, Rc<ReactorBuilder<S>>>,
    pub reactions: Vec<ReactionBuilder<S>>,
}
