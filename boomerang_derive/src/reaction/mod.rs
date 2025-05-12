use std::{collections::HashMap, hash::Hash};

use darling::{
    ast::{self},
    util, FromDeriveInput, FromField, FromMeta,
};
use quote::{quote, ToTokens};
use syn::{Expr, GenericParam, Generics, Ident, Type};

mod from_defs;
mod reaction_field_inner;

use from_defs::FromDefsImpl;
use reaction_field_inner::ReactionFieldInner;

const INPUT_REF: &str = "InputRef";
const OUTPUT_REF: &str = "OutputRef";
const ACTION: &str = "Action";
const ACTION_REF: &str = "ActionRef";
const ASYNC_ACTION_REF: &str = "AsyncActionRef";

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum TriggerAttr {
    Startup,
    Shutdown,
    Action(Expr),
    Port(Expr),
}

impl FromMeta for TriggerAttr {
    fn from_meta(item: &syn::Meta) -> darling::Result<Self> {
        (match *item {
            syn::Meta::Path(ref path) => path.segments.first().map_or_else(
                || Err(darling::Error::unsupported_shape("something wierd")),
                |path| match path.ident.to_string().as_ref() {
                    "startup" => Ok(TriggerAttr::Startup),
                    "shutdown" => Ok(TriggerAttr::Shutdown),
                    __other => Err(darling::Error::unknown_field_with_alts(
                        __other,
                        &["startup", "shutdown"],
                    )
                    .with_span(&path.ident)),
                },
            ),
            syn::Meta::List(ref value) => {
                let meta: syn::Meta = syn::parse2(value.tokens.clone())?;
                Self::from_meta(&meta)
            }
            syn::Meta::NameValue(ref value) => value
                .path
                .segments
                .first()
                .map(|path| match path.ident.to_string().as_ref() {
                    "action" => {
                        let value = darling::FromMeta::from_expr(&value.value)?;
                        Ok(TriggerAttr::Action(value))
                    }
                    "port" => {
                        let value = darling::FromMeta::from_expr(&value.value)?;
                        Ok(TriggerAttr::Port(value))
                    }
                    __other => Err(darling::Error::unknown_field_with_alts(
                        __other,
                        &["action", "timer", "port"],
                    )
                    .with_span(&path.ident)),
                })
                .expect("oopsie"),
        })
        .map_err(|e| e.with_span(item))
    }

    fn from_string(value: &str) -> darling::Result<Self> {
        let value = darling::FromMeta::from_string(value)?;
        Ok(TriggerAttr::Port(value))
    }
}

#[derive(Clone, Debug, FromField)]
#[darling(attributes(reaction), forward_attrs(doc, cfg, allow))]
pub struct ReactionField {
    ident: Option<Ident>,
    ty: Type,
    triggers: Option<bool>,
    effects: Option<bool>,
    uses: Option<bool>,
    path: Option<Expr>,
}

fn parse_bound(item: &syn::Meta) -> Result<syn::GenericParam, darling::Error> {
    match item {
        syn::Meta::NameValue(syn::MetaNameValue { value, .. }) => match value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit_str),
                ..
            }) => syn::parse_str(lit_str.value().as_str())
                .map_err(|e| darling::Error::custom(format!("Failed to parse bound: {}", e))),

            _ => Err(darling::Error::unsupported_shape(
                "Only string literals are supported",
            )),
        },
        _ => Err(darling::Error::unsupported_shape(
            "Only name value pairs are supported",
        )),
    }
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reaction), supports(struct_named, struct_unit))]
pub struct ReactionReceiver {
    ident: Ident,
    generics: Generics,
    data: ast::Data<util::Ignored, ReactionField>,

    /// Type of the reactor
    reactor: syn::Type,

    #[darling(default, multiple, rename = "bound", with = parse_bound)]
    bounds: Vec<syn::GenericParam>,

    /// Connection definitions
    #[darling(default, multiple)]
    triggers: Vec<TriggerAttr>,
}

pub struct Reaction {
    ident: Ident,
    generics: Generics,
    combined_generics: Generics,
    reactor: syn::Type,
    fields: Vec<ReactionFieldInner>,
    fromdefs: FromDefsImpl,
    /// Whether the reaction has a startup trigger
    trigger_startup: bool,
    /// Whether the reaction has a shutdown trigger
    trigger_shutdown: bool,
}

impl TryFrom<ReactionReceiver> for Reaction {
    type Error = darling::Error;

    fn try_from(value: ReactionReceiver) -> Result<Self, Self::Error> {
        // Combine the bounds with the generics
        let mut combined_generics = value.generics.clone();
        combined_generics
            .params
            .extend(value.bounds.iter().cloned().map(GenericParam::from));

        let fromdefs = FromDefsImpl::new(&value, &combined_generics)?;

        let fields = value
            .data
            .take_struct()
            .ok_or(darling::Error::unsupported_shape(
                "Only structs are supported",
            ))?;

        let inner_fields: Vec<ReactionFieldInner> = fields
            .into_iter()
            .map(TryFrom::try_from)
            .collect::<Result<_, _>>()?;

        let mut fields_map: HashMap<_, (usize, ReactionFieldInner)> = inner_fields
            .into_iter()
            .enumerate()
            .map(|(idx, mut field)| {
                if let ReactionFieldInner::FieldDefined {
                    ref mut uses,
                    triggers,
                    path,
                    ..
                } = &mut field
                {
                    // If the field is a trigger, then it implies use
                    if *triggers {
                        *uses = true;
                    }
                    (path.clone(), (idx, field))
                } else {
                    panic!("Unexpected reaction field");
                }
            })
            .collect();

        let mut last_idx = fields_map.len();

        // Update/apply the struct_fields with any triggers clauses
        for trigger in value.triggers.iter() {
            match trigger {
                TriggerAttr::Action(path) => {
                    fields_map
                        .entry(path.clone())
                        .and_modify(|(_idx, field)| {
                            if let ReactionFieldInner::FieldDefined {
                                ref mut triggers, ..
                            } = field
                            {
                                *triggers = true;
                            } else {
                                panic!("Trigger action path already used");
                            }
                        })
                        .or_insert_with(|| {
                            last_idx += 1;
                            (
                                last_idx,
                                ReactionFieldInner::TriggerAction {
                                    action: path.clone(),
                                },
                            )
                        });
                }

                TriggerAttr::Port(path) => {
                    fields_map
                        .entry(path.clone())
                        .and_modify(|(_idx, field)| {
                            if let ReactionFieldInner::FieldDefined {
                                ref mut triggers, ..
                            } = field
                            {
                                *triggers = true;
                            } else {
                                panic!("Trigger port path already used");
                            }
                        })
                        .or_insert_with(|| {
                            last_idx += 1;
                            (
                                last_idx,
                                ReactionFieldInner::TriggerPort { port: path.clone() },
                            )
                        });
                }

                _ => {}
            }
        }

        let trigger_startup = value
            .triggers
            .iter()
            .any(|t| matches!(t, TriggerAttr::Startup));
        let trigger_shutdown = value
            .triggers
            .iter()
            .any(|t| matches!(t, TriggerAttr::Shutdown));

        let mut idx_fields: Vec<_> = fields_map.into_values().collect();
        idx_fields.sort_by_key(|(idx, _)| *idx);
        let fields = idx_fields.into_iter().map(|(_, field)| field).collect();

        Ok(Self {
            ident: value.ident,
            generics: value.generics,
            combined_generics,
            reactor: value.reactor,
            fields,
            fromdefs,
            trigger_startup,
            trigger_shutdown,
        })
    }
}

impl ToTokens for Reaction {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;
        let reactor = &self.reactor;
        let struct_fields = &self.fields;
        let fromdefs_impl = &self.fromdefs;

        // We use impl_generics from `combined_generics` to allow additional bounds to be added, but
        // type and where come from the original generics
        let (impl_generics, _, _) = self.combined_generics.split_for_impl();
        let (_, type_generics, where_clause) = self.generics.split_for_impl();
        let inner_type_generics = {
            let g = self
                .generics
                .const_params()
                .map(|ty| &ty.ident)
                .chain(self.generics.type_params().map(|ty| &ty.ident));
            quote! { ::<#(#g),*> }
        };

        let trigger_startup = self.trigger_startup.then(|| {
            quote! {
                let mut __reaction = __reaction.with_action(
                    __startup_action,
                    0,
                    ::boomerang::builder::TriggerMode::TriggersOnly
                )?;
            }
        });

        let trigger_shutdown = self.trigger_shutdown.then(|| {
            quote! {
                let mut __reaction = __reaction.with_action(
                    __shutdown_action,
                    0,
                    ::boomerang::builder::TriggerMode::TriggersOnly
                )?;
            }
        });

        tokens.extend(quote! {
            #fromdefs_impl

            #[automatically_derived]
            impl #impl_generics ::boomerang::builder::Reaction<#reactor> for #ident #type_generics #where_clause {
                fn build<'builder, S: runtime::ReactorData>(
                    name: &str,
                    reactor: &#reactor,
                    builder: &'builder mut ::boomerang::builder::ReactorBuilderState<S>,
                ) -> Result<
                    ::boomerang::builder::ReactionBuilderState<'builder>,
                    ::boomerang::builder::BuilderError
                >
                {
                    use ::boomerang::builder::DeferedBuild;

                    let __startup_action = builder.get_startup_action();
                    let __shutdown_action = builder.get_shutdown_action();

                    let mut __reaction = {
                        let wrapper = ::boomerang::runtime::ReactionAdapter::<
                            #ident #inner_type_generics,
                            <#reactor as ::boomerang::builder::Reactor>::State
                        >::default();
                        builder.add_reaction(name, wrapper.defer())
                    };

                    #trigger_startup
                    #trigger_shutdown
                    #(#struct_fields;)*
                    Ok(__reaction)
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use syn::{parse_quote, DeriveInput};

    use super::*;

    #[test]
    fn test_struct_attrs() {
        let input = r#"
#[derive(Reaction)]
#[reaction(
    reactor = "Inner::Count<T>",
    bound = "T: runtime::ReactorData",
    bound = "const N: usize",
    triggers(action = "x"),
    triggers(port = "child.y"),
    triggers(startup),
    triggers(shutdown),
)]
struct ReactionT;"#;
        let parsed: DeriveInput = syn::parse_str(input).unwrap();
        let receiver = ReactionReceiver::from_derive_input(&parsed).unwrap();
        assert_eq!(receiver.reactor, parse_quote! {Inner::Count<T>});
        assert_eq!(
            receiver.bounds,
            vec![
                parse_quote! {T: runtime::ReactorData},
                parse_quote! {const N: usize}
            ]
        );
        assert_eq!(
            receiver.triggers.iter().collect::<Vec<_>>(),
            vec![
                &TriggerAttr::Action(parse_quote! {x}),
                &TriggerAttr::Port(parse_quote! {child.y}),
                &TriggerAttr::Startup,
                &TriggerAttr::Shutdown
            ]
        );
    }

    #[test]
    fn test_port_fields() {
        let input = r#"
#[derive(Reaction)]
#[reaction(reactor = "Foo")]
struct ReactionT<'a> {
    ref_port: runtime::InputRef<'a, ()>,
    mut_port: runtime::OutputRef<'a, ()>,
    #[reaction(uses)]
    uses_only_port: runtime::InputRef<'a, ()>,
    #[reaction(path = "child.y.z")]
    renamed_port: runtime::OutputRef<'a, u32>,
}"#;

        let parsed = syn::parse_str(input).unwrap();
        let receiver = ReactionReceiver::from_derive_input(&parsed).unwrap();
        let reaction = Reaction::try_from(receiver).unwrap();
        assert_eq!(
            reaction.fields[0],
            ReactionFieldInner::FieldDefined {
                elem: parse_quote! {runtime::InputRef<'a, ()>},
                triggers: true,
                effects: false,
                uses: true,
                path: parse_quote! {ref_port},
            },
        );
        assert_eq!(
            reaction.fields[1],
            ReactionFieldInner::FieldDefined {
                elem: parse_quote! {runtime::OutputRef<'a, ()>},
                triggers: false,
                effects: true,
                uses: false,
                path: parse_quote! {mut_port},
            },
        );
        assert_eq!(
            reaction.fields[2],
            ReactionFieldInner::FieldDefined {
                elem: parse_quote! {runtime::InputRef<'a, ()>},
                triggers: false,
                effects: false,
                uses: true,
                path: parse_quote! {uses_only_port},
            },
        );
        assert_eq!(
            reaction.fields[3],
            ReactionFieldInner::FieldDefined {
                elem: parse_quote! {runtime::OutputRef<'a, u32>},
                triggers: false,
                effects: true,
                uses: false,
                path: parse_quote! {child.y.z},
            }
        );
    }

    #[test]
    fn test_action_fields() {
        let input = r#"
#[derive(Reaction)]
#[reaction(reactor = "Foo")]
struct ReactionT<'a> {
    #[reaction(triggers)]
    raw_action: &'a runtime::Action,
}"#;
        let parsed = syn::parse_str(input).unwrap();
        let receiver = ReactionReceiver::from_derive_input(&parsed).unwrap();
        let reaction = Reaction::try_from(receiver).unwrap();
        assert_eq!(
            reaction.fields[0],
            ReactionFieldInner::FieldDefined {
                elem: parse_quote! {runtime::Action},
                triggers: true,
                effects: false,
                uses: true,
                path: parse_quote! {raw_action},
            }
        );
    }
}
