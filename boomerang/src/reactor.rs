use crate::Rc;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

use super::*;
use derive_more::Display;

pub trait Reactor {
    /// Invoke code that must execute before starting a new logical time round, such as initializing
    /// outputs to be absent.
    fn start_time_step(&self);

    fn start_timers(&self) {}
}

#[derive(Debug)]
struct ReactorBuilder {
    /// Name of the reactor
    pub name: String,

    /// State common to all reactions
    // state: Box<dyn ReactorState>,

    /// Contained child instances
    pub children: BTreeMap<String, Rc<ReactorBuilder>>,

    /// List of reaction instances for this reactor instance.
    pub reactions: Vec<Rc<ReactionBuilder>>,

    /// The trigger instances belonging to this reactor instance.
    pub triggers: BTreeSet<Rc<TriggerBuilder>>,
}

impl ReactorBuilder {
    pub fn new<T>(name: &str) -> Self
    where
        T: ReactorState + std::default::Default,
    {
        Self {
            name: name.to_owned(),
            // state: Box::new(T::default()),
            children: BTreeMap::new(),
            reactions: Vec::new(),
            triggers: BTreeSet::new(),
        }
    }

    pub fn with<F: Fn(&mut Self)>(mut self, f: F) -> Self {
        f(&mut self);
        self
    }

    /// Create a new Timer trigger with the given builder function.
    /// Returns None if an equal trigger already exists
    pub fn create_timer<F: Fn(TriggerBuilder) -> TriggerBuilder>(
        &mut self,
        name: &str,
        f: F,
    ) -> Option<Rc<TriggerBuilder>> {
        let trigger = f(TriggerBuilder {
            name: name.to_owned(),
            ..Default::default()
        });
        let trigger = Rc::new(trigger);
        if self.triggers.insert(trigger.clone()) {
            Some(trigger)
        } else {
            None
        }
    }

    pub fn add_reaction(&mut self, reaction: &Rc<ReactionBuilder>) {
        // If there is an earlier reaction in this reactor, then create a link in the dependence
        // graph.
        if let Some(previous_reaction) = self.reactions.last_mut() {
            previous_reaction
                .dependent_reactions
                .borrow_mut()
                .insert(reaction.clone());
            reaction
                .depends_on_reactions
                .borrow_mut()
                .insert(previous_reaction.clone());
        }
        self.reactions.push(reaction.clone());
    }

    /// Build an iterator over dependency edges
    fn get_dependency_edges(
        &self,
    ) -> impl Iterator<Item = (Rc<ReactionBuilder>, Rc<ReactionBuilder>, ())> + '_ {
        self.reactions.iter().flat_map(|r| {
            r.depends_on_reactions
                .borrow()
                .iter()
                .map(move |dependency| (r.clone(), dependency.clone(), ()))
                .chain(
                    r.dependent_reactions
                        .borrow()
                        .iter()
                        .map(move |dependent| (dependent.clone(), r.clone(), ())),
                )
                .collect::<Vec<_>>()
                .into_iter()
        })
    }

    pub fn analyze(&self) {
        // let g = self.get_dependency_graph();
        // let mut bfs = petgraph::visit::Bfs::new(g)
    }

    // fn get_dependency_graph(&self) -> DiGraphMap<&Rc<ReactionBuilder>, ()> {
    // DiGraphMap::<_, ()>::from_edges(
    // self.children
    // .iter()
    // .flat_map(|(_instance_name, child)| child.get_dependency_edges().map(|(a,b,c)| (&a,&b,c)))
    // .chain(self.get_dependency_edges()),
    // )
    // }

    fn add_child(&mut self, child: Rc<ReactorBuilder>, instance_name: &str) {
        self.children.insert(instance_name.to_owned(), child);
    }

    fn build(&self) {
        for reaction in self.reactions.iter() {
            println!("Reaction \"{}\"", reaction.name);

            for tr in reaction.triggers.borrow().iter() {
                println!("Trigger: {}", tr.name);
            }
        }

        let all_triggers = self
            .reactions
            .iter()
            .flat_map(|reaction: &Rc<ReactionBuilder>| {
                reaction
                    .triggers
                    .borrow()
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
            });

        for pp in all_triggers {
            println!("X: {}", pp.name);
        }

        for pp in self.triggers.iter() {
            println!("tr: {}", pp.name);
        }
    }
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TriggerBuilderSubtype {
    Timer,
    Action,
    Port(Rc<PortBuilder>),
}

impl std::default::Default for TriggerBuilderSubtype {
    fn default() -> Self {
        Self::Timer
    }
}

#[derive(Debug, Default, Eq, Ord, PartialEq, PartialOrd, Display)]
#[display(fmt = "TriggerBuilder<{}, {:?}, {:?}>", name, offset, period)]
struct TriggerBuilder {
    pub name: String,

    pub offset: Option<Duration>,

    pub period: Option<Duration>,

    pub subtype: TriggerBuilderSubtype,

    /// Reaction instances that are triggered by this trigger.
    pub dependent_reactions: RefCell<BTreeSet<Rc<ReactionBuilder>>>,

    /// Reaction instances that may send outputs via this port.
    pub depends_on_reactions: RefCell<BTreeSet<Rc<ReactionBuilder>>>,
}

impl TriggerBuilder {
    pub fn with_offset(mut self, offset: Duration) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn with_period(mut self, period: Duration) -> Self {
        self.period = Some(period);
        self
    }

    pub fn build<S>(self: Rc<Self>) -> Trigger<S>
    where
        S: Sched,
    {
        Trigger::new(
            vec![],
            Duration::from_secs(0),
            None,
            false,
            QueuingPolicy::NONE,
        )
    }
}

#[derive(Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ActionBuilder {}

struct PortBuilder {
    pub name: String,

    // pub is_present: Rc<RefCell<dyn IsPresent + 'static>>,
    /// Set of port instances that receive messages from this port.
    pub dependent_ports: BTreeSet<Rc<PortBuilder>>,

    /// Port that sends messages to this port, if there is one.
    pub depends_on_port: Option<Rc<PortBuilder>>,
}

impl std::fmt::Debug for PortBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortBuilder")
            //.field("is_present", &self.is_present.as_ptr())
            .field(
                "dependent_ports",
                &self.dependent_ports.iter().map(|port| &port.name),
            )
            .field(
                "depends_on_port",
                &self.depends_on_port.as_ref().map(|port| &port.name),
            )
            .finish()
    }
}

impl PartialEq for PortBuilder {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name //&& Rc::ptr_eq(&self.is_present, &other.is_present)
    }
}
impl Eq for PortBuilder {}

impl PartialOrd for PortBuilder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.name.partial_cmp(&other.name)
    }
}

impl Ord for PortBuilder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl PortBuilder {
    pub fn new(name: &str) -> Rc<Self> {
        // let port = Rc::new(RefCell::new(Port::<T>::default()));
        Rc::new(Self {
            name: name.to_owned(),
            // is_present: port.clone() as Rc<RefCell<dyn IsPresent>>,
            dependent_ports: Default::default(),
            depends_on_port: None,
        })
    }
}

#[derive(Debug, Default, Eq, PartialEq, Ord, PartialOrd, Display)]
#[display(fmt = "ReactionBuilder<{}>", name)]
struct ReactionBuilder {
    pub name: String,

    /// The actions that this reaction triggers.
    pub dependent_actions: BTreeSet<Rc<ActionBuilder>>,

    /// The ports that this reaction may write to.
    pub dependent_ports: BTreeSet<Rc<PortBuilder>>,

    /// The reactions that depend on this reaction.
    pub dependent_reactions: RefCell<BTreeSet<Rc<ReactionBuilder>>>,

    /// The actions that this reaction is triggered by.
    pub depends_on_actions: BTreeSet<Rc<ActionBuilder>>,

    /// The ports that this reaction is triggered by or uses.
    pub depends_on_ports: BTreeSet<Rc<PortBuilder>>,

    /// The timers that this reaction is triggered by.
    pub depends_on_timers: RefCell<BTreeSet<Rc<TriggerBuilder>>>,

    /// The reactions that this reaction depends on.
    pub depends_on_reactions: RefCell<BTreeSet<Rc<ReactionBuilder>>>,
    // Inferred deadline. Defaults to the maximum long value.
    // pub deadline = new TimeValue(TimeValue.MAX_LONG_DEADLINE, TimeUnit.NSEC)
    /// The triggers (input ports, timers, and actions that trigger reactions) that trigger this
    /// reaction
    pub triggers: RefCell<BTreeSet<Rc<TriggerBuilder>>>,
}

impl ReactionBuilder {
    pub fn new(name: &str) -> Rc<Self> {
        Rc::new(ReactionBuilder {
            name: name.to_owned(),
            ..Default::default()
        })
    }

    pub fn with_trigger(self: Rc<Self>, trigger: &Rc<TriggerBuilder>) -> Rc<Self> {
        self.triggers.borrow_mut().insert(trigger.clone());
        trigger
            .dependent_reactions
            .borrow_mut()
            .insert(self.clone());
        self
    }

    pub fn with_effects<I>(self: Rc<Self>, effects: I) -> Rc<Self>
    where
        I: IntoIterator,
        // I::Item: Rc<PortBuilder>,
    {
        // self.dependent_ports
        self
    }

    pub fn with_closure<State, S, F>(self: Rc<Self>, closure: F) -> Rc<Self>
    where
        State: ReactorState + std::default::Default,
        S: Sched,
        F: FnMut(&mut State, &mut S),
    {
        let state = Box::new(State::default());

        self
    }

    // pub fn with_reaction<S: Sched>(
    // mut self,
    // triggers: Vec<&str>,
    // uses: Vec<&str>,
    // effects: Vec<&str>,
    // closure: dyn FnMut(&mut State, &mut S),
    // &mut dyn React<State = SourceState>, sched: &mut dyn Sched<Value = ()>| {},
    // /
    // ) -> Self {
    // let x = triggers.iter().map(|trigger_name| {
    // self.triggers
    // .iter()
    // .find(|trigger| &trigger.name == trigger_name)
    // });
    // let r = ReactionBuilder {};
    // self
    // }
}

trait ReactorState: std::any::Any + std::fmt::Debug {
    // Get a mutable reference to the innter state.
    // fn get_state_mut<State>(&mut self) -> &mut State;

    fn new(inputs: &[&dyn std::any::Any], outputs: &[&dyn std::any::Any]) -> Self;
}

type MySched = Scheduler<()>;
#[derive(Debug, Default)]
struct SourceState {
    pub count: u32,
    pub x: Rc<RefCell<Port<u32>>>,
    pub y: Rc<RefCell<Port<u32>>>,
}

impl SourceState {
    pub fn poo(self: &mut Self, _sched: &mut MySched) {
        self.y.borrow_mut().set(self.count);
        println!("Hello poo: {}", self.count);
    }

    pub fn fart(self: &mut Self, _sched: &mut MySched) {
        println!("Hello fart: {}", self.x.borrow().get());
    }
}

impl ReactorState for SourceState {
    fn new(inputs: &[&dyn std::any::Any], _outputs: &[&dyn std::any::Any]) -> Self {
        let x = inputs[0].downcast_ref::<Rc<RefCell<Port<u32>>>>().unwrap();
        let y = inputs[1].downcast_ref::<Rc<RefCell<Port<u32>>>>().unwrap();
        Self {
            x: x.clone(),
            y: y.clone(),
            ..Default::default()
        }
    }
}

#[test]
fn test_reactor_state() {
    let o1 = Rc::new(RefCell::new(Port::<u32>::new(0)));
    let o2 = Rc::new(RefCell::new(Port::<u32>::new(1)));

    let y: Vec<&dyn std::any::Any> = vec![&o1, &o2];

    let r = SourceState::new(y.as_slice(), y.as_slice());
    dbg!(r);
}

#[test]
fn test() {
    let source = ReactorBuilder::new::<SourceState>("Source").with(|reactor| {
        let t = reactor
            .create_timer("t", |timer| {
                timer
                    .with_offset(Duration::from_secs(1))
                    .with_period(Duration::from_secs(2))
            })
            .unwrap();

        //let (y, y_port) = PortBuilder::new::<u32>("y");

        let r1 = ReactionBuilder::new("r1")
            .with_trigger(&t)
            //.with_uses()
            //.with_effects(&[y])
            .with_closure(SourceState::poo);
        // .with_closure(|state: &mut SourceState, outputs: (&RefCell<Port<u32>>,), _sched: &mut
        // MySched| { let (y,) = outputs;
        // state.count += 1;
        // y.borrow_mut().set(state.count);
        // println!("Hello World: {}", state.count);
        // });

        reactor.add_reaction(&r1);
    });

    source.build();

    // let mut composition = ReactorBuilder::new::<u32>("composition");
    // composition.add_child(&mut source, "s");

    // let mut source = ReactorBuilder::new::<SourceState>("Source")
    // .with_timer(
    // "t",
    // Some(Duration::from_secs(1)),
    // Some(Duration::from_secs(2)),
    // )
    // .with_output::<u32>("y")
    // .with_reaction(
    // vec!["t"],
    // vec![],
    // vec!["y"],
    // |react: &mut dyn ReactorState<State = SourceState>,
    // sched: &mut dyn Sched<Value = ()>| {},
    // );
    //
    // let mut test = ReactorBuilder::new("Test")
    // .with_input<u32>("x")
    // .with_reaction(vec!["x"], vec![], vec![]);
    //
    // let composition = ReactorBuilder::new("composition")
    // .with_child(&mut source, "s")
    // .with_child(&mut test, "d")
    // .with_connection("s.y", "d.x")
    // ;
}
