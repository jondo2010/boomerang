use super::{
    ChildBuilder, PortBuilder, ReactionBuilder, ReactorBuilderGen, ReactorStateBuilder,
    TimerBuilder,
};
use derive_more::Display;
use petgraph::graphmap::DiGraphMap;
use std::rc::Rc;

#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash, Display)]
pub(super) enum ReactorBuildableNode<'a> {
    #[display(fmt = "Timer: {}", _0)]
    Timer(&'a Rc<TimerBuilder>),

    #[display(fmt = "Port: {}", _0)]
    Port(&'a Rc<PortBuilder>),

    #[display(fmt = "Child: {}", _0)]
    Child(&'a Rc<ChildBuilder>),

    #[display(fmt = "Reaction: {}", _0)]
    Reaction(&'a Rc<ReactionBuilder>),

    UnpackInputs,

    PackOutputs,
}

impl<'a> From<&'a ReactorBuilderGen> for DiGraphMap<ReactorBuildableNode<'a>, bool> {
    /// Create the complete dependency graph needed to generate the reactor structures
    fn from(builder: &'a ReactorBuilderGen) -> Self {
        let children_edges = builder
            .children
            .values()
            .flat_map(|child: &Rc<ChildBuilder>| {
                // Each child node depends on its' child input ports
                let input_edges = child.inputs.iter().map(move |(_, input)| {
                    (
                        ReactorBuildableNode::Child(child),
                        ReactorBuildableNode::Port(input),
                        true,
                    )
                });
                // Each child output port depends on its' child node
                let output_edges = child.outputs.iter().map(move |(_, output)| {
                    (
                        ReactorBuildableNode::Port(output),
                        ReactorBuildableNode::Child(child),
                        true,
                    )
                });
                input_edges.chain(output_edges)
            });

        let connection_edges = builder.connections.iter().flat_map(|(from, to)| {
            to.iter().map(move |to| {
                (
                    ReactorBuildableNode::Port(to),
                    ReactorBuildableNode::Port(from),
                    true,
                )
            })
        });

        let reaction_edges = builder.reactions.iter().flat_map(|reaction| {
            let input_edges = reaction.depends_on_inputs.iter().map(move |port| {
                (
                    ReactorBuildableNode::Reaction(reaction),
                    ReactorBuildableNode::Port(port),
                    true,
                )
            });
            let output_edges = reaction.provides_outputs.iter().map(move |port| {
                (
                    ReactorBuildableNode::Reaction(reaction),
                    ReactorBuildableNode::Port(port),
                    true,
                )
            });
            let timer_edges = reaction.depends_on_timers.iter().map(move |timer| {
                (
                    ReactorBuildableNode::Reaction(reaction),
                    ReactorBuildableNode::Timer(timer),
                    true,
                )
            });
            input_edges.chain(output_edges).chain(timer_edges)
        });

        // All input ports depend on the unpack node
        let unpack_edges = builder.inputs.values().map(|port| {
            (
                ReactorBuildableNode::Port(port),
                ReactorBuildableNode::UnpackInputs,
                true,
            )
        });

        // All output ports fan into the PackOutputs node
        let pack_edges = builder.outputs.values().map(|port| {
            (
                ReactorBuildableNode::PackOutputs,
                ReactorBuildableNode::Port(port),
                true,
            )
        });

        DiGraphMap::from_edges(
            children_edges
                .chain(connection_edges)
                .chain(reaction_edges)
                .chain(unpack_edges)
                .chain(pack_edges),
        )
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Display)]
pub(super) enum TriggerNodeType<'a> {
    #[display(fmt = "Timer: {}", _0.)]
    Timer(&'a Rc<TimerBuilder>),

    #[display(fmt = "Input: {}", _0.)]
    Input(&'a Rc<PortBuilder>),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Display)]
pub(super) enum GraphNode<'a> {
    #[display(fmt = "tri_{}", _0)]
    Trigger(TriggerNodeType<'a>),

    #[display(fmt = "inp_{}", _0)]
    Input(&'a Rc<PortBuilder>),

    #[display(fmt = "out_{}", _0)]
    Output(&'a Rc<PortBuilder>),

    #[display(fmt = "rea_{}", _0)]
    Reaction(&'a Rc<ReactionBuilder>),

    #[display(Fmt = "{}", _0)]
    State(&'a Rc<ReactorStateBuilder>),
}

pub(super) struct NodeWithContext<'a, G> {
    pub node: GraphNode<'a>,
    pub graph: G,
}

impl<'a> GraphNode<'a> {
    /// Create an ident for code generation
    pub(super) fn create_ident(&self) -> syn::Ident {
        use quote::format_ident;
        match *self {
            GraphNode::Trigger(TriggerNodeType::Input(input)) => {
                format_ident!("_trigger_{}", &input.name)
            }
            GraphNode::Trigger(TriggerNodeType::Timer(timer)) => {
                format_ident!("_trigger_{}", &timer.name)
            }
            GraphNode::Input(input) => format_ident!("_input_{}", &input.name),
            GraphNode::Output(output) => format_ident!("_output_{}", &output.name),
            GraphNode::Reaction(reaction) => format_ident!(
                "_reaction_{}",
                &reaction.attr.function.segments.last().unwrap().ident
            ),
            GraphNode::State(state) => {
                format_ident!("_state_{}", &state.ident.to_string().to_lowercase())
            }
        }
    }
}
