//! Generate the `FromRefs` implementation for a reaction

use quote::{quote, ToTokens};
use syn::{Generics, Ident, Type, TypeReference};

use crate::util::extract_path_ident;

use super::{ReactionReceiver, ACTION, ACTION_REF, INPUT_REF, OUTPUT_REF, PHYSICAL_ACTION_REF};

pub struct FromDefsImpl {
    reaction_ident: Ident,
    reaction_generics: Generics,
    combined_generics: Generics,
    reactor: Type,
    initializer_idents: Vec<Ident>,
    action_idents: Vec<Ident>,
    port_idents: Vec<Ident>,
    port_mut_idents: Vec<Ident>,
}

impl FromDefsImpl {
    pub fn new(
        reaction_receiver: &ReactionReceiver,
        combined_generics: &Generics,
    ) -> darling::Result<Self> {
        let fields = reaction_receiver
            .data
            .as_ref()
            .take_struct()
            .ok_or_else(|| darling::Error::custom("Only structs are supported"))?;

        let mut initializer_idents = vec![];
        let mut action_idents = vec![];
        let mut port_idents = vec![];
        let mut port_mut_idents = vec![];

        for field in fields.iter() {
            let field_inner_type = extract_path_ident(&field.ty).ok_or_else(|| {
                darling::Error::custom("Unable to extract path ident ").with_span(&field.ty)
            })?;

            match &field.ty {
                Type::Reference(TypeReference {
                    mutability: None,
                    elem,
                    ..
                }) => {
                    let ty = extract_path_ident(elem.as_ref()).ok_or_else(|| {
                        darling::Error::custom(format!(
                            "Unable to extract path ident for {:?}",
                            elem
                        ))
                    })?;
                    if *ty == ACTION {
                        initializer_idents.push(field.ident.clone().unwrap());
                        action_idents.push(field.ident.clone().unwrap());
                    } else {
                        return Err(darling::Error::custom(format!(
                            "Unexpected ref type: {:?}",
                            ty
                        )));
                    }
                }

                Type::Reference(TypeReference {
                    mutability: Some(_),
                    elem,
                    ..
                }) => {
                    let ty = extract_path_ident(elem.as_ref()).ok_or_else(|| {
                        darling::Error::custom(format!(
                            "Unable to extract path ident for {:?}",
                            elem
                        ))
                    })?;
                    if *ty == ACTION {
                        initializer_idents.push(field.ident.clone().unwrap());
                        action_idents.push(field.ident.clone().unwrap());
                    } else {
                        return Err(darling::Error::custom(format!(
                            "Unexpected mut ref type: {:?}",
                            ty
                        )));
                    }
                }

                Type::Path(_) | Type::Array(_) => match field_inner_type.to_string().as_ref() {
                    INPUT_REF => {
                        initializer_idents.push(field.ident.clone().unwrap());
                        port_idents.push(field.ident.clone().unwrap());
                    }
                    OUTPUT_REF => {
                        initializer_idents.push(field.ident.clone().unwrap());
                        port_mut_idents.push(field.ident.clone().unwrap());
                    }
                    ACTION_REF | PHYSICAL_ACTION_REF => {
                        initializer_idents.push(field.ident.clone().unwrap());
                        action_idents.push(field.ident.clone().unwrap());
                    }

                    _ => {
                        return Err(darling::Error::custom("Unexpected Reaction member")
                            .with_span(&field.ty));
                    }
                },

                _ => {
                    return Err(darling::Error::custom(format!(
                        "Not handling {:?}",
                        field.ty
                    )));
                }
            }
        }

        Ok(Self {
            reaction_ident: reaction_receiver.ident.clone(),
            reaction_generics: reaction_receiver.generics.clone(),
            combined_generics: combined_generics.clone(),
            reactor: reaction_receiver.reactor.clone(),
            initializer_idents,
            action_idents,
            port_idents,
            port_mut_idents,
        })
    }
}

impl ToTokens for FromDefsImpl {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let reaction_ident = &self.reaction_ident;

        let reaction_generics = self
            .reaction_generics
            .const_params()
            .map(|ty| &ty.ident)
            .chain(self.reaction_generics.type_params().map(|ty| &ty.ident));
        let reaction_generics = quote! { #(#reaction_generics),* };

        let lifetimes = self.reaction_generics.lifetimes().collect::<Vec<_>>();
        assert!(lifetimes.len() <= 1, "Expected at most one lifetime");

        let anon_lt = if lifetimes.is_empty() {
            reaction_generics.clone()
        } else {
            quote! { '_, #reaction_generics }
        };
        let marker_lt = if lifetimes.is_empty() {
            reaction_generics.clone()
        } else {
            quote! { 's, #reaction_generics }
        };

        // We pass through the const and type generics from the reactor to parameters of the trigger function
        let inner_generics = {
            //let const_generics = self.combined_generics.const_params();
            //let type_params = self.combined_generics.type_params();
            let const_generics = self.reaction_generics.const_params();
            let type_params = self.reaction_generics.type_params();
            quote! { #(#const_generics),* #(#type_params),* }
        };

        let reactor = &self.reactor;
        let initializer_idents = &self.initializer_idents;
        let action_idents = &self.action_idents;
        let port_idents = &self.port_idents;
        let port_mut_idents = &self.port_mut_idents;

        let actions = (!action_idents.is_empty()).then(|| {
            quote! {
                let (#(#action_idents,)*) = actions.partition_mut()
                    .expect("Unable to destructure actions for reaction");
            }
        });

        let ports = (!port_idents.is_empty()).then(|| {
            quote! {
                let (#(#port_idents,)*) = ports.partition()
                    .expect("Unable to destructure ref ports for reaction");
            }
        });

        let port_muts = (!port_mut_idents.is_empty()).then(|| {
            quote! {
                let (#(#port_mut_idents,)*) = ports_mut.partition_mut()
                    .expect("Unable to destructure mut ports for reaction");
            }
        });

        tokens.extend(quote! {
            #[automatically_derived]
            impl <#inner_generics> ::boomerang::runtime::FromRefs for #reaction_ident <#anon_lt> {
                type Marker<'s> = #reaction_ident <#marker_lt>;

                #[allow(unused_variables)]
                fn from_refs<'store>(
                    ports: ::boomerang::runtime::Refs<'store, dyn ::boomerang::runtime::BasePort>,
                    ports_mut: ::boomerang::runtime::RefsMut<'store, dyn ::boomerang::runtime::BasePort>,
                    actions: ::boomerang::runtime::RefsMut<'store, ::boomerang::runtime::Action>,
                ) -> Self::Marker<'store> {
                    #actions
                    #ports
                    #port_muts

                    #reaction_ident { #(#initializer_idents),* }
                }
            }
        });
    }
}
