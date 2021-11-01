#![feature(hash_set_entry)]

//! This crate provides Boomerangs' derive macro.

mod parse;
mod util;

use darling::{FromDeriveInput, ToTokens};
use parse::ReactorReceiver;
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{format_ident, quote};
use syn::parse_macro_input;
use util::NamedField;

use crate::parse::TriggerAttr;

fn build_reactor_parts(
    receiver: &ReactorReceiver,
    tokens: &mut proc_macro2::TokenStream,
) -> (proc_macro2::Ident, proc_macro2::Ident, proc_macro2::Ident) {
    let ident = &receiver.ident;
    let inputs_ident = format_ident!("{}Inputs", &ident);
    let outputs_ident = format_ident!("{}Outputs", &ident);
    let actions_ident = format_ident!("{}Actions", &ident);

    let input_decls = receiver.inputs.iter().map(|port| {
        let name = &port.name;
        let ty = &port.ty;
        quote! { (#name, #ty) }
    });
    let output_decls = receiver.outputs.iter().map(|port| {
        let name = &port.name;
        let ty = &port.ty;
        quote! { (#name, #ty) }
    });
    let action_decls = receiver.actions.iter().map(|action| {
        let name = &action.name;
        let ty = &action.ty;
        quote! { (#name, #ty) }
    });
    tokens.extend(quote! {
        ::boomerang::ReactorInputs!(
            #ident,
            #inputs_ident,
            #(#input_decls),*
        );
    });
    tokens.extend(quote! {
        ::boomerang::ReactorOutputs!(
            #ident,
            #outputs_ident,
            #(#output_decls),*
        );
    });
    tokens.extend(quote! {
        ::boomerang::ReactorActions!(
            #ident,
            #actions_ident,
            #(#action_decls),*
        );
    });
    (inputs_ident, outputs_ident, actions_ident)
}

fn build_internal_ports(receiver: &ReactorReceiver) -> proc_macro2::TokenStream {
    let slf = Ident::new("self", Span::call_site());
    let internal_port_decls = receiver
        .reactions
        .iter()
        .flat_map(|reaction| reaction.effects.iter())
        .filter_map(|effect| {
            if effect.0.eq(&slf) {
                None
            } else {
                let port_ident = format_ident!("__internal_{}_{}", effect.0, effect.1);
                let name = port_ident.to_string();
                Some(quote! {
                    let #port_ident = __builder.add_internal_port::<()>(
                        #name,
                        ::boomerang::builder::PortType::Input
                    )?;
                })
            }
        });

    quote! {
        #(#internal_port_decls)*
    }
}

fn build_timers(receiver: &ReactorReceiver) -> proc_macro2::TokenStream {
    let timers_decl = receiver.timers.iter().map(|timer| {
        let ident = format_ident!("__timer_{}", &timer.name);
        let name = timer.name.to_string();

        let period = util::duration_quote(&timer.period.unwrap_or_default());
        let offset = util::duration_quote(&timer.offset.unwrap_or_default());
        quote! {
            let #ident = __builder.add_timer(#name, #period, #offset)?;
        }
    });
    quote! {
        let __startup = __builder.add_startup_action("startup")?;
        let __shutdown = __builder.add_startup_action("shutdown")?;
        #(#timers_decl)*
    }
}

fn build_reactions(receiver: &ReactorReceiver) -> proc_macro2::TokenStream {
    let reaction_decls = receiver.reactions.iter().map(|reaction| {
        let function = &reaction.function;
        let name = quote! {#function}.to_string();

        let trigger_parts = reaction.triggers.iter().map(|trigger| match trigger {
            TriggerAttr::Startup => quote! {
                .with_trigger_action(__startup)
            },
            TriggerAttr::Shutdown => quote! {
                .with_trigger_action(__shutdown)
            },
            TriggerAttr::Timer(timer) => {
                if receiver.timers.iter().find(|&t| t.name == *timer).is_some() {
                    let timer = format_ident!("__timer_{}", timer);
                    quote! {
                        .with_trigger_action(#timer)
                    }
                } else {
                    syn::Error::new(timer.span(), format!("Timer '{}' not found.", timer))
                        .to_compile_error()
                }
            }
            TriggerAttr::Port(port) => {
                let port = build_port(receiver, port, "inputs");
                quote! {
                    .with_trigger_port(#port)
                }
            }
        });

        let effect_parts = reaction.effects.iter().map(|effect| {
            let port = build_port(receiver, effect, "outputs");
            quote! {
                .with_antidependency(#port)
            }
        });
        quote! {
            let _ = __builder
                .add_reaction(#name, #function)
                #(#effect_parts)*
                #(#trigger_parts)*
                .finish();
        }
    });
    quote! {
        #(#reaction_decls)*
    }
}

/// Build the macro output for a named port, e.g., `__inputs_foo.x`
fn build_port(
    receiver: &ReactorReceiver,
    named_field: &NamedField,
    suffix: &str,
) -> proc_macro2::TokenStream {
    let NamedField(reactor, port) = named_field;
    let slf = Ident::new("self", Span::call_site());

    if *reactor != slf
        && receiver
            .children
            .iter()
            .find(|&child| child.name == *reactor)
            .is_none()
    {
        let children = itertools::join(receiver.children.iter().map(|child| &child.name), ", ");
        return syn::Error::new(
            port.span(),
            format!("Child Reactor '{}' not found in [{}]", reactor, children),
        )
        .to_compile_error();
    }

    let base = format_ident!("__{}_{}", reactor, suffix);
    quote! {#base.#port}
}

fn build_bindings(receiver: &ReactorReceiver) -> proc_macro2::TokenStream {
    let bindings_decls = receiver.connections.iter().map(|connection| {
        let from_port = build_port(receiver, &connection.from, "outputs");
        let to_port = build_port(receiver, &connection.to, "inputs");

        quote! {
            env.bind_port(#from_port, #to_port)?;
        }
    });

    // Create bindings for internal ports
    let slf = Ident::new("self", Span::call_site());
    let internal_port_bindings = receiver
        .reactions
        .iter()
        .flat_map(|reaction| reaction.effects.iter())
        .filter_map(|effect| {
            if effect.0.eq(&slf) {
                None
            } else {
                let internal_port = format_ident!("__internal_{}_{}", effect.0, effect.1);
                let reactor = format_ident!("__{}_inputs", effect.0);
                let external_port = &effect.1;
                Some(quote! {
                    env.bind_port(#internal_port, #reactor.#external_port)?;
                })
            }
        });

    quote! {
        #(#bindings_decls)*
        #(#internal_port_bindings)*
    }
}

fn build_children(receiver: &ReactorReceiver) -> proc_macro2::TokenStream {
    let children_decls = receiver.children.iter().map(|child| {
        let reactor = &child.reactor;
        let name = child.name.to_string();
        let key = format_ident!("__{}_key", &name);
        let inputs = format_ident!("__{}_inputs", &name);
        let outputs = format_ident!("__{}_outputs", &name);
        quote! {
            let (#key, #inputs, #outputs) = #reactor.build(#name, env, Some(__parent_key))?;
        }
    });
    quote! {
        #(#children_decls)*
    }
}

impl ToTokens for ReactorReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;

        let (ref inputs_ident, ref outputs_ident, ref actions_ident) =
            build_reactor_parts(self, tokens);

        let internal_ports = build_internal_ports(self);
        let timers = build_timers(self);
        let reactions = build_reactions(self);
        let bindings = build_bindings(self);
        let children = build_children(self);

        tokens.extend(quote! {
            #[automatically_derived]
            impl<S> ::boomerang::builder::Reactor<S> for #ident
                where S: ::boomerang_runtime::SchedulerPoint
            {
                type Inputs = #inputs_ident;
                type Outputs = #outputs_ident;
                type Actions = #actions_ident;

                fn build_parts<'__b>(
                    &'__b self,
                    env: &'__b mut ::boomerang::builder::EnvBuilder<S>,
                    reactor_key: ::boomerang_runtime::ReactorKey,
                ) -> Result<
                    (Self::Inputs, Self::Outputs, Self::Actions),
                    ::boomerang::builder::BuilderError,
                > {
                    Ok((
                        Self::Inputs::build(env, reactor_key)?,
                        Self::Outputs::build(env, reactor_key)?,
                        Self::Actions::build(env, reactor_key)?,
                    ))
                }

                fn build(
                    self,
                    name: &str,
                    env: &mut ::boomerang::builder::EnvBuilder<S>,
                    parent: Option<::boomerang_runtime::ReactorKey>,
                ) -> Result<
                    (::boomerang_runtime::ReactorKey, Self::Inputs, Self::Outputs),
                    ::boomerang::builder::BuilderError,
                > {
                    let mut __builder = env.add_reactor(name, parent, self);
                    let __self_inputs = __builder.inputs.clone();
                    let __self_outputs = __builder.outputs.clone();
                    let __self_actions = __builder.actions.clone();

                    #internal_ports;
                    #timers
                    #reactions

                    let (__parent_key, __inputs_self, __outputs_self) = __builder.finish()?;

                    #children
                    #bindings

                    Ok((__parent_key, __inputs_self, __outputs_self))
                }
            }
        });
    }
}

#[doc(hidden)]
#[proc_macro_derive(Reactor, attributes(reactor))]
pub fn derive(input: TokenStream) -> TokenStream {
    #[cfg(feature = "logging")]
    INIT_LOGGER.call_once(|| {
        env_logger::init().unwrap();
    });
    let ast = parse_macro_input!(input as syn::DeriveInput);
    let receiver = parse::ReactorReceiver::from_derive_input(&ast).unwrap();
    quote!(#receiver).into()
}
