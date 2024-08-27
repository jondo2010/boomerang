use itertools::Itertools;
use quote::{format_ident, quote};
use syn::parse_quote;

use crate::reactor::TriggerAttr;

use super::{ActionAttrs, ArgumentAttr, PortAttrs, ReactionReceiver};

impl ReactionReceiver {
    /// Returns an iterator over the identifiers of the inputs to the reaction
    fn inputs_idents(&self) -> impl Iterator<Item = &syn::Ident> {
        self.args.iter().filter_map(|arg| match arg.attr {
            ArgumentAttr::Port {
                attrs: PortAttrs::Triggers | PortAttrs::Uses,
                ..
            } => Some(&arg.ident),
            _ => None,
        })
    }

    /// Returns an iterator over the identifiers of the outputs to the reaction
    fn outputs_idents(&self) -> impl Iterator<Item = &syn::Ident> {
        self.args.iter().filter_map(|arg| match arg.attr {
            ArgumentAttr::Port {
                attrs: PortAttrs::Effects,
                ..
            } => Some(&arg.ident),
            _ => None,
        })
    }

    /// Returns an iterator over the identifiers of all actions used in the reaction.
    pub(crate) fn actions_idents(&self) -> impl Iterator<Item = syn::Ident> + '_ {
        // Actions from top-level attributes
        let attr_idents = self
            .attr
            .triggers
            .iter()
            .filter_map(|trigger| match trigger {
                TriggerAttr::Startup => Some(format_ident!("_startup")),
                TriggerAttr::Shutdown => Some(format_ident!("_shutdown")),
                TriggerAttr::Action(ident) => Some(ident.clone()),
                _ => None,
            });

        // Actions from function arguments
        let arg_idents = self.args.iter().filter_map(|arg| match arg.attr {
            ArgumentAttr::Action { .. } => Some(arg.ident.clone()),
            _ => None,
        });

        attr_idents.chain(arg_idents).dedup()
    }

    fn inputs_destructure_tokens(&self) -> proc_macro2::TokenStream {
        let port_idents = self.inputs_idents().collect_vec();
        let num_ports = port_idents.len();
        if num_ports > 0 {
            quote! { let [#(#port_idents,)*]: &[&Box<dyn ::boomerang::runtime::BasePort>; #num_ports]
            = ::std::convert::TryInto::try_into(inputs).unwrap(); }
        } else {
            proc_macro2::TokenStream::new()
        }
    }

    fn outputs_destructure_tokens(&self) -> proc_macro2::TokenStream {
        let port_idents = self.outputs_idents().collect_vec();
        let num_ports = port_idents.len();
        if num_ports > 0 {
            quote! {
                let [#(#port_idents,)*]: &mut [&mut Box<dyn ::boomerang::runtime::BasePort>; #num_ports]
                    = ::std::convert::TryInto::try_into(outputs).unwrap();
            }
        } else {
            proc_macro2::TokenStream::new()
        }
    }

    pub(crate) fn actions_destructure_tokens(&self) -> proc_macro2::TokenStream {
        let action_idents = self.actions_idents().collect_vec();
        let num_idents = action_idents.len();
        if num_idents > 0 {
            quote! {
                #[allow(unused_variables)]
                let [#(#action_idents,)*]: &mut [&mut runtime::Action; #num_idents]
                    = ::std::convert::TryInto::try_into(actions).unwrap();
            }
        } else {
            proc_macro2::TokenStream::new()
        }
    }

    fn fn_args_tokens(&self) -> impl Iterator<Item = proc_macro2::TokenStream> + '_ {
        self.args.iter().map(|arg| {
            let ident = &arg.ident;
            // let ty = quote! {<#ty_elem as ::boomerang::runtime::AssociatedItem>::Inner};
            let ty = &arg.ty;
            match arg.attr {
                ArgumentAttr::Port {
                    attrs: PortAttrs::Triggers | PortAttrs::Uses,
                    ..
                } => quote! { #ident.downcast_ref::<#ty>().expect("Wrong Port type!") },
                ArgumentAttr::Port {
                    attrs: PortAttrs::Effects,
                    ..
                } => quote! { #ident.downcast_mut::<#ty>().expect("Wrong Port type!") },
                ArgumentAttr::Action {
                    //attrs: ActionAttrs::Effects | ActionAttrs::TriggersAndEffects,
                    ..
                } => quote! { (* #ident).into() },
            }
        })
    }

    fn relations_tokens(&self) -> impl Iterator<Item = proc_macro2::TokenStream> + '_ {
        // Values from top-level attributes
        let trigger_vals = self.attr.triggers.iter().map(|trigger| match trigger {
            TriggerAttr::Startup => quote! { .with_trigger_action(__startup_action, 0)? },
            TriggerAttr::Shutdown => quote! { .with_trigger_action(__shutdown_action, 0)? },
            TriggerAttr::Action(action) => quote! { .with_trigger_action(reactor.#action, 0)? },
            TriggerAttr::Port(port) => quote! { .with_trigger_port(reactor.#port, 0)? },
        });

        // Values from argument attributes
        let arg_vals = self.args.iter().filter_map(|arg| {
            let expr = match &arg.attr {
                ArgumentAttr::Port {
                    path: Some(expr), ..
                }
                | ArgumentAttr::Action {
                    rename: Some(expr), ..
                } => quote!(#expr),
                _ => {
                    let ident = &arg.ident;
                    quote!(#ident)
                }
            };

            let ty = {
                let ty = &arg.ty;
                quote! {<#ty as ::boomerang::runtime::InnerType>::Inner}
            };

            match &arg.attr {
                ArgumentAttr::Port {
                    attrs: PortAttrs::Triggers,
                    ..
                } => Some(quote! {
                    .with_trigger_port(::boomerang::builder::TypedPortKey::<#ty>::from(reactor.#expr), 0)?
                }),
                ArgumentAttr::Port {
                    attrs: PortAttrs::Effects,
                    ..
                } => Some(quote! {
                    .with_effect_port(::boomerang::builder::TypedPortKey::<#ty>::from(reactor.#expr), 0)?
                }),
                ArgumentAttr::Action {
                    attrs: ActionAttrs::Triggers,
                    ..
                } => Some(quote! {
                    .with_trigger_action(::boomerang::builder::TypedActionKey::<#ty, _>::from(reactor.#expr), 0)?
                }),
                ArgumentAttr::Action {
                    attrs: ActionAttrs::Effects,
                    ..
                } => Some(quote! {
                    .with_effect_action(::boomerang::builder::TypedActionKey::<#ty, _>::from(reactor.#expr), 0)?
                }),
                ArgumentAttr::Action {
                    attrs: ActionAttrs::TriggersAndEffects,
                    ..
                } => Some(quote! {
                    .with_trigger_action(::boomerang::builder::TypedActionKey::<#ty, _>::from(reactor.#expr), 0)?
                    .with_effect_action(::boomerang::builder::TypedActionKey::<#ty, _>::from(reactor.#expr), 0)?
                }),
                _ => None,
            }
        });
        trigger_vals.chain(arg_vals)
    }

    fn wrapper_signature(&self) -> syn::Signature {
        {
            let mut generics = self.itemfn.sig.generics.clone();
            generics
                .params
                .insert(0, syn::GenericParam::Lifetime(parse_quote! {'builder}));
            let reactor_path = &self.attr.reactor;
            syn::Signature {
                constness: None,
                asyncness: None,
                unsafety: None,
                abi: None,
                fn_token: self.itemfn.sig.fn_token,
                ident: format_ident!("__build_{}", self.itemfn.sig.ident),
                generics,
                paren_token: self.itemfn.sig.paren_token,
                inputs: parse_quote!(
                    name: &str,
                    reactor: &#reactor_path,
                    builder: &'builder mut ::boomerang::builder::ReactorBuilderState,
                ),
                variadic: None,
                output: parse_quote!(
                    -> Result<::boomerang::builder::ReactionBuilderState<'builder>,
                        ::boomerang::builder::BuilderError>
                ),
            }
        }
    }
}

impl darling::ToTokens for ReactionReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        // Build the wrapper closure
        let fn_ident = &self.itemfn.sig.ident;
        let inputs_destructure = self.inputs_destructure_tokens();
        let outputs_destructure = self.outputs_destructure_tokens();
        let actions_destructure = self.actions_destructure_tokens();
        let fn_args = self.fn_args_tokens();
        let wrapper_block = quote! {
            let __wrapper: ::boomerang::runtime::ReactionFn = Box::new(
                move |
                    ctx: &mut ::boomerang::runtime::Context,
                    state: &mut dyn runtime::ReactorState,
                    inputs,
                    outputs,
                    actions: &mut [&mut runtime::Action]
                | {
                    #inputs_destructure
                    #outputs_destructure
                    #actions_destructure
                    Self::#fn_ident(
                        state.downcast_mut().expect("Unable to downcast reactor state"),
                        ctx,
                        #(#fn_args,)*
                    );
                });
        };

        // Build the rest of the reaction declaration
        let relations = self.relations_tokens();
        let builder_block = quote! {
            let __startup_action = builder.get_startup_action();
            let __shutdown_action = builder.get_shutdown_action();
            let reaction = builder.add_reaction(&name, __wrapper)
                #(#relations)*
            ;
            Ok(reaction)
        };

        let builder_fn = syn::ItemFn {
            attrs: Vec::new(),
            vis: syn::Visibility::Public(syn::VisPublic {
                pub_token: Default::default(),
            }),
            sig: self.wrapper_signature(),
            block: parse_quote! {{
                #wrapper_block
                #builder_block
            }},
        };

        tokens.extend(builder_fn.to_token_stream());
        tokens.extend(self.itemfn.to_token_stream());
    }
}
