use crate::{reactor::TriggerAttr, util::MetaList};
use darling::{FromAttributes, FromMeta};
use itertools::Itertools;

mod output;
#[cfg(test)]
mod tests;

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

#[derive(Debug, PartialEq)]
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

#[derive(Debug, PartialEq)]
enum PortAttrs {
    Triggers,
    Effects,
    Uses,
}

#[derive(Debug, PartialEq)]
enum ActionAttrs {
    Triggers,
    Effects,
    TriggersAndEffects,
}

#[derive(Debug, PartialEq)]
struct ReactionArg {
    ident: syn::Ident,
    attr: ArgumentAttr,
    ty: syn::Type,
}

#[derive(Debug)]
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
                    _ => *typed.ty.clone(),
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
    Ok(args)
}

impl ReactionReceiver {
    pub fn new(args: syn::AttributeArgs, itemfn: syn::ItemFn) -> darling::Result<Self> {
        // Parse the attributes on the overall function
        let attr = ReactionAttr::from_list(&args)?;

        Self::from_attr_itemfn(attr, itemfn)
    }

    pub fn from_attr_itemfn(attr: ReactionAttr, mut itemfn: syn::ItemFn) -> darling::Result<Self> {
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
}
