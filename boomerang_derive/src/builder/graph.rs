use super::{PortBuilder, ReactionBuilder, ReactorStateBuilder, TimerBuilder};
use darling::ToTokens;
use derive_more::Display;
use quote::{format_ident, quote};
use std::{rc::Rc, time::Duration};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Display)]
pub enum TriggerNodeType<'a> {
    #[display(fmt = "Timer: {}", _0.)]
    Timer(&'a Rc<TimerBuilder>),
    #[display(fmt = "Input: {}", _0.)]
    Input(&'a Rc<PortBuilder>),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Display)]
pub enum GraphNode<'a> {
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

impl<'a> GraphNode<'a> {
    /// Create an ident for code generation
    fn create_ident(&self) -> syn::Ident {
        match *self {
            GraphNode::Trigger(TriggerNodeType::Input(input)) => {
                format_ident!("trigger_{}", &input.attr.name)
            }
            GraphNode::Trigger(TriggerNodeType::Timer(timer)) => {
                format_ident!("trigger_{}", &timer.attr.name)
            }
            GraphNode::Input(input) => format_ident!("input_{}", &input.attr.name),
            GraphNode::Output(output) => format_ident!("output_{}", &output.attr.name),
            GraphNode::Reaction(reaction) => format_ident!(
                "reaction_{}",
                &reaction.attr.function.segments.last().unwrap().ident
            ),
            GraphNode::State(state) => {
                format_ident!("state_{}", &state.ident.to_string().to_lowercase())
            }
        }
    }
}

pub struct NodeWithContext<'a, G> {
    pub node: GraphNode<'a>,
    pub graph: G,
}

fn duration_quote(duration: &Option<Duration>) -> proc_macro2::TokenStream {
    match duration {
        Some(offset) => {
            let secs = offset.as_secs();
            let nanos = offset.subsec_nanos();
            quote!(Some(Duration::new(#secs, #nanos)))
        }
        None => quote!(None),
    }
}

impl<'a, G> ToTokens for NodeWithContext<'a, G>
where
    G: petgraph::visit::IntoNeighborsDirected<NodeId = GraphNode<'a>>,
{
    /// # Panics
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let NodeWithContext { node, graph } = self;
        let node_ident = node.create_ident();
        match *node {
            GraphNode::Trigger(trigger) => {
                let (offset, period) = match trigger {
                    TriggerNodeType::Timer(timer) => {
                        let offset = duration_quote(&timer.attr.offset);
                        let period = duration_quote(&timer.attr.period);
                        (offset, period)
                    }
                    TriggerNodeType::Input(input) => {
                        let offset = quote!(None);
                        let period = quote!(None);
                        (offset, period)
                    }
                };

                let reactions_iter = graph
                    .neighbors_directed(*node, petgraph::Direction::Outgoing)
                    .filter_map(|node| match node {
                        GraphNode::Reaction(_) => Some(node.create_ident()),
                        _ => None,
                    });

                let reactions = quote!(vec![#(#reactions_iter.clone()),*]);

                tokens.extend(quote! {
                    let #node_ident = std::rc::Rc::new(Trigger {
                        reactions: #reactions,
                        offset: #offset,
                        period: #period,
                        value: std::rc::Rc::new(RefCell::new(None)),
                        is_physical: false,
                        scheduled: RefCell::new(None),
                        policy: QueuingPolicy::NONE,
                    });
                })
            }
            GraphNode::Input(input) => {
                let ty = &input.attr.ty;
                if let Some(out_ident) = graph
                    .neighbors_directed(*node, petgraph::Direction::Outgoing)
                    .next()
                    .map(|node| match node {
                        GraphNode::Output(out_node) if out_node.attr.ty == *ty => {
                            Some(node.create_ident())
                        }
                        _ => None,
                    })
                    .flatten()
                {
                    // If there is an output port connect to this input, clone it.
                    tokens.extend(quote! {
                        let #node_ident = #out_ident.clone();
                    })
                } else {
                    // Otherwise, create a new, disconnected one.
                    tokens.extend(quote!{
                        let #node_ident = std::rc::Rc::new(
                            std::cell::RefCell::new(
                                <Port::<#ty>>::new(Default::default())
                            )
                        );
                    });
                }
            }
            GraphNode::Output(output) => {
                let ty = &output.attr.ty;
                tokens.extend(quote! {
                    let #node_ident = std::rc::Rc::new(
                        std::cell::RefCell::new(
                            <Port::<#ty>>::new(Default::default())
                        )
                    );
                });
            }
            GraphNode::Reaction(reaction) => {
                let function = &reaction.attr.function;

                let state_ident = graph
                    .neighbors_directed(*node, petgraph::Direction::Outgoing)
                    .filter_map(|node| match node {
                        GraphNode::State(_) => Some(node.create_ident()),
                        _ => None,
                    })
                    .next()
                    .expect("State node not found for Reaction");

                let input_idents_iter = reaction
                    .depends_on_inputs
                    .iter()
                    .map(|input| GraphNode::Input(input).create_ident())
                    .collect::<Vec<_>>();

                let clone_input_idents = {
                    let iter = input_idents_iter.iter().map(|ident| {
                        let cloned_ident = format_ident!("_{}_cloned", ident);
                        quote!(let #cloned_ident = #ident.clone();)
                    });
                    quote!(#(#iter)*)
                };

                let input_idents = {
                    let iter = input_idents_iter
                        .iter()
                        .map(|ident| format_ident!("_{}_cloned", ident));
                    quote!(#(&mut *#iter.borrow_mut()),*)
                };

                let output_nodes = graph
                    .neighbors_directed(*node, petgraph::Direction::Outgoing)
                    .filter_map(|node| match node {
                        GraphNode::Output(_) => Some(node),
                        _ => None,
                    })
                    .collect::<Vec<_>>();

                let output_idents_iter = output_nodes
                    .iter()
                    .map(|node| node.create_ident())
                    .collect::<Vec<_>>();

                let clone_output_idents = {
                    let iter = output_idents_iter.iter().map(|output| {
                        let cloned_ident = format_ident!("_{}_cloned", output);
                        quote!(let #cloned_ident = #output.clone();)
                    });
                    quote!(#(#iter)*)
                };

                let output_idents = {
                    let iter = output_idents_iter
                        .iter()
                        .map(|output| format_ident!("_{}_cloned", output));
                    quote!(#(&mut *#iter.borrow_mut()),*)
                };

                let output_triggers_iter = output_nodes.iter().map(|node| {
                    let output_ident = node.create_ident();
                    // Get all input ports (triggers) into this output port
                    let triggers = graph
                        .neighbors_directed(*node, petgraph::Direction::Incoming)
                        .filter_map(|node| match node {
                            GraphNode::Input(input) => Some(
                                GraphNode::Trigger(TriggerNodeType::Input(input)).create_ident(),
                            ),
                            _ => None,
                        });
                    quote! {(
                        #output_ident.clone() as std::rc::Rc<std::cell::RefCell<dyn IsPresent>>,
                        vec![#(#triggers.clone()),*]
                    )}
                });

                let output_triggers = quote!(#(#output_triggers_iter),*);

                tokens.extend(quote! {
                    let #node_ident = {
                        let _state_cloned = #state_ident.clone();
                        #clone_input_idents;
                        #clone_output_idents;
                        let _closure = std::boxed::Box::new(
                            std::cell::RefCell::new(move |sched: &mut S| {
                            #function(
                                &mut (*_state_cloned).borrow_mut(),
                                sched,
                                (#input_idents),
                                (#output_idents),
                            );
                        }));
                        let _output_triggers = vec![#output_triggers];

                        std::rc::Rc::new(Reaction::new(
                            "reply_reaction",
                            _closure,
                            u64::MAX,
                            1,
                            _output_triggers,
                        ))
                    };
                });
            }
            GraphNode::State(state) => {
                tokens.extend(quote! {
                    let #node_ident = std::rc::Rc::new(
                        std::cell::RefCell::new(
                            Self::default()
                        )
                    );
                });
            }
        };
    }
}
