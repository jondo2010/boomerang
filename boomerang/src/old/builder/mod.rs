// mod action;
mod port;
mod reaction;
mod reactor;

use crate::{Error, ErrorKind};
// use action::*;
use port::*;
use reaction::*;
use reactor::*;
use std::{
    cell::RefCell,
    fmt::{Debug, Display},
    hash::Hash,
};

/// Basic trait for Builder types
pub trait NamedBuilder<'a> {
    /// Get the name of this Builder
    fn get_name(&self) -> &str;
    /// Get the parent Builder
    // fn get_parent(&self) -> &'a dyn NamedBuilder<'a>;
    /// Get the fully-qualified-name
    fn get_fqn(&self) -> String;
}

impl<'a> Debug for &'a dyn NamedBuilder<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamedBuilder")
            .field("name", &self.get_name())
            .field("fqn", &self.get_fqn())
            .finish()
    }
}

impl<'a> Display for &'a dyn NamedBuilder<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("NamedBuilder({})", self.get_fqn()))
    }
}

impl<'a> PartialEq for &'a dyn NamedBuilder<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.get_fqn().eq(&other.get_fqn())
    }
}

impl<'a> Hash for &'a dyn NamedBuilder<'a> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get_fqn().hash(state);
    }
}

// pub struct EnvironmentBuilder<'a> {
// arena: &'a bumpalo::Bump,
// arena: &'a toolshed::Arena,
// reactors: toolshed::set::Set<'a, &'a ReactorBuilder<'a>>,
// reactors: typed_arena::Arena<ReactorBuilder<'a>>,
// reactions: typed_arena::Arena<ReactionBuilder<'a>>,
// actions: typed_arena::Arena<ActionBuilder<'a>>,
// ports: typed_arena::Arena<PortBuilder<'a>>, */
// }

// impl<'a> EnvironmentBuilder<'a> {
// pub fn new(arena: &'a toolshed::Arena) -> Self {
// Self {
// reactors: toolshed::set::Set::new(),
// }
// }
// pub fn new_reactor(&self, name: &str, arena: &toolshed::Arena) -> &'a ReactorBuilder<'a> {
// let reactor = arena.alloc(ReactorBuilder::new(name));
// self.reactors.insert(reactor);
// reactor
// }
// pub fn dependency_graph(&self) {}
// }
//
// impl<'a> Drop for EnvironmentBuilder<'a>{
// fn drop(&mut self) {
// println!("EnvironmentBuilder::drop()");
// }
// }

/// Trait used by generated code to create `ReactorBuilder`s at runtime.
/// The inputs and outputs of reactors are fixed at compile-time.
// pub trait ReactorBuildable {
// type Inputs: Default;
// type Outputs: Default;
//
// Create a new ReactorBuilder for this Reactor
// fn create<'a>(inputs: Self::Inputs) -> (ReactorBuilder<'a>, Self::Outputs);
//
// Create a top-level ReactorBuilder for this Reactor, with any inputs/outputs left
// disconnected.
// fn create_main<'a>() -> ReactorBuilder<'a> {
// let inputs = Self::Inputs::default();
// let (reactor_builder, _outputs) = Self::create(inputs);
// reactor_builder
// }
// }

#[cfg(test)]
mod test {
    use super::*;
    use crate::runtime::{self, PortData, PortType};
    use std::{
        any::Any,
        collections::BTreeMap,
        sync::{Arc, RwLock},
    };

    #[test]
    fn test0() {
        let arena = toolshed::Arena::new();
        let master = ReactorBuilder::<u32>::new(&arena, "master");

        let source_out0 = {
            let source = master.new_child(&arena, "source");
            let out0 = source.new_port(&arena, "out0", PortType::Output);

            let r0_body_builder = arena.alloc(
                move |builder_state: &BuilderStateHelper<u32>| -> runtime::ReactionFn {
                    let out0_value = builder_state.get_port(out0);
                    runtime::ReactionFn::new(|scheduler| {
                        *out0_value.write().unwrap() = Some(1);
                    })
                },
            );

            let r0 = source.new_reaction(&arena, "r0", 0, r0_body_builder);
            r0.declare_antidependency(&arena, out0);
            // r0.declare_trigger_action(&mut env, &startup);
            out0
        };

        let sink_in0 = {
            let sink = master.new_child(&arena, "sink");
            let in0 = sink.new_port(&arena, "in0", PortType::Input);

            let r0_body_builder: &ReactionBodyBuilderFn<u32> = arena.alloc(
                move |builder_state: &BuilderStateHelper<_>| -> runtime::ReactionFn {
                    let in0_value = builder_state.get_port(in0);
                    runtime::ReactionFn::new(|scheduler| {})
                },
            );
            let r0 = sink.new_reaction(&arena, "r0", 0, r0_body_builder);
            r0.declare_trigger_port(&arena, in0);
            in0
        };

        source_out0.bind_to(&arena, sink_in0);

        let mut port_map = BTreeMap::new();
        let mut reaction_map = BTreeMap::new();
        let x = sink_in0.build(&mut port_map, &mut reaction_map);
        // dbg!(&port_list);
        dbg!(&x);

        // println!("{}", in0.get_inward_binding().unwrap());

        for edge in master.reaction_dependency_graph() {
            println!("({}, {})", edge.0, edge.1);
        }

        let graph =
            petgraph::graphmap::DiGraphMap::<_, ()>::from_edges(master.reaction_dependency_graph());
        let mut space = petgraph::algo::DfsSpace::new(&graph);
        let sort = petgraph::algo::toposort(&graph, Some(&mut space)).unwrap();

        for n in sort.iter() {
            println!("{}", n);
        }

        // let res = petgraph::algo::dijkstra(&graph, sort.first().unwrap(), None, |_| 1);
        // for x in res.iter() {
        // println!("{}: {}", x.0, x.1);
        // }
    }

    // #[test]
    // fn test() {
    // let mut env = EnvironmentBuilder::new();
    // let source = {
    // let reactor = ReactorNode::new(&mut env, "source");
    // let startup = ActionNode::new_startup_action(&mut env, "startup", &reactor);
    // let shutdown = ActionNode::new_shutdown_action(&mut env, "shutdown", &reactor);
    // let r0 = ReactionNode::new(&mut env, "r0", &reactor);
    // let out0 = PortNode::<u32>::new(&mut env, "out", &r0, PortType::Output);
    // r0.declare_trigger_action(&mut env, &startup);
    // r0.declare_antidependency(&mut env, &out0);
    // (reactor, out0)
    // };
    //
    // let sink = {
    // let reactor = ReactorNode::new(&mut env, "sink");
    // let startup = ActionNode::new_startup_action(&mut env, "startup", &reactor);
    // let shutdown = ActionNode::new_shutdown_action(&mut env, "shutdown", &reactor);
    // let r0 = ReactionNode::new(&mut env, "r0", &reactor);
    // let in0 = PortNode::<u32>::new(&mut env, "out", &r0, PortType::Output);
    // (reactor, in0)
    // };
    // source.1.bind_to(&mut env, &sink.1);
    // dbg!(&env);
    // }
}
