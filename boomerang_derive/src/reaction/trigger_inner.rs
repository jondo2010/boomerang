use quote::{quote, ToTokens};
use syn::{GenericParam, Generics, Ident, Type, TypeReference};

use crate::util::extract_path_ident;

use super::{ReactionReceiver, ACTION, ACTION_REF, INPUT_REF, OUTPUT_REF, PHYSICAL_ACTION_REF};

pub struct TriggerInner {
    reaction_ident: Ident,
    reaction_generics: Generics,
    combined_generics: Generics,
    reactor: Type,
    bounds: Vec<GenericParam>,
    initializer_idents: Vec<Ident>,
    action_idents: Vec<Ident>,
    port_idents: Vec<Ident>,
    port_mut_idents: Vec<Ident>,
}

impl TriggerInner {
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
            bounds: reaction_receiver.bounds.clone(),
            initializer_idents,
            action_idents,
            port_idents,
            port_mut_idents,
        })
    }
}

impl ToTokens for TriggerInner {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let reaction_ident = &self.reaction_ident;

        let reaction_generics = self
            .reaction_generics
            .const_params()
            .map(|ty| &ty.ident)
            .chain(self.reaction_generics.type_params().map(|ty| &ty.ident));
        let reaction_generics = quote! { <#(#reaction_generics),*> };

        // We pass through the const and type generics from the reactor to parameters of the trigger function
        let inner_generics = {
            let const_generics = self.combined_generics.const_params();
            let type_params = self.combined_generics.type_params();
            quote! { #(#const_generics),* #(#type_params),* }
        };

        let reactor = &self.reactor;
        let initializer_idents = &self.initializer_idents;
        let action_idents = &self.action_idents;
        let port_idents = &self.port_idents;
        let port_mut_idents = &self.port_mut_idents;

        let actions_len = action_idents.len();
        let actions = if actions_len > 0 {
            quote! {
                let [#(#action_idents,)*]: &mut [&mut ::boomerang::runtime::Action; #actions_len] =
                    ::std::convert::TryInto::try_into(actions)
                        .expect("Unable to destructure actions for reaction");

                #(let #action_idents = (*#action_idents).into(); );*
            }
        } else {
            quote! {}
        };

        let ports = if !port_idents.is_empty() {
            quote! {
                let (#(#port_idents,)*) = ::boomerang::runtime::partition(ports)
                    .expect("Unable to destructure ref ports for reaction");
            }
        } else {
            quote! {}
        };

        let port_muts = if !port_mut_idents.is_empty() {
            quote! {
                let (#(#port_mut_idents,)*) = ::boomerang::runtime::partition_mut(ports_mut)
                    .expect("Unable to destructure mut ports for reaction");
            }
        } else {
            quote! {}
        };

        tokens.extend(quote! {
            #[allow(unused_variables)]
            fn __trigger_inner<'inner, #inner_generics>(
                ctx: &mut ::boomerang::runtime::Context,
                state: &'inner mut dyn ::boomerang::runtime::ReactorState,
                ports: &'inner [::boomerang::runtime::PortRef<'inner>],
                ports_mut: &'inner mut [::boomerang::runtime::PortRefMut<'inner>],
                actions: &'inner mut [&'inner mut ::boomerang::runtime::Action],
            ) {
                let state: &mut <#reactor as ::boomerang::builder::Reactor>::State = state
                    .downcast_mut()
                    .expect("Unable to downcast reactor state");

                #actions
                #ports
                #port_muts

                <#reaction_ident #reaction_generics as ::boomerang::builder::Trigger<#reactor>>::trigger(
                    #reaction_ident { #(#initializer_idents),* },
                    ctx,
                    state
                );
            }
        });
    }
}
