//! Analyze the data supplied by a ReactorReceiver struct and generate the impl code
//! for the Reactor.

use crate::parse::{PortAttr, ReactionAttr, ReactorReceiver, TimerAttr};

use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    hash::Hash,
    iter::FromIterator,
    rc::Rc,
};

use derive_more::Display;
use petgraph::graphmap::DiGraphMap;

#[derive(Debug, PartialEq, Eq, Ord, PartialOrd, Display, Clone, Hash)]
pub enum PortBuilderType {
    #[display(fmt = "[I]")]
    Input,
    #[display(fmt = "[O]")]
    Output,
}

#[derive(Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Display)]
#[display(fmt = "PortBuilder: '{}' {}", "attr.name.to_string()", subtype)]
pub struct PortBuilder {
    attr: PortAttr,
    subtype: PortBuilderType,
}

#[derive(Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Display)]
pub struct TimerBuilder {
    attr: TimerAttr,
}

#[derive(Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Display)]
#[display(fmt = "{}", attr)]
pub struct ReactionBuilder {
    attr: ReactionAttr,
    depends_on_timers: Vec<Rc<TimerBuilder>>,
    depends_on_inputs: Vec<Rc<PortBuilder>>,

    provides_outputs: Vec<Rc<PortBuilder>>,
    // provides_actions:
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct ReactorBuilder {
    idents: HashSet<syn::Ident>,
    timers: HashMap<syn::Ident, Rc<TimerBuilder>>,
    inputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    outputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    reactions: Vec<Rc<ReactionBuilder>>,
    connections: HashSet<(Rc<PortBuilder>, Rc<PortBuilder>)>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Display)]
pub enum GraphNode<'a> {
    #[display(fmt = "Timer: {}", _0.)]
    Timer(&'a Rc<TimerBuilder>),
    Input(&'a Rc<PortBuilder>),
    Output(&'a Rc<PortBuilder>),
    Reaction(&'a Rc<ReactionBuilder>),
}

impl<'a> From<&'a Rc<TimerBuilder>> for GraphNode<'a> {
    fn from(tim: &'a Rc<TimerBuilder>) -> Self {
        GraphNode::Timer(tim)
    }
}

impl<'a> From<&'a Rc<PortBuilder>> for GraphNode<'a> {
    fn from(port: &'a Rc<PortBuilder>) -> Self {
        match port.subtype {
            PortBuilderType::Input => GraphNode::Input(port),
            PortBuilderType::Output => GraphNode::Output(port),
        }
    }
}

impl<'a> From<&'a Rc<ReactionBuilder>> for GraphNode<'a> {
    fn from(reac: &'a Rc<ReactionBuilder>) -> Self {
        GraphNode::Reaction(reac)
    }
}

impl ReactorBuilder {
    pub fn get_dependency_graph(&self) -> DiGraphMap<GraphNode, bool> {
        let iter =
            self.reactions.iter().flat_map(|reaction| {
                reaction
                    .depends_on_timers
                    .iter()
                    .map(move |timer| (GraphNode::from(reaction), GraphNode::from(timer), false))
                    .chain(reaction.depends_on_inputs.iter().map(move |input| {
                        (GraphNode::from(reaction), GraphNode::from(input), false)
                    }))
                    .chain(reaction.provides_outputs.iter().map(move |output| {
                        (GraphNode::from(output), GraphNode::from(reaction), false)
                    }))
                    .chain(
                        self.connections
                            .iter()
                            .map(|(from, to)| (GraphNode::from(to), GraphNode::from(from), false)),
                    )
            });

        DiGraphMap::from_edges(iter)
    }
}

/// Extract the expected (base, member) ident tuple, or Err if the ExprField doesn't match.
fn expr_field_parts(expr: &syn::ExprField) -> Result<(&syn::Ident, &syn::Ident), darling::Error> {
    match *expr.base {
        syn::Expr::Path(syn::ExprPath { ref path, .. }) => path.get_ident(),
        _ => None,
    }
    .and_then(|base| match &expr.member {
        syn::Member::Named(member) => Some((base, member)),
        _ => None,
    })
    .ok_or(darling::Error::custom("Unexpected expression for effect"))
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
                Ok((attr.name.clone(), Rc::new(TimerBuilder { attr: attr })))
            }
        })
        .collect::<Result<M, _>>()
}

fn build_ports<I, M>(
    idents: &mut HashSet<syn::Ident>,
    ports: I,
    subtype: PortBuilderType,
) -> Result<M, darling::Error>
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
                        attr: attr,
                        subtype: subtype.clone(),
                    }),
                ))
            }
        })
        .collect::<Result<M, _>>()
}

impl TryFrom<ReactorReceiver> for ReactorBuilder {
    type Error = darling::Error;
    fn try_from(receiver: ReactorReceiver) -> Result<Self, Self::Error> {
        let mut idents = HashSet::<syn::Ident>::new();

        let all_timers: HashMap<_, _> = build_timers(&mut idents, receiver.timers)?;
        let all_inputs: HashMap<_, _> =
            build_ports(&mut idents, receiver.inputs, PortBuilderType::Input)?;
        let all_outputs: HashMap<_, _> =
            build_ports(&mut idents, receiver.outputs, PortBuilderType::Output)?;

        // Children
        for child in receiver.children.into_iter() {
            todo!();
        }

        // Reactions
        let reactions = receiver
            .reactions
            .into_iter()
            .map(|reaction| {
                // Triggers can be timers, inputs, outputs of child reactors, or actions
                let triggers = reaction
                    .triggers
                    .iter()
                    .map(|trigger| {
                        expr_field_parts(trigger)
                            .map(|(base, member)| {
                                let trigger_name = format!("{}.{}", base, member);
                                if base.to_string() == "self" {
                                    (
                                        trigger_name,
                                        all_timers.get(member).cloned(),
                                        all_inputs.get(member).cloned(),
                                    )
                                } else {
                                    todo!("Outputs of child reactors");
                                    (trigger_name, None, None)
                                }
                            })
                            .and_then(|(field_name, timer, input)| match (&timer, &input) {
                                (None, None) => Err(darling::Error::unknown_field(&field_name)),
                                _ => Ok((timer, input)),
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let (timers, inputs): (Vec<_>, Vec<_>) = triggers.into_iter().unzip();

                let outputs = reaction
                    .effects
                    .iter()
                    .map(|effect| {
                        expr_field_parts(effect).map(|(base, member)| {
                            if base.to_string() == "self" {
                                all_outputs.get(member).cloned()
                            } else {
                                todo!("Handle this error");
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(Rc::new(ReactionBuilder {
                    attr: reaction,
                    depends_on_timers: timers.into_iter().filter_map(|x| x).collect(),
                    depends_on_inputs: inputs.into_iter().filter_map(|x| x).collect(),
                    provides_outputs: outputs.into_iter().filter_map(|x| x).collect(),
                }))
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Connections
        let connections = receiver
            .connections
            .into_iter()
            .map(|attr| {
                let out_port = expr_field_parts(&attr.from).and_then(|(base, member)| {
                    if base.to_string() == "self" {
                        all_outputs
                            .get(member)
                            .cloned()
                            .ok_or_else(|| darling::Error::unknown_field(&member.to_string()))
                    } else {
                        todo!("Handle child")
                    }
                })?;

                let in_port = expr_field_parts(&attr.to).and_then(|(base, member)| {
                    if base.to_string() == "self" {
                        all_inputs
                            .get(member)
                            .cloned()
                            .ok_or_else(|| darling::Error::unknown_field(&member.to_string()))
                    } else {
                        todo!("Handle child")
                    }
                })?;

                Ok((out_port, in_port))
            })
            .collect::<Result<HashSet<_>, _>>()?;

        Ok(ReactorBuilder {
            idents,
            timers: all_timers,
            inputs: all_inputs,
            outputs: all_outputs,
            reactions,
            connections,
        })
    }
}
