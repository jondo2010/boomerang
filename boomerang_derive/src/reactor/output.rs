use darling::ToTokens;
use itertools::{Either, Itertools};
use quote::{format_ident, quote};

use super::{ReactorField, ReactorFieldInner};
use crate::{
    reactor::ReactorReceiver,
    util::{self},
};

#[cfg(feature = "disabled")]
impl<'a, 'b> ToTokens for ReactorFieldBuilder<'a, 'b> {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            field,
            builder_ident,
        } = self;
        tokens.extend(match field {
            ReactorField::Timer { ident, name, period, offset } => {
                let period = util::tokenize_duration(period);
                let offset = util::tokenize_duration(offset);
                quote! { let #ident = #builder_ident.add_timer(#name, #period, #offset)?; }
            }
            ReactorField::Input { ident, name, ty } => {
                let ty = quote! {<#ty as ::boomerang::runtime::InnerType>::Inner};
                quote! { let #ident = #builder_ident.add_port::<#ty>(#name, ::boomerang::builder::PortType::Input)?; }
            }
            ReactorField::Output { ident, name, ty } => {
                let ty = quote! {<#ty as ::boomerang::runtime::InnerType>::Inner};
                quote! { let #ident = #builder_ident.add_port::<#ty>(#name, ::boomerang::builder::PortType::Output)?; }
            }
            ReactorField::Action { ident, name, ty, physical: _, min_delay, mit: _, policy :_} => {
                let ty = quote! {<#ty as ::boomerang::runtime::InnerType>::Inner};
                let min_delay = OptionalDuration(**min_delay);
                quote! { let #ident = #builder_ident.add_logical_action::<#ty>(#name, #min_delay)?; }
            }
            ReactorField::Child { ident, name, state, ty } => {
                quote! { let #ident: #ty = #builder_ident.add_child_reactor(#name, #state)?; }
            }
            ReactorField::Reaction { ident, path } => {
                quote! {
                    reactor.#ident =
                        #path(stringify!(#ident), &reactor, &mut #builder_ident).and_then(|b| b.finish())?;
                }
            }
        });
    }
}

fn build_field_args(field: &ReactorField) -> proc_macro2::TokenStream {
    match field {
        ReactorField {
            inner: ReactorFieldInner::Timer { period, offset },
            ..
        } => {
            let period = util::tokenize_optional(*period, util::tokenize_duration);
            let offset = util::tokenize_optional(*offset, util::tokenize_duration);
            quote! { ::boomerang::builder::args::TimerArgs { period: #period, offset: #offset } }
        }
        ReactorField {
            inner:
                ReactorFieldInner::Action {
                    physical,
                    min_delay,
                    mit,
                    policy,
                },
            ..
        } => {
            let min_delay = util::tokenize_optional(*min_delay, util::tokenize_duration);
            let mit = util::tokenize_optional(*mit, util::tokenize_duration);
            quote! { ::boomerang::builder::args::ActionArgs { physical: #physical, min_delay: #min_delay, mit: #mit} }
        }
        ReactorField {
            inner: ReactorFieldInner::Child { state },
            ..
        } => {
            quote! { ::boomerang::builder::args::ChildArgs { state: #state } }
        }
        ReactorField {
            inner: ReactorFieldInner::Reaction { path },
            ..
        } => {
            quote! { () }
        }
        ReactorField {
            inner: ReactorFieldInner::Empty,
            ..
        } => quote! { () },
    }
}

fn build_bindings<'b>(
    receiver: &'b ReactorReceiver,
    builder_ident: &'b syn::Ident,
) -> impl Iterator<Item = proc_macro2::TokenStream> + 'b {
    receiver.connections.iter().map(move |connection| {
        let from_port = &connection.from;
        let to_port = &connection.to;
        quote! { #builder_ident.bind_port(#from_port, #to_port)?; }
    })
}

impl ToTokens for ReactorReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let builder_ident = format_ident!("__builder");

        let name = &self.ident;
        let state = &self.state;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();

        let fields = self
            .data
            .as_ref()
            .take_struct()
            .expect("Should never be enum")
            .into_iter()
            .map(|f| ReactorField::from(f.clone()));

        let (reaction_fields, rest_fields): (Vec<_>, Vec<_>) =
            fields.clone().partition_map(|field| {
                let args = build_field_args(&field);
                let ident = &field.ident;
                let ty = &field.ty;
                let name = &field.name;
                

                if let ReactorField {
                    inner: ReactorFieldInner::Reaction { .. },
                    ..
                } = field
                {
                    Either::Left(
                        quote! { reactor.#ident = <#ty as ::boomerang::builder::ReactorPart> ::build_part(&mut #builder_ident, #name, #args)?; }
                    )
                } else {
                    Either::Right(
                        quote! { let #ident = <#ty as ::boomerang::builder::ReactorPart> ::build_part(&mut #builder_ident, #name, #args)?; }
                    )
                }
            });

        let field_values = fields.map(|field| match field {
            ReactorField {
                ident,
                inner: ReactorFieldInner::Reaction { .. },
                ..
            } => quote!(#ident: Default::default()),
            _ => {
                let ident = &field.ident;
                quote!(#ident)
            }
        });

        tokens.extend(quote! {
            impl #impl_generics ::boomerang::builder::Reactor for #name #ty_generics #where_clause {
                type State = #state;
                fn build<'__builder>(
                    name: &str,
                    state: Self::State,
                    parent: Option<::boomerang::runtime::ReactorKey>,
                    env: &'__builder mut ::boomerang::builder::EnvBuilder,
                ) -> Result<Self, ::boomerang::builder::BuilderError> {
                    let mut #builder_ident = env.add_reactor(name, parent, state);
                    #(#rest_fields)*
                    let mut reactor = Self {
                        #(#field_values),*
                    };
                    #(#reaction_fields)*
                    Ok(reactor)
                }
            }
        });

        // let bindings = build_bindings(self, &builder_ident);
        // let ident = &self.ident;
        //
        // tokens.extend(quote! {
        // #[automatically_derived]
        // impl ::boomerang::builder::Reactor for #ident
        // {
        // fn build<'__builder, S: ::boomerang::runtime::ReactorState>(
        // name: &str,
        // state: S,
        // parent: Option<::boomerang::runtime::ReactorKey>,
        // env: &'__builder mut ::boomerang::builder::EnvBuilder,
        // ) -> Result<Self, ::boomerang::builder::BuilderError> {
        // let mut #builder_ident = env.add_reactor(name, parent, state);
        // #(#bindings)*
        // #(#reaction_builders)*
        // }
        // }
        // });
    }
}
