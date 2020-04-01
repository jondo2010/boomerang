//! Analyze the data supplied by a ReactorReceiver struct and generate the impl code
//! for the Reactor.

use crate::{
    parse::{ChildAttr, PortAttr, ReactionAttr, ReactorReceiver, TimerAttr},
    util::NamedField,
};

mod generate;
mod graph;
use graph::{GraphNode, NodeWithContext, TriggerNodeType};

use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    hash::Hash,
    iter::FromIterator,
    rc::Rc,
    time::Duration,
};

use darling::ToTokens;
use derive_more::Display;
use petgraph::graphmap::DiGraphMap;
use quote::format_ident;

#[derive(Debug, PartialEq, Eq, Hash, Display)]
#[display(fmt = "({})", "name.to_string()")]
struct PortBuilder {
    pub name: syn::Ident,
    pub ty: Option<syn::Type>,
}

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

#[derive(Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Display)]
#[display(fmt = "({})", "name.to_string()")]
struct TimerBuilder {
    pub name: syn::Ident,
    pub offset: Option<Duration>,
    pub period: Option<Duration>,
}

struct ActionBuilder {
    pub name: syn::Ident,
    pub min_delay: Duration,
    pub is_physical: bool,
}

#[derive(Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Display)]
#[display(fmt = "({})", attr)]
struct ReactionBuilder {
    attr: ReactionAttr,
    name: syn::Ident,
    index: usize,
    depends_on_timers: Vec<Rc<TimerBuilder>>,
    depends_on_inputs: Vec<Rc<PortBuilder>>,
    provides_outputs: Vec<Rc<PortBuilder>>,
    // provides_actions:
}

#[derive(Debug, Eq, PartialEq, Hash, Display)]
#[display(fmt = "{}", ident)]
struct ReactorStateBuilder {
    /// Ident for the Reactor state struct
    ident: syn::Ident,
    /// Generics information for the Reactor
    generics: syn::Generics,
}

impl PartialOrd for ReactorStateBuilder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.ident.partial_cmp(&other.ident)
    }
}

impl Ord for ReactorStateBuilder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ident.cmp(&other.ident)
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Display)]
#[display(fmt = "({}: <{}>)", name, "class.to_token_stream()")]
struct ChildBuilder {
    pub class: syn::Path,
    pub name: syn::Ident,
    pub inputs: Vec<(syn::Ident, Rc<PortBuilder>)>,
    pub outputs: Vec<(syn::Ident, Rc<PortBuilder>)>,
}

impl PartialOrd for ChildBuilder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.name.partial_cmp(&other.name)
    }
}

impl Ord for ChildBuilder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

#[derive(Debug)]
pub struct ReactorBuilderGen {
    state: Rc<ReactorStateBuilder>,
    children: HashMap<syn::Ident, Rc<ChildBuilder>>,
    timers: HashMap<syn::Ident, Rc<TimerBuilder>>,
    inputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    outputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    reactions: Vec<Rc<ReactionBuilder>>,
    connections: HashMap<Rc<PortBuilder>, Vec<Rc<PortBuilder>>>,
}

impl ReactorBuilderGen {
    /// Create the complete dependency graph needed to generate the reactor structures
    fn get_dependency_graph(&self) -> DiGraphMap<GraphNode, bool> {
        let reaction_edges = self.reactions.iter().flat_map(|reaction| {
            // All reactions depend on the reactor state
            let reactor_state =
                std::iter::once((GraphNode::Reaction(reaction), GraphNode::State(&self.state)));

            // Trigger output reactions from timers.
            let trigger_reactions_timers = reaction.depends_on_timers.iter().map(move |timer| {
                (
                    GraphNode::Trigger(TriggerNodeType::Timer(timer)),
                    GraphNode::Reaction(reaction),
                )
            });

            // Trigger output reactions from inputs
            let trigger_reactions_inputs = reaction.depends_on_inputs.iter().map(move |input| {
                (
                    GraphNode::Trigger(TriggerNodeType::Input(input)),
                    GraphNode::Reaction(reaction),
                )
            });

            // Reaction input ports
            let reaction_inputs = reaction
                .depends_on_inputs
                .iter()
                .map(move |input| (GraphNode::Reaction(reaction), GraphNode::Input(input)));

            // Reaction output ports
            let reaction_outputs = reaction
                .provides_outputs
                .iter()
                .map(move |output| (GraphNode::Reaction(reaction), GraphNode::Output(output)));

            // Reaction output triggers
            // Find any reactions who's `provides_outputs` contains `to`
            let reaction_output_triggers = reaction
                .provides_outputs
                .iter()
                .map(move |output: &Rc<PortBuilder>| {
                    self.connections.get(output).map(|input_vec| {
                        input_vec.iter().map(move |input| {
                            (
                                GraphNode::Reaction(reaction),
                                GraphNode::Trigger(TriggerNodeType::Input(input)),
                            )
                        })
                    })
                })
                .filter_map(|x| x)
                .flatten();

            reactor_state
                .chain(trigger_reactions_timers)
                .chain(trigger_reactions_inputs)
                .chain(reaction_inputs)
                .chain(reaction_outputs)
                .chain(reaction_output_triggers)
        });

        // Connections between ports
        let port_connections = self.connections.iter().flat_map(|(from, to_vec)| {
            to_vec
                .iter()
                .map(move |to| (GraphNode::Input(to), GraphNode::Output(from)))
        });

        DiGraphMap::from_edges(reaction_edges.chain(port_connections))
    }
}

/// Build a map of TimerBuilders from an iterable of TimerAttr
fn build_timers<I, M>(idents: &mut HashSet<syn::Ident>, timers: I) -> Result<M, darling::Error>
where
    I: IntoIterator<Item = TimerAttr>,
    M: FromIterator<(syn::Ident, Rc<TimerBuilder>)>,
{
    timers
        .into_iter()
        .map(|attr| {
            if idents.contains(&attr.name) {
                Err(darling::Error::duplicate_field(&attr.name.to_string()))
            } else {
                idents.insert(attr.name.clone());
                Ok((
                    attr.name.clone(),
                    Rc::new(TimerBuilder {
                        name: format_ident!("__timer_{}", &attr.name),
                        offset: attr.offset,
                        period: attr.period,
                    }),
                ))
            }
        })
        .collect()
}

fn build_ports<I, M>(idents: &mut HashSet<syn::Ident>, ports: I) -> Result<M, darling::Error>
where
    I: IntoIterator<Item = PortAttr>,
    M: FromIterator<(syn::Ident, Rc<PortBuilder>)>,
{
    ports
        .into_iter()
        .map(|attr| {
            if idents.contains(&attr.name) {
                Err(darling::Error::duplicate_field(&attr.name.to_string()))
            } else {
                idents.insert(attr.name.clone());
                Ok((
                    attr.name.clone(),
                    Rc::new(PortBuilder {
                        name: format_ident!("__self_{}", &attr.name),
                        ty: Some(attr.ty),
                    }),
                ))
            }
        })
        .collect::<Result<M, _>>()
}

fn build_children<I, M>(idents: &mut HashSet<syn::Ident>, children: I) -> Result<M, darling::Error>
where
    I: IntoIterator<Item = ChildAttr>,
    M: FromIterator<(syn::Ident, Rc<ChildBuilder>)>,
{
    // Children
    children
        .into_iter()
        .map(|attr| {
            if idents.contains(&attr.name) {
                Err(darling::Error::duplicate_field(&attr.name.to_string())
                    .with_span(&attr.name.span()))
            } else {
                idents.insert(attr.name.clone());
                let inputs = attr
                    .inputs
                    .iter()
                    .map(|i| {
                        (
                            i.clone(),
                            Rc::new(PortBuilder {
                                name: format_ident!("__{}_{}", &attr.name, i),
                                ty: None,
                            }),
                        )
                    })
                    .collect();
                let outputs = attr
                    .outputs
                    .iter()
                    .map(|i| {
                        (
                            i.clone(),
                            Rc::new(PortBuilder {
                                name: format_ident!("__{}_{}", &attr.name, i),
                                ty: None,
                            }),
                        )
                    })
                    .collect();
                Ok((
                    attr.name.clone(),
                    Rc::new(ChildBuilder {
                        class: attr.class,
                        name: format_ident!("__{}_reactor", &attr.name),
                        inputs: inputs,
                        outputs: outputs,
                    }),
                ))
            }
        })
        .collect()
}

impl TryFrom<ReactorReceiver> for ReactorBuilderGen {
    type Error = darling::Error;

    /// Create a ReactorBuilder from a parsed ReactorReceiver
    fn try_from(receiver: ReactorReceiver) -> Result<Self, Self::Error> {
        let mut idents = HashSet::<syn::Ident>::new();
        let children: HashMap<_, _> = build_children(&mut idents, receiver.children)?;
        let timers: HashMap<_, _> = build_timers(&mut idents, receiver.timers)?;
        let inputs: HashMap<_, _> = build_ports(&mut idents, receiver.inputs)?;
        let outputs: HashMap<_, _> = build_ports(&mut idents, receiver.outputs)?;

        let reactions = receiver
            .reactions
            .into_iter()
            .enumerate()
            .map(|(reaction_index, reaction)| {
                // Triggers can be timers, inputs, outputs of child reactors, or actions
                let (depends_on_timers, depends_on_inputs) = {
                    let mut depends_on_timers = vec![];
                    let mut depends_on_inputs = vec![];
                    for trigger in reaction.triggers.iter() {
                        let NamedField(base, member) = &trigger;
                        if base.to_string() == "self" {
                            if timers
                                .get(member)
                                .cloned()
                                .map(|timer| depends_on_timers.push(timer))
                                .is_none()
                            {
                                if inputs
                                    .get(member)
                                    .cloned()
                                    .map(|input| depends_on_inputs.push(input))
                                    .is_none()
                                {
                                    Err(darling::Error::unknown_field(&format!("{}", member)))?;
                                    // Err(darling::Error::unknown_field_with_alts( &format!("{}",
                                    // member), timers .keys() .map(|key| format!("{}", key))
                                    // .chain(inputs.keys().map(|key| key.to_string().as_str()))))?;
                                }
                            }
                        } else {
                            children
                                .get(base)
                                .ok_or(
                                    darling::Error::unknown_field( &format!("{}", trigger))
                                    //darling::Error::unknown_field_with_alts( &format!("{}", trigger), children.keys().map(|key| &key.to_string()),
                                )
                                .and_then(|child| {
                                    child
                                        .outputs
                                        .iter()
                                        .find(|(output, _)| output == member)
                                        .cloned()
                                        .ok_or(
                                            darling::Error::unknown_field(&format!("{}", trigger))
                                            //darling::Error::unknown_field_with_alts( &format!("{}", member), child .outputs .iter() .map(|output| &output.name.to_string()),)
                                    )
                                    .map(|(_, input)| depends_on_inputs.push(input))
                                })?;
                        }
                    }
                    (depends_on_timers, depends_on_inputs)
                };

                let outputs = reaction
                    .effects
                    .iter()
                    .map(|NamedField(ref base, ref member)| {
                        if base.to_string() == "self" {
                            outputs.get(member).cloned()
                        } else {
                            todo!("Handle this error");
                        }
                    })
                    .collect::<Vec<_>>();

                let reaction_ident = reaction
                    .function
                    .segments
                    .last()
                    .map(|seg| format_ident!("__{}_reaction", seg.ident))
                    .unwrap();

                Ok(Rc::new(ReactionBuilder {
                    attr: reaction,
                    name: reaction_ident,
                    index: reaction_index,
                    depends_on_timers: depends_on_timers,
                    depends_on_inputs: depends_on_inputs,
                    provides_outputs: outputs.into_iter().filter_map(|x| x).collect(),
                }))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let find_port = |NamedField(ref base, ref member)| {
            if base.to_string() == "self" {
                outputs
                    .get(member)
                    .or_else(|| inputs.get(member))
                    .cloned()
                    .ok_or_else(|| {
                        darling::Error::unknown_field(&format!("self.{}", &member))
                            .with_span(&member.span())
                    })
            } else {
                children
                    .get(base)
                    .ok_or(darling::Error::unknown_field(&base.to_string()))
                    .and_then(|child| {
                        child
                            .outputs
                            .iter()
                            .find(|(output, _)| output == member)
                            .or_else(|| child.inputs.iter().find(|(input, _)| input == member))
                            .map(|(_, port)| port)
                            .cloned()
                            .ok_or_else(|| {
                                darling::Error::unknown_field(&format!("{}.{}", &base, &member))
                                    .with_span(&member.span())
                            })
                    })
            }
        };

        // Connections
        let connection_pairs = receiver
            .connections
            .into_iter()
            .map(|attr| {
                let from_port = find_port(attr.from)?;
                let to_port = find_port(attr.to)?;
                Ok((from_port, to_port))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let connections =
            connection_pairs
                .into_iter()
                .fold(HashMap::new(), |mut acc, (key, val)| {
                    acc.entry(key).or_insert_with(Vec::new).push(val);
                    acc
                });

        let state = Rc::new(ReactorStateBuilder {
            ident: receiver.ident,
            generics: receiver.generics,
        });

        Ok(ReactorBuilderGen {
            state,
            children,
            timers: timers,
            inputs: inputs,
            outputs: outputs,
            reactions,
            connections,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{graph::ReactorBuildableNode, ReactorBuilderGen};
    use crate::parse::ReactorReceiver;
    use darling::FromDeriveInput;
    use petgraph::{
        dot::{Config, Dot},
        graphmap::DiGraphMap,
    };
    use std::convert::TryFrom;
    use std::{fs::File, io::prelude::*, path::Path};

    fn write_graphviz<P: AsRef<Path>>(graph: &DiGraphMap<ReactorBuildableNode, bool>, path: P) {
        let dot = Dot::with_config(&graph, &[Config::EdgeNoLabel]);
        let mut file = File::create(path).unwrap();
        write!(file, "{}", dot).unwrap();
    }

    fn topo_sorted_idents(graph: &DiGraphMap<ReactorBuildableNode, bool>) -> Vec<String> {
        petgraph::algo::toposort(graph, None)
            .unwrap()
            .iter()
            .rev()
            .map(|node| match *node {
                ReactorBuildableNode::Timer(timer) => timer.name.to_string(),
                ReactorBuildableNode::Port(port) => port.name.to_string(),
                ReactorBuildableNode::Child(child) => child.name.to_string(),
                ReactorBuildableNode::Reaction(reaction) => reaction.name.to_string(),
                ReactorBuildableNode::UnpackInputs => "unpack".to_owned(),
                ReactorBuildableNode::PackOutputs => "pack".to_owned(),
            })
            .collect::<Vec<_>>()
    }

    #[test]
    fn test_child() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    input(name="in", type="u32"),
    input(name="in2", type="u32"),
    output(name="out", type="u32"),
    output(name="out2", type="u32"),
    child(name="gain", class="Gain", inputs("in"), outputs("out")),
    connection(from="in", to="gain.in"),
    connection(from="gain.out", to="out"),
    connection(from="gain.out", to="out2"),
)]
pub struct GainContainer {}
"#,
        )
        .expect("Error parsing test");
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        let builder = ReactorBuilderGen::try_from(receiver).unwrap();
        let graph: DiGraphMap<_, _> = (&builder).into();
        let topo = topo_sorted_idents(&graph);
        assert_eq!(
            topo,
            vec![
                "unpack",
                "__self_in",
                "__gain_in",
                "__gain_reactor",
                "__gain_out",
                "__self_out",
                "__self_out2",
                "__self_in2",
                "pack"
            ]
        );
    }

    #[test]
    fn test_reaction() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    input(name="in", type="i32"),
    output(name="out", type="i32"),
    reaction(function="Gain::r0", triggers("in"), effects("out")),
)]
pub struct Gain {}
            "#,
        )
        .expect("Error parsing test");
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        let builder = ReactorBuilderGen::try_from(receiver).unwrap();
        let graph = DiGraphMap::<_, _>::from(&builder);
        let topo = topo_sorted_idents(&graph);
        // write_graphviz(&graph, "test.dot");
        assert_eq!(
            topo,
            vec!["__self_out", "unpack", "__self_in", "__r0_reaction", "pack",]
        );
    }

    #[test]
    fn test_source() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    output(name="out", type="i32"),
    timer(name="t"),
    reaction(function="Source::r0", triggers("t"), effects("out")),
)]
pub struct Source {}
            "#,
        )
        .expect("Error parsing test");
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        let builder = ReactorBuilderGen::try_from(receiver).unwrap();
        let graph = DiGraphMap::<_, _>::from(&builder);
        // let topo = topo_sorted_idents(&graph);
        write_graphviz(&graph, "test.dot");
    }

    #[test]
    fn test_gen_delay() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    input(name="in", type="T")
    output(name="out", type="T"),
    action(name="a0"),
    reaction(function="GenDelay::r0", triggers("act"), effects("out")),
    reaction(function="GenDelay::r1", triggers("in"), effects("a0")),
)]
pub struct GenDelay {}
            "#,
        )
        .expect("Error parsing test");
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        let builder = ReactorBuilderGen::try_from(receiver).unwrap();
        let graph = DiGraphMap::<_, _>::from(&builder);
        // let topo = topo_sorted_idents(&graph);
        write_graphviz(&graph, "test.dot");
    }

    #[test]
    fn test_delayed_reaction() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    child(class="Source", name="source", outputs("out")),
    child(class="Sink", name="sink", outputs("in")),
    connection(from="source.out", to="sink.in", after="100 msec"),
    connection(from="source.out", to="sink.in"),
    reaction(function="None", triggers("source.out"))
)]
pub struct DelayedReaction {}
"#,
        )
        .expect("Error parsing test");
        let receiver1 = ReactorReceiver::from_derive_input(&input).unwrap();
        let builder = ReactorBuilderGen::try_from(receiver1).unwrap();
        let _graph = DiGraphMap::<_, _>::from(&builder);
    }

    #[test]
    fn test2() {
        let input2 = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    child(class="Source", name="source", outputs("out")),
    child(class="Sink", name="sink", outputs("in")),
    child(class="GenDelay", name="__delay0", inputs("inp"), outputs("out")),
    action(name="action_delay", mit="1 msec"),
    reaction(function="DelayedReaction::r0", triggers("source.out"), uses(), effects()),
    reaction(function="DelayedReaction::r1", triggers("source.out"), uses(), effects()),
    connection(from="source.out", to="sink.in"),
)]
pub struct DelayedReaction {}
"#,
        )
        .expect("Error parsing test");
        let _receiver2 = ReactorReceiver::from_derive_input(&input2).unwrap();
    }
}
