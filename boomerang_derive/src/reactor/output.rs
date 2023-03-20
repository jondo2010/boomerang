use darling::ToTokens;
use itertools::{Either, Itertools};
use quote::{format_ident, quote};

use super::ReactorField;
use crate::{
    reactor::ReactorReceiver,
    util::{self, OptionalDuration},
};

pub struct ReactorFieldBuilder<'a, 'b> {
    pub(super) field: &'b ReactorField<'a>,
    pub(super) builder_ident: &'b syn::Ident,
}

impl<'a, 'b> ReactorFieldBuilder<'a, 'b> {
    pub fn new(
        field: &'b ReactorField<'a>,
        builder_ident: &'b syn::Ident,
    ) -> ReactorFieldBuilder<'a, 'b> {
        ReactorFieldBuilder {
            field,
            builder_ident,
        }
    }
}

impl<'a, 'b> ToTokens for ReactorFieldBuilder<'a, 'b> {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            field,
            builder_ident,
        } = self;
        tokens.extend(match field {
            ReactorField::Timer { ident, name, period, offset } => {
                let period = util::duration_quote(period);
                let offset = util::duration_quote(offset);
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

        let fields = self
            .data
            .as_ref()
            .take_struct()
            .unwrap()
            .into_iter()
            .map(ReactorField::from);

        let (reaction_fields, rest_fields): (Vec<_>, Vec<_>) =
            fields.clone().partition_map(|field| match field {
                ReactorField::Reaction { .. } => Either::Left(field),
                _ => Either::Right(field),
            });

        let reaction_builders = reaction_fields
            .iter()
            .map(|field| ReactorFieldBuilder::new(field, &builder_ident).to_token_stream());

        let field_builders = rest_fields
            .iter()
            .map(|field| ReactorFieldBuilder::new(field, &builder_ident).to_token_stream());

        let field_values = fields.map(|field| match field {
            ReactorField::Reaction { ident, .. } => quote!(#ident: Default::default()),
            _ => {
                let ident = field.get_ident();
                quote!(#ident)
            }
        });

        let bindings = build_bindings(self, &builder_ident);
        let ident = &self.ident;
        let state = self.state.clone().unwrap_or_else(|| syn::parse_quote!(()));

        tokens.extend(quote! {
            #[automatically_derived]
            impl ::boomerang::builder::Reactor for #ident
            {
                type State = #state;
                fn build<'__builder>(
                    name: &str,
                    state: Self::State,
                    parent: Option<::boomerang::builder::BuilderReactorKey>,
                    env: &'__builder mut ::boomerang::builder::EnvBuilder,
                ) -> Result<Self, ::boomerang::builder::BuilderError> {
                    let mut #builder_ident = env.add_reactor(name, parent, state);
                    #(#field_builders)*
                    #(#bindings)*
                    let mut reactor = Self {
                        #(#field_values),*
                    };
                    #(#reaction_builders)*
                    Ok(reactor)
                }
            }
        });
    }
}
