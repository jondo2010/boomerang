use crate::{
    reactor::TriggerAttr,
    util::{MetaList, NamedField},
};
use darling::{FromAttributes, FromMeta};
use itertools::Itertools;
use quote::{format_ident, quote};
use syn::parse_quote;

#[cfg(test)]
mod test;

/// Triggers can be timers, inputs, outputs of contained reactors, or actions.
/// The uses field specifies inputs that the reaction observes but that do not trigger the
/// reaction.
/// The effects field declares which outputs and actions the target code may produce or
/// schedule.

/// Used for parsing attributes on the reaction function itself
#[derive(Debug, FromMeta)]
pub struct ReactionAttr {
    reactor: syn::Path,
    #[darling(default, map = "MetaList::into")]
    pub triggers: Vec<TriggerAttr>,
}

/// Used for parsing attributes on the reaction function arguments
#[derive(Debug, Default, FromMeta)]
struct PortRawArgumentAttr {
    triggers: Option<bool>,
    uses: Option<bool>,
    effects: Option<bool>,
    path: Option<syn::Expr>,
}

#[derive(Debug, Default, FromMeta)]
struct ActionRawArgumentAttr {
    triggers: Option<bool>,
    effects: Option<bool>,
    rename: Option<syn::Expr>,
}

#[derive(Debug)]
enum ArgumentAttr {
    Port {
        attrs: PortAttrs,
        path: Option<syn::Expr>,
    },
    Action {
        attrs: ActionAttrs,
        rename: Option<syn::Expr>,
    },
}

#[derive(Debug)]
enum PortAttrs {
    Triggers,
    Effects,
    Uses,
}

#[derive(Debug)]
enum ActionAttrs {
    Triggers,
    Effects,
    TriggersAndEffects,
}

#[derive(Debug)]
struct ReactionArg {
    ident: syn::Ident,
    attr: ArgumentAttr,
    ty: syn::Type,
}

pub(crate) struct ReactionReceiver {
    attr: ReactionAttr,
    itemfn: syn::ItemFn,
    args: Vec<ReactionArg>,
}

impl darling::FromAttributes for ArgumentAttr {
    fn from_attributes(attrs: &[syn::Attribute]) -> darling::Result<Self> {
        // Only support a single attribute per item
        if attrs.len() > 1 {
            return Err(darling::Error::too_many_items(1).with_span(&attrs[1]));
        }

        let meta = darling::util::parse_attribute_to_meta_list(&attrs[0])?;

        match darling::util::path_to_string(&meta.path).as_ref() {
            "reactor::port" => {
                let raw_attr = PortRawArgumentAttr::from_meta(&syn::Meta::List(meta))?;
                if !matches!(
                    raw_attr.path,
                    Some(syn::Expr::Field(_)) | Some(syn::Expr::Path(_)) | None
                ) {
                    return Err(darling::Error::unexpected_type(
                        "Expected ExprField such as 'a.b'",
                    )
                    .with_span(&raw_attr.path));
                }
                match raw_attr {
                    PortRawArgumentAttr {
                        triggers: Some(true),
                        uses: Some(false) | None,
                        effects: Some(false) | None,
                        ..
                    } => Ok(ArgumentAttr::Port {
                        attrs: PortAttrs::Triggers,
                        path: raw_attr.path,
                    }),
                    PortRawArgumentAttr {
                        triggers: Some(false) | None,
                        uses: Some(true),
                        effects: Some(false) | None,
                        ..
                    } => Ok(ArgumentAttr::Port {
                        attrs: PortAttrs::Uses,
                        path: raw_attr.path,
                    }),
                    PortRawArgumentAttr {
                        triggers: Some(false) | None,
                        uses: Some(false) | None,
                        effects: Some(true),
                        ..
                    } => Ok(ArgumentAttr::Port {
                        attrs: PortAttrs::Effects,
                        path: raw_attr.path,
                    }),
                    _ => Err(darling::Error::custom(
                        "A Port arg may be only one of 'triggers', 'effects', or 'uses",
                    )),
                }
            }
            "reactor::action" => {
                let raw_attr = ActionRawArgumentAttr::from_meta(&syn::Meta::List(meta))?;
                if !matches!(
                    raw_attr.rename,
                    Some(syn::Expr::Field(_)) | Some(syn::Expr::Path(_)) | None
                ) {
                    return Err(darling::Error::unexpected_type(
                        "Expected ExprField such as 'a.b'",
                    )
                    .with_span(&raw_attr.rename));
                }
                match raw_attr {
                    ActionRawArgumentAttr {
                        triggers: Some(true),
                        effects: Some(false) | None,
                        ..
                    } => Ok(ArgumentAttr::Action {
                        attrs: ActionAttrs::Triggers,
                        rename: raw_attr.rename,
                    }),
                    ActionRawArgumentAttr {
                        triggers: Some(false) | None,
                        effects: Some(true),
                        ..
                    } => Ok(ArgumentAttr::Action {
                        attrs: ActionAttrs::Effects,
                        rename: raw_attr.rename,
                    }),
                    ActionRawArgumentAttr {
                        triggers: Some(true),
                        effects: Some(true),
                        ..
                    } => Ok(ArgumentAttr::Action {
                        attrs: ActionAttrs::TriggersAndEffects,
                        rename: raw_attr.rename,
                    }),
                    _ => Err(darling::Error::custom(
                        "An Action arg may be either 'triggers', 'effects', or both.",
                    )),
                }
            }
            _ => Err(darling::Error::unknown_field_path(&meta.path)),
        }
    }
}

fn build_reaction_args(itemfn: &syn::ItemFn) -> darling::Result<Vec<ReactionArg>> {
    let mut errors = darling::Error::accumulator();
    let args = itemfn
        .sig
        .inputs
        .pairs()
        .filter_map(|arg| match arg.value() {
            syn::FnArg::Typed(typed) if !typed.attrs.is_empty() => errors.handle_in(|| {
                let arg_ident =
                    match typed.pat.as_ref() {
                        syn::Pat::Ident(pat_ident) => Ok(&pat_ident.ident),
                        _ => Err(darling::Error::custom("Unexpected PatType")
                            .with_span(typed.pat.as_ref())),
                    }?;

                let arg_attr = ArgumentAttr::from_attributes(&typed.attrs)?;

                /*
                let arg_ty = match typed.ty.as_ref() {
                    syn::Type::Reference(reference) => Ok(reference.elem),
                    //syn::Type::Path(path) => Ok(path.)
                    syn::Type
                    _ => {
                        Err(darling::Error::custom("Unexpected Type").with_span(typed.ty.as_ref()))
                    }
                }?;
                */
                let arg_ty = match typed.ty.as_ref() {
                    syn::Type::Reference(reference) => *reference.elem.clone(),
                    _ => *typed.ty.clone()
                };

                Ok(ReactionArg {
                    ident: arg_ident.clone(),
                    attr: arg_attr,
                    ty: arg_ty,
                })
            }),
            syn::FnArg::Receiver(_) => None,
            _ => None,
        })
        .collect_vec();

    errors.finish()?;
    return Ok(args);
}

impl ReactionReceiver {
    pub fn new(args: syn::AttributeArgs, mut itemfn: syn::ItemFn) -> darling::Result<Self> {
        // Parse the attributes on the overall function
        let attr = ReactionAttr::from_list(&args)?;

        // Parse the attributes on the function arguments
        let args = build_reaction_args(&itemfn)?;

        // Remove all function argument attributes, since the compiler will complain about them.
        for mut arg in itemfn.sig.inputs.pairs_mut() {
            if let syn::FnArg::Typed(typed) = arg.value_mut() {
                typed.attrs.clear();
            }
        }

        Ok(Self { attr, itemfn, args })
    }

    fn inputs_destructure_tokens(&self) -> proc_macro2::TokenStream {
        let port_idents = self
            .args
            .iter()
            .filter_map(|arg| match arg.attr {
                ArgumentAttr::Port {
                    attrs: PortAttrs::Triggers | PortAttrs::Uses,
                    ..
                } => Some(&arg.ident),
                _ => None,
            })
            .collect_vec();
        let num_ports = port_idents.len();
        if num_ports > 0 {
            quote! { let [#(#port_idents,)*]: &[&dyn ::boomerang::runtime::BasePort; #num_ports]
            = ::std::convert::TryInto::try_into(inputs).unwrap(); }
        } else {
            proc_macro2::TokenStream::new()
        }
    }

    fn outputs_destructure_tokens(&self) -> proc_macro2::TokenStream {
        let port_idents = self
            .args
            .iter()
            .filter_map(|arg| match arg.attr {
                ArgumentAttr::Port {
                    attrs: PortAttrs::Effects,
                    ..
                } => Some(&arg.ident),
                _ => None,
            })
            .collect_vec();
        let num_ports = port_idents.len();
        if num_ports > 0 {
            quote! {
                let [#(#port_idents,)*]: &mut [&mut dyn ::boomerang::runtime::BasePort; #num_ports]
                    = ::std::convert::TryInto::try_into(outputs).unwrap();
            }
        } else {
            proc_macro2::TokenStream::new()
        }
    }

    fn trig_actions_destructure_tokens(&self) -> proc_macro2::TokenStream {
        let action_idents = self
            .args
            .iter()
            .filter_map(|arg| match arg.attr {
                ArgumentAttr::Action {
                    attrs: ActionAttrs::Triggers,
                    ..
                } => Some(&arg.ident),
                _ => None,
            })
            .collect_vec();

        let num_idents = action_idents.len();
        if num_idents > 0 {
            quote! {
                let [#(#action_idents,)*]: &[&runtime::InternalAction; #num_idents]
                    = ::std::convert::TryInto::try_into(trig_actions).unwrap();
            }
        } else {
            proc_macro2::TokenStream::new()
        }
    }

    fn sched_actions_destructure_tokens(&self) -> proc_macro2::TokenStream {
        let action_idents = self
            .args
            .iter()
            .filter_map(|arg| match arg.attr {
                ArgumentAttr::Action {
                    attrs: ActionAttrs::Effects | ActionAttrs::TriggersAndEffects,
                    ..
                } => Some(&arg.ident),
                _ => None,
            })
            .collect_vec();

        let num_idents = action_idents.len();
        if num_idents > 0 {
            quote! {
                let [#(#action_idents,)*]: &mut [&mut runtime::InternalAction; #num_idents]
                    = ::std::convert::TryInto::try_into(sched_actions).unwrap();
            }
        } else {
            proc_macro2::TokenStream::new()
        }
    }

    fn fn_args_tokens(&self) -> impl Iterator<Item = proc_macro2::TokenStream> + '_ {
        self.args.iter().filter_map(|arg| {
            let ident = &arg.ident;
            // let ty = quote! {<#ty_elem as ::boomerang::runtime::AssociatedItem>::Inner};
            let ty = &arg.ty;
            match arg.attr {
                ArgumentAttr::Port {
                    attrs: PortAttrs::Triggers | PortAttrs::Uses,
                    ..
                } => Some(quote! { #ident.downcast_ref::<#ty>().expect("Wrong Port type!") }),
                ArgumentAttr::Port {
                    attrs: PortAttrs::Effects,
                    ..
                } => Some(quote! { #ident.downcast_mut::<#ty>().expect("Wrong Port type!") }),
                ArgumentAttr::Action {
                    //attrs: ActionAttrs::Effects | ActionAttrs::TriggersAndEffects,
                    ..
                } => Some(quote! { (* #ident).into() }),
                _ => todo!(),
            }
        })
    }

    fn find_items_tokens(&self) -> impl Iterator<Item = proc_macro2::TokenStream> + '_ {
        self.args.iter().filter_map(|arg| {
            let ident = &arg.ident;
            let name = ident.to_string();
            match arg.attr {
                ArgumentAttr::Port { .. } => Some(quote! {
                    let #ident = <::boomerang::builder::ReactorBuilderState<Self> as
                        ::boomerang::builder::FindElements>::get_port_by_name(builder, #name)?;
                }),
                ArgumentAttr::Action { .. } => Some(quote! {
                    let #ident = <::boomerang::builder::ReactionBuilderState<Self> as
                        ::boomerang::builder::FindElements>::get_action_by_name(builder, #name)?;
                }),
            }
        })
    }

    fn relations_tokens(&self) -> impl Iterator<Item = proc_macro2::TokenStream> + '_ {
        // Values from top-level attributes
        let trigger_vals = self.attr.triggers.iter().map(|trigger| match trigger {
            TriggerAttr::Startup => quote! { .with_trigger_action(__startup_action, 0)},
            TriggerAttr::Shutdown => quote! { .with_trigger_action(__shutdown_action, 0)},
            TriggerAttr::Action(action) => quote! { .with_trigger_action(reactor.#action, 0)},
            TriggerAttr::Timer(timer) => quote! { .with_trigger_action(reactor.#timer, 0) },
            TriggerAttr::Port(_) => todo!(),
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
                } => Some(quote! { .with_trigger_port::<#ty>(reactor.#expr, 0) }),
                ArgumentAttr::Port {
                    attrs: PortAttrs::Effects,
                    ..
                } => Some(quote! { .with_antidependency::<#ty>(reactor.#expr, 0) }),
                ArgumentAttr::Action {
                    attrs: ActionAttrs::Triggers,
                    ..
                } => Some(quote! { .with_trigger_action::<#ty>(reactor.#expr, 0) }),
                ArgumentAttr::Action {
                    attrs: ActionAttrs::Effects,
                    ..
                } => Some(quote! { .with_scheduable_action::<#ty>(reactor.#expr, 0) }),
                ArgumentAttr::Action {
                    attrs: ActionAttrs::TriggersAndEffects,
                    ..
                } => Some(quote! {
                    .with_trigger_action::<#ty>(reactor.#expr, 0)
                    .with_scheduable_action::<#ty>(reactor.#expr, 0)
                }),
                _ => None,
            }
        });
        trigger_vals.chain(arg_vals)
    }

    fn wrapper_signature(&self) -> syn::Signature {
        let signature = {
            let mut generics = self.itemfn.sig.generics.clone();
            generics
                .params
                .insert(0, syn::GenericParam::Lifetime(parse_quote! {'builder}));
            generics.params.push(syn::GenericParam::Type(
                parse_quote! {S: runtime::ReactorState},
            ));
            let reactor_path = &self.attr.reactor;
            syn::Signature {
                constness: None,
                asyncness: None,
                unsafety: None,
                abi: None,
                fn_token: self.itemfn.sig.fn_token,
                ident: format_ident!("__build_{}", self.itemfn.sig.ident),
                generics: generics,
                paren_token: self.itemfn.sig.paren_token,
                inputs: parse_quote!(
                    name: &str,
                    reactor: &#reactor_path,
                    builder: &'builder mut ::boomerang::builder::ReactorBuilderState<S>,
                ),
                variadic: None,
                output: parse_quote!(
                    -> Result<::boomerang::builder::ReactionBuilderState<'builder>,
                        ::boomerang::builder::BuilderError>
                ),
            }
        };
        signature
    }
}

impl darling::ToTokens for ReactionReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        // Build the wrapper closure
        let fn_ident = &self.itemfn.sig.ident;
        let inputs_destructure = self.inputs_destructure_tokens();
        let outputs_destructure = self.outputs_destructure_tokens();
        let trig_actions_destructure = self.trig_actions_destructure_tokens();
        let sched_actions_destructure = self.sched_actions_destructure_tokens();
        let fn_args = self.fn_args_tokens();
        let wrapper_block = quote! {
            let __wrapper: Box<dyn ::boomerang::runtime::ReactionFn> = Box::new(
                move |
                    ctx,
                    reactor,
                    inputs,
                    outputs,
                    trig_actions: &[&runtime::InternalAction],
                    sched_actions: &mut [&mut runtime::InternalAction]
                | {
                    let reactor = reactor.downcast_mut::<Self>().expect("Wrong Reactor Type!");
                    #inputs_destructure
                    #outputs_destructure
                    #trig_actions_destructure
                    #sched_actions_destructure
                    Self::#fn_ident(
                        reactor,
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
