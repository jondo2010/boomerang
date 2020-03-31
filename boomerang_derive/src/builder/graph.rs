use super::{PortBuilder, ReactionBuilder, TimerBuilder};
use darling::ToTokens;
use derive_more::Display;
use quote::quote;
use std::rc::Rc;
use std::time::Duration;

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
            quote!(Duration::new(#secs, #nanos))
        }
        None => quote!(None),
    }
}

impl<'a, G> ToTokens for NodeWithContext<'a, G>
where
    G: petgraph::visit::IntoNeighborsDirected<NodeId = GraphNode<'a>>,
{
    /// # Panics
    /// This method panics if the field attributes input, output, timer are not mutually-exclusive.
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let NodeWithContext { node, graph } = self;
        match *node {
            GraphNode::Trigger(trigger) => match trigger {
                TriggerNodeType::Timer(timer) => {
                    let ident = &timer.attr.name;
                    let reactions = quote!(vec![]);

                    let offset = duration_quote(&timer.attr.offset);
                    let period = duration_quote(&timer.attr.period);

                    tokens.extend(quote! {
                        let #ident = std::rc::Rc::new(Trigger {
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
                TriggerNodeType::Input(_) => {}
            },
            GraphNode::Input(input) => {
                let ident = &input.attr.name;
                let ty = &input.attr.ty;
                let (out_ident, out_ty) = graph
                    .neighbors_directed(*node, petgraph::Direction::Outgoing)
                    .next()
                    .map(|node| {
                        if let GraphNode::Output(out_node) = node {
                            Some((&out_node.attr.name, &out_node.attr.ty))
                        } else {
                            None
                        }
                    })
                    .flatten()
                    .expect("Expected Output Node");
                assert_eq!(ty, out_ty, "Connected ports must have the same type.");
                tokens.extend(quote! {
                    let #ident = #out_ident.clone();
                })
            }
            GraphNode::Output(output) => {
                let ident = &output.attr.name;
                let ty = &output.attr.ty;
                tokens.extend(quote! {
                    let #ident = std::rc::Rc::new(
                        std::cell::RefCell::new(
                            Port::<#ty>::new(Default::default())
                        )
                    );
                });
            }
            GraphNode::Reaction(_) => {}
        };
    }
}
