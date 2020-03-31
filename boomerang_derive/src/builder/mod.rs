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

mod graph;
pub use graph::{GraphNode, NodeWithContext, TriggerNodeType};

use darling::ToTokens;
use derive_more::Display;
use petgraph::graphmap::DiGraphMap;
use quote::format_ident;

#[derive(Debug, PartialEq, Eq, Ord, PartialOrd, Display, Clone, Hash)]
pub enum PortBuilderType {
    #[display(fmt = "I")]
    Input,
    #[display(fmt = "O")]
    Output,
}

#[derive(Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Display)]
#[display(fmt = "{}", "attr.name.to_string()")]
pub struct PortBuilder {
    attr: PortAttr,
    subtype: PortBuilderType,
}

#[derive(Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Display)]
#[display(fmt = "{}", "attr.name.to_string()")]
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

#[derive(Debug, Eq, PartialEq, Hash, Display)]
#[display(fmt = "{}", ident)]
pub struct ReactorStateBuilder {
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

#[derive(Debug)]
pub struct ReactorBuilder {
    state: Rc<ReactorStateBuilder>,
    /// Set of idents used in the macro attributes
    idents: HashSet<syn::Ident>,
    timers: HashMap<syn::Ident, Rc<TimerBuilder>>,
    inputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    outputs: HashMap<syn::Ident, Rc<PortBuilder>>,
    reactions: Vec<Rc<ReactionBuilder>>,
    connections: HashMap<Rc<PortBuilder>, Vec<Rc<PortBuilder>>>,
}

impl ReactorBuilder {
    /// Create the complete dependency graph needed to generate the reactor structures
    pub fn get_dependency_graph(&self) -> DiGraphMap<GraphNode, bool> {
        let reaction_edges = self.reactions.iter().flat_map(|reaction| {
            // All reactions depend on the reactor state
            let reactor_state = std::iter::once((
                GraphNode::Reaction(reaction),
                GraphNode::State(&self.state),
                false,
            ));

            // Trigger output reactions from timers.
            let trigger_reactions_timers = reaction.depends_on_timers.iter().map(move |timer| {
                (
                    GraphNode::Trigger(TriggerNodeType::Timer(timer)),
                    GraphNode::Reaction(reaction),
                    false,
                )
            });

            // Trigger output reactions from inputs
            let trigger_reactions_inputs = reaction.depends_on_inputs.iter().map(move |input| {
                (
                    GraphNode::Trigger(TriggerNodeType::Input(input)),
                    GraphNode::Reaction(reaction),
                    false,
                )
            });

            // Reaction input ports
            let reaction_inputs = reaction.depends_on_inputs.iter().map(move |input| {
                (
                    GraphNode::Reaction(reaction),
                    GraphNode::Input(input),
                    false,
                )
            });

            // Reaction output ports
            let reaction_outputs = reaction.provides_outputs.iter().map(move |output| {
                (
                    GraphNode::Reaction(reaction),
                    GraphNode::Output(output),
                    false,
                )
            });

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
                                false,
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
                .map(move |to| (GraphNode::Input(to), GraphNode::Output(from), false))
        });

        DiGraphMap::from_edges(reaction_edges.chain(port_connections))
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
            .collect::<Result<Vec<_>, _>>()?;

        let connections = connections
            .into_iter()
            .fold(HashMap::new(), |mut acc, (key, val)| {
                acc.entry(key).or_insert_with(Vec::new).push(val);
                acc
            });

        let state = Rc::new(ReactorStateBuilder {
            ident: receiver.ident,
            generics: receiver.generics,
        });

        Ok(ReactorBuilder {
            state,
            idents,
            timers: all_timers,
            inputs: all_inputs,
            outputs: all_outputs,
            reactions,
            connections,
        })
    }
}

impl ToTokens for ReactorBuilder {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        use petgraph::algo::{toposort, DfsSpace};
        use quote::quote;

        let graph = self.get_dependency_graph();
        let mut space = DfsSpace::new(&graph);
        let sorted = toposort(&graph, Some(&mut space));

        use petgraph::dot::{Config, Dot};
        let dot = Dot::with_config(&graph, &[Config::EdgeNoLabel]);
        println!("{}", dot);

        // Turn the reversed graph traversal into output tokens
        let graph_tokens = sorted
            .unwrap()
            .iter()
            .rev()
            .map(|node| {
                let out = NodeWithContext {
                    node: *node,
                    graph: &graph,
                };
                out.into_token_stream()
            })
            .collect::<proc_macro2::TokenStream>();

        let type_ident = &self.state.ident;
        let (imp, ty, wher) = self.state.generics.split_for_impl();

        tokens.extend(quote! {
            impl #imp #type_ident #ty #wher {
                // pub fn schedule(this: &Rc<RefCell<Self>>, scheduler: &mut S) {}
                pub fn create<S: Sched>(scheduler: &mut S) {
                    #graph_tokens

                    scheduler.schedule(trigger_tim1, Duration::from_micros(0), None);
                }
            }
        })
    }
}
