use quote::{quote, ToTokens};
use syn::{Ident, Type, TypePath, TypeReference};

use crate::util::extract_path_ident;

use super::{ReactionReceiver, ACTION, ACTION_REF, INPUT_REF, OUTPUT_REF, PHYSICAL_ACTION_REF};

pub struct TriggerInner {
    reaction_ident: Ident,
    initializer_idents: Vec<Ident>,
    action_idents: Vec<Ident>,
    #[allow(dead_code)]
    action_types: Vec<Type>,
    port_idents: Vec<Ident>,
    port_types: Vec<Type>,
    port_mut_idents: Vec<Ident>,
    port_mut_types: Vec<Type>,
}

impl TriggerInner {
    pub fn new(reaction_receiver: &ReactionReceiver) -> darling::Result<Self> {
        let fields = reaction_receiver
            .data
            .as_ref()
            .take_struct()
            .ok_or_else(|| darling::Error::custom("Only structs are supported"))?;

        let mut initializer_idents = vec![];
        let mut action_idents = vec![];
        let mut action_types = vec![];
        let mut port_idents = vec![];
        let mut port_types = vec![];
        let mut port_mut_idents = vec![];
        let mut port_mut_types = vec![];

        for field in fields.iter() {
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

                Type::Path(TypePath { path, .. }) => {
                    let ty = extract_path_ident(&field.ty).ok_or_else(|| {
                        darling::Error::custom(format!(
                            "Unable to extract path ident for {:?}",
                            field.ty
                        ))
                    })?;

                    if *ty == INPUT_REF {
                        initializer_idents.push(field.ident.clone().unwrap());
                        port_idents.push(field.ident.clone().unwrap());
                        port_types.push(field.ty.clone());
                    } else if *ty == OUTPUT_REF {
                        initializer_idents.push(field.ident.clone().unwrap());
                        port_mut_idents.push(field.ident.clone().unwrap());
                        port_mut_types.push(field.ty.clone());
                    } else if *ty == ACTION_REF || *ty == PHYSICAL_ACTION_REF {
                        initializer_idents.push(field.ident.clone().unwrap());
                        action_idents.push(field.ident.clone().unwrap());
                        action_types.push(field.ty.clone());
                    } else {
                        return Err(
                            darling::Error::custom("Unexpected Reaction member").with_span(&ty)
                        );
                    }
                }

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
            initializer_idents,
            action_idents,
            action_types,
            port_idents,
            port_types,
            port_mut_idents,
            port_mut_types,
        })
    }
}

impl ToTokens for TriggerInner {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let reaction_ident = &self.reaction_ident;
        let initializer_idents = &self.initializer_idents;
        let action_idents = &self.action_idents;
        let port_idents = &self.port_idents;
        let port_types = &self.port_types;
        let port_mut_idents = &self.port_mut_idents;
        let port_mut_types = &self.port_mut_types;

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

        let ports_len = port_idents.len();
        let ports = if ports_len > 0 {
            quote! {
                let [#(#port_idents,)*]: &[::boomerang::runtime::PortRef; #ports_len] =
                    ::std::convert::TryInto::try_into(ports)
                        .expect("Unable to destructure ref ports for reaction");

                #(let #port_idents = #port_idents.downcast_ref::<::boomerang::runtime::Port<_>>()
                    .map(Into::into)
                    .expect("Wrong Port type for reaction"); );*
            }
        } else {
            quote! {}
        };

        let port_muts_len = port_mut_idents.len();
        let port_muts = if port_muts_len > 0 {
            quote! {
                let [#(#port_mut_idents,)*]: &mut [::boomerang::runtime::PortRefMut; #port_muts_len] =
                    ::std::convert::TryInto::try_into(ports_mut)
                        .expect("Unable to destructure mut ports for reaction");

                #(let #port_mut_idents = #port_mut_idents.downcast_mut::<::boomerang::runtime::Port<_>>()
                    .map(Into::into)
                    .expect("Wrong Port type for reaction"); );*
            }
        } else {
            quote! {}
        };

        tokens.extend(quote! {
        #[allow(unused_variables)]
        fn __trigger_inner(
            ctx: &mut ::boomerang::runtime::Context,
            state: &mut dyn ::boomerang::runtime::ReactorState,
            ports: &[::boomerang::runtime::PortRef],
            ports_mut: &mut [::boomerang::runtime::PortRefMut],
            actions: &mut [&mut ::boomerang::runtime::Action],
        ) {
            let state: &mut <<#reaction_ident as Trigger>::Reactor as ::boomerang::builder::Reactor>::State =
                state
                    .downcast_mut()
                    .expect("Unable to downcast reactor state");

            #actions
            #ports
            #port_muts

            #reaction_ident {
                #(#initializer_idents),*
            }.trigger(ctx, state);
        }
    });
    }
}
