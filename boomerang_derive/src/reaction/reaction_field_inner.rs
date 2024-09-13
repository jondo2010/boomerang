use quote::{quote, ToTokens};
use syn::{parse_quote, Expr, Type, TypePath, TypeReference};

use crate::util::extract_path_ident;

use super::{ReactionField, ACTION, ACTION_REF, PHYSICAL_ACTION_REF, PORT};

#[derive(Debug, PartialEq, Eq)]
pub enum ReactionFieldInner {
    /// The definition came from a field on the struct.
    FieldDefined {
        elem: syn::Type,
        triggers: bool,
        effects: bool,
        uses: bool,
        path: Expr,
    },
    /// The definition came from a #[reaction(triggers(port = "..."))] attribute.
    TriggerPort { port: Expr },
    /// The definition came from a #[reaction(triggers(action = "..."))] attribute.
    TriggerAction { action: Expr },
}

impl TryFrom<ReactionField> for ReactionFieldInner {
    type Error = darling::Error;

    fn try_from(value: ReactionField) -> Result<Self, Self::Error> {
        // Builds an expression for the path of this ReactionField.
        // If `path` is specified (not-none), then this overrides the fallback to using `ident`;
        let path = value
            .path
            .or_else(|| value.ident.map(|i| parse_quote!(#i)))
            .ok_or_else(|| darling::Error::custom("Field must have either a path or an ident"))?;

        let field_inner_type = extract_path_ident(&value.ty).ok_or_else(|| {
            darling::Error::custom("Unable to extract path ident ").with_span(&value.ty)
        })?;

        match &value.ty {
            // For ports, only 3 variants are valid:
            // - &runtime::Port<T>, corresponds to TriggerMode::TriggersAndUses
            // - &runtime::Port<T> with #[reaction(uses)], corresponds to TriggerMode::UsesOnly
            // - &mut runtime::Port<T> corresponds to TriggerMode::EffectsOnly
            Type::Reference(TypeReference {
                mutability: None,
                elem,
                ..
            }) if *field_inner_type == PORT => match (value.triggers, value.effects, value.uses) {
                (None, None, None) => Ok(Self::FieldDefined {
                    elem: *elem.clone(),
                    triggers: true,
                    effects: false,
                    uses: true,
                    path,
                }),
                (None, None, Some(true)) => Ok(Self::FieldDefined {
                    elem: *elem.clone(),
                    triggers: false,
                    effects: false,
                    uses: true,
                    path,
                }),
                _ => Err(darling::Error::custom(
                    "Invalid Port field. Possible attributes are 'use'",
                )
                .with_span(&value.ty)),
            },

            Type::Reference(TypeReference {
                mutability: Some(_),
                elem,
                ..
            }) if *field_inner_type == PORT => match (value.triggers, value.effects, value.uses) {
                (None, None, None) => Ok(Self::FieldDefined {
                    elem: *elem.clone(),
                    triggers: false,
                    effects: true,
                    uses: false,
                    path,
                }),
                _ => Err(darling::Error::custom("Invalid Port variant").with_span(&value.ty)),
            },

            Type::Reference(TypeReference {
                mutability, elem, ..
            }) if *field_inner_type == ACTION => Ok(Self::FieldDefined {
                elem: *elem.clone(),
                triggers: value.triggers.unwrap_or(false),
                effects: value.effects.unwrap_or(mutability.is_some()),
                uses: value.uses.unwrap_or(true),
                path,
            }),

            Type::Path(TypePath { path: elem, .. })
                if *field_inner_type == ACTION_REF || *field_inner_type == PHYSICAL_ACTION_REF =>
            {
                Ok(Self::FieldDefined {
                    elem: syn::Type::Path(TypePath {
                        qself: None,
                        path: elem.clone(),
                    }),
                    triggers: value.triggers.unwrap_or(false),
                    effects: value.effects.unwrap_or(false),
                    uses: value.uses.unwrap_or(true),
                    path,
                })
            }

            _ => Err(darling::Error::custom("Unexpected field type").with_span(&value.ty)),
        }
    }
}

impl ToTokens for ReactionFieldInner {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::FieldDefined {
                path,
                elem,
                triggers,
                uses,
                effects,
            } => {
                let trigger_mode = match (triggers, uses, effects) {
                    (true, true, false) => {
                        quote! {::boomerang::builder::TriggerMode::TriggersAndUses}
                    }
                    (false, true, false) => {
                        quote! {::boomerang::builder::TriggerMode::UsesOnly}
                    }
                    (false, false, true) => {
                        quote! {::boomerang::builder::TriggerMode::EffectsOnly}
                    }

                    // Additional trigger modes for Actions
                    (true, false, false) => {
                        quote! {::boomerang::builder::TriggerMode::TriggersOnly}
                    }
                    (false, true, true) => {
                        quote! {::boomerang::builder::TriggerMode::EffectsOnly}
                    }
                    (true, _, true) => {
                        quote! {::boomerang::builder::TriggerMode::TriggersAndEffects}
                    }
                    _ => panic!("Invalid trigger mode: {:?}", (triggers, uses, effects)),
                };

                tokens.extend(quote! {
                    <#elem as ::boomerang::builder::ReactionField>::build(
                        &mut __reaction,
                        reactor.#path.into(),
                        0,
                        #trigger_mode,
                    )?;
                });
            }
            Self::TriggerPort { port } => {
                tokens.extend(quote! {
                __reaction.add_port(reactor.#port.into(), 0, ::boomerang::builder::TriggerMode::TriggersOnly)?;
            });
            }
            Self::TriggerAction { action } => {
                tokens.extend(quote! {
                __reaction.add_action(reactor.#action.into(), 0, ::boomerang::builder::TriggerMode::TriggersOnly)?;
            });
            }
        }
    }
}
