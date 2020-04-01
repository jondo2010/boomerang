use darling::ToTokens;
use quote::{format_ident, quote};
use std::time::Duration;

use super::{GraphNode, NodeWithContext, ReactionBuilder, ReactorBuilderGen, TriggerNodeType};

/// Generate a TokenSTream from an Option<Duration>
fn duration_quote(duration: &Option<Duration>) -> proc_macro2::TokenStream {
    match duration {
        Some(offset) => {
            let secs = offset.as_secs();
            let nanos = offset.subsec_nanos();
            quote!(Some(boomerang::Duration::new(#secs, #nanos)))
        }
        None => quote!(None),
    }
}

/// Generate member types
fn node_ty(node: &GraphNode) -> proc_macro2::TokenStream {
    match *node {
        GraphNode::Output(output) => {
            let ty = output.ty.as_ref().unwrap();
            quote!(::std::rc::Rc<::std::cell::RefCell<::boomerang::Port::<#ty>>>)
        }
        GraphNode::Trigger(TriggerNodeType::Timer(_)) => {
            quote!(::std::rc::Rc<::boomerang::Trigger<S>>)
        }
        _ => quote!(),
    }
}

fn build_internal_struct<'a, G>(struct_ident: &syn::Ident, graph: G) -> proc_macro2::TokenStream
where
    G: petgraph::visit::GraphBase<NodeId = GraphNode<'a>>,
    G: petgraph::visit::IntoNodeIdentifiers,
{
    let struct_decls = graph.node_identifiers().filter_map(|node| {
        let node_ident = node.create_ident();
        let ty = node_ty(&node);
        match node {
            GraphNode::Output(_) => Some(quote!(#node_ident : #ty,)),
            GraphNode::Trigger(TriggerNodeType::Timer(_)) => Some(quote!(#node_ident : #ty,)),
            _ => None,
        }
    });
    let output_resets = {
        let idents = graph.node_identifiers().filter_map(|node| match node {
            GraphNode::Output(_) => Some(node.create_ident()),
            _ => None,
        });
        quote!(#(
            ::boomerang::IsPresent::reset(
                &mut *self.#idents.borrow_mut()
            )
        );*)
    };
    let timer_starts = {
        let idents = graph.node_identifiers().filter_map(|node| match node {
            GraphNode::Trigger(TriggerNodeType::Timer(_)) => Some(node.create_ident()),
            _ => None,
        });
        quote!(std::boxed::Box::new([#(self.#idents.clone()),*]))
    };

    quote! {
        #[derive(Debug)]
        struct #struct_ident <S>
        where
            S: ::boomerang::Sched,
            <S as ::boomerang::Sched>::Value: ::std::fmt::Debug,
        {
            #(#struct_decls)*
            _phantom: ::std::marker::PhantomData<S>,
        }

        #[automatically_derived]
        impl<S> ::boomerang::Reactor for #struct_ident <S>
        where
            S: ::boomerang::Sched,
            <S as ::boomerang::Sched>::Value: ::std::fmt::Debug,
        {
            type Sched = S;

            fn start_time_step(&self) {
                #output_resets
            }

            fn get_starting_timers(
                &self
            ) -> ::std::boxed::Box<[::std::rc::Rc<::boomerang::Trigger<Self::Sched>>]>
            {
                #timer_starts
            }

            fn wrapup(&self) -> bool {
                false
            }
        }
    }
}

impl ToTokens for ReactorBuilderGen {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        use petgraph::{
            algo::{toposort, DfsSpace},
            dot::{Config, Dot},
            graphmap::DiGraphMap,
        };

        let graph = self.get_dependency_graph();
        let mut space = DfsSpace::new(&graph);
        let sorted = toposort(&graph, Some(&mut space));

        let graph2: DiGraphMap<_, _> = self.into();

        let dot = Dot::with_config(&graph2, &[Config::EdgeNoLabel]);
        println!("{:?}", dot);

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
        let reactor_struct_ident = format_ident!("{}Reactor", &self.state.ident);
        let reactor_struct = build_internal_struct(&reactor_struct_ident, &graph);

        let ret_struct_values = graph.nodes().filter_map(|node| {
            let node_ident = node.create_ident();
            match node {
                GraphNode::Output(_) => Some(quote!(#node_ident,)),
                GraphNode::Trigger(TriggerNodeType::Timer(_)) => Some(quote!(#node_ident,)),
                _ => None,
            }
        });

        let (imp, ty, wher) = self.state.generics.split_for_impl();
        tokens.extend(quote! {
            #reactor_struct

            #[automatically_derived]
            impl #imp #type_ident #ty #wher {
                fn create_reactor<S>() -> ::std::boxed::Box< #reactor_struct_ident <S> >
                where
                    S: ::boomerang::Sched,
                    S::Value: ::std::fmt::Debug
                {
                    #graph_tokens

                    ::std::boxed::Box::new( #reactor_struct_ident {
                        #(#ret_struct_values)*
                        _phantom: ::std::marker::PhantomData,
                    })
                }
            }
        });

        // TODO: New stuff here:

        let inputs_idents = self
            .inputs
            .values()
            .map(|input| format_ident!("__{}", &input.name));

        let inputs_struct_ident = format_ident!("{}Inputs", &self.state.ident);
        let outputs_struct_ident = format_ident!("{}Outputs", &self.state.ident);

        let inputs_struct = generate_input_output_struct(
            &inputs_struct_ident,
            self.inputs
                .values()
                .map(|port| (format_ident!("__{}", port.name), port.ty.as_ref().unwrap())),
        );
        let outputs_struct = generate_input_output_struct(
            &outputs_struct_ident,
            self.outputs
                .values()
                .map(|port| (format_ident!("__{}", &port.name), port.ty.as_ref().unwrap())),
        );

        // Only create destructure statement if there actually are any inputs.
        let inputs_destructure = if self.inputs.is_empty() {
            quote!()
        } else {
            quote! {let Self::Inputs { #(#inputs_idents),* ,.. } = inputs;}
        };

        let reactor_builder_struct = generate_reactor_builder_struct(self);

        tokens.extend(quote! {
            #inputs_struct
            #outputs_struct

            #[automatically_derived]
            impl #imp ::boomerang::builder::ReactorBuildable for #type_ident #ty #wher {
                type Inputs = #inputs_struct_ident;
                type Outputs = #outputs_struct_ident;
                fn create<S: Sched>(inputs: Self::Inputs) -> (::boomerang::builder::ReactorBuilder<S>, Self::Outputs) {
                    // Destructure the inputs struct
                    #inputs_destructure

                    #reactor_builder_struct

                    (
                        __reactor,
                        Self::Outputs::default()
                    )
                }
            }
        });
    }
}

fn generate_reactor_builder_struct(_builder: &ReactorBuilderGen) -> proc_macro2::TokenStream {
    quote! {
        let __reactor = {
            let reactions = vec![];
            ::boomerang::builder::ReactorBuilder {
                timers: [].iter().cloned().collect(),
                inputs: [].iter().cloned().collect(),
                outputs: [].iter().cloned().collect(),
                children: [].iter().cloned().collect(),
                reactions,
            }
        };
    }
}

impl ToTokens for ReactionBuilder {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let reaction_ident = format_ident!("__react{}", self.index);
        tokens.extend(quote! {
            ::boomerang::ReactionBuilder {
                reaction: #reaction_ident,
                depends_on_timers: vec![],
                depends_on_inputs: vec![],
                provides_outputs: vec![],
            }
        })
    }
}

/// Generate the Input/Output struct and Default impl for ReactorBuildable
fn generate_input_output_struct<'a, V>(
    struct_ident: &syn::Ident,
    values: V,
) -> proc_macro2::TokenStream
where
    V: IntoIterator<Item = (syn::Ident, &'a syn::Type)>,
    V::IntoIter: Clone,
{
    let iter = values.into_iter();
    let member_decls = iter.clone().map(|attr| {
        let (ident, ty) = attr;
        quote!(#ident : ::std::rc::Rc<::std::cell::RefCell<::boomerang::Port::<#ty>>>)
    });

    let member_defaults = iter.clone().map(|attr| {
        let (ident, ty) = attr;
        quote!(#ident : ::std::rc::Rc::new(::std::cell::RefCell::new(<::boomerang::Port::<#ty>>::new(::std::default::Default::default()))))
    });

    quote! {
        pub struct #struct_ident {
            #(#member_decls),*
        }
        #[automatically_derived]
        impl ::std::default::Default for #struct_ident {
            fn default() -> Self {
                Self {
                    #(#member_defaults),*
                }
            }
        }
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
                        let offset = duration_quote(&timer.offset);
                        let period = duration_quote(&timer.period);
                        (offset, period)
                    }
                    TriggerNodeType::Input(_) => {
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
                    let #node_ident = std::rc::Rc::new(::boomerang::Trigger {
                        reactions: #reactions,
                        offset: #offset,
                        period: #period,
                        value: ::std::rc::Rc::new(::std::cell::RefCell::new(None)),
                        is_physical: false,
                        scheduled: ::std::cell::RefCell::new(None),
                        policy: ::boomerang::QueuingPolicy::NONE,
                    });
                })
            }
            GraphNode::Input(input) => {
                let ty = &input.ty.as_ref().unwrap();
                if let Some(out_ident) = graph
                    .neighbors_directed(*node, petgraph::Direction::Outgoing)
                    .next()
                    .map(|node| match node {
                        GraphNode::Output(out_node) if out_node.ty.as_ref().unwrap() == *ty => {
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
                    tokens.extend(quote! {
                        let #node_ident = std::rc::Rc::new(
                            ::std::cell::RefCell::new(
                                <::boomerang::Port::<#ty>>::new(Default::default())
                            )
                        );
                    });
                }
            }
            GraphNode::Output(output) => {
                let ty = &output.ty.as_ref().unwrap();
                tokens.extend(quote! {
                    let #node_ident = std::rc::Rc::new(
                        std::cell::RefCell::new(
                            <boomerang::Port::<#ty>>::new(Default::default())
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
                        #output_ident.clone() as std::rc::Rc<
                            std::cell::RefCell<dyn boomerang::IsPresent>>,
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

                        std::rc::Rc::new(boomerang::Reaction::new(
                            "reply_reaction",
                            _closure,
                            u64::MAX,
                            1,
                            _output_triggers,
                        ))
                    };
                });
            }
            GraphNode::State(_) => {
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
