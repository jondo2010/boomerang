use std::{collections::HashMap, hash::Hash};

use darling::{
    ast::{self},
    util, FromDeriveInput, FromField, FromMeta,
};
use quote::{quote, ToTokens};
use syn::{parse_quote, Expr, Generics, Ident, Type, TypePath, TypeReference};

const PORT: &'static str = "Port";
const ACTION_REF: &'static str = "ActionRef";
const PHYSICAL_ACTION_REF: &'static str = "PhysicalActionRef";

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
            }) if field_inner_type.to_string() == PORT => {
                match (value.triggers, value.effects, value.uses) {
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
                }
            }

            Type::Reference(TypeReference {
                mutability: Some(_),
                elem,
                ..
            }) if field_inner_type.to_string() == PORT => {
                match (value.triggers, value.effects, value.uses) {
                    (None, None, None) => Ok(Self::FieldDefined {
                        elem: *elem.clone(),
                        triggers: false,
                        effects: true,
                        uses: false,
                        path,
                    }),
                    _ => Err(darling::Error::custom("Invalid Port variant").with_span(&value.ty)),
                }
            }

            Type::Path(TypePath { path: elem, .. })
                if field_inner_type.to_string() == ACTION_REF
                    || field_inner_type.to_string() == PHYSICAL_ACTION_REF =>
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
                    (false, true, false) => quote! {::boomerang::builder::TriggerMode::UsesOnly},
                    (false, false, true) => quote! {::boomerang::builder::TriggerMode::EffectsOnly},

                    // Additional trigger modes for Actions
                    (true, false, false) => {
                        quote! {::boomerang::builder::TriggerMode::TriggersOnly}
                    }
                    (false, true, true) => quote! {::boomerang::builder::TriggerMode::EffectsOnly},
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

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reaction), supports(struct_named, struct_unit))]
pub struct ReactionReceiver {
    ident: Ident,
    generics: Generics,
    data: ast::Data<util::Ignored, ReactionField>,
    /// Connection definitions
    #[darling(default, multiple)]
    pub triggers: Vec<TriggerAttr>,
}

fn extract_path_ident(elem: &Type) -> Option<&Ident> {
    match elem {
        Type::Path(syn::TypePath {
            path: syn::Path { segments, .. },
            ..
        }) => segments.last().map(|segment| &segment.ident),
        Type::Reference(syn::TypeReference { elem, .. }) => extract_path_ident(elem),
        _ => None,
    }
}

struct TriggerInner {
    reaction_ident: Ident,
    initializer_idents: Vec<Ident>,
    action_idents: Vec<Ident>,
    #[allow(dead_code)]
    action_types: Vec<Type>,
    port_idents: Vec<Ident>,
    port_types: Vec<Box<Type>>,
    port_mut_idents: Vec<Ident>,
    port_mut_types: Vec<Box<Type>>,
}

impl TriggerInner {
    fn new(reaction_receiver: &ReactionReceiver) -> darling::Result<Self> {
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
                    if ty.to_string() == PORT {
                        initializer_idents.push(field.ident.clone().unwrap());
                        port_idents.push(field.ident.clone().unwrap());
                        port_types.push(elem.clone());
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
                    if ty.to_string() == PORT {
                        initializer_idents.push(field.ident.clone().unwrap());
                        port_mut_idents.push(field.ident.clone().unwrap());
                        port_mut_types.push(elem.clone());
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
                    if ty.to_string() == ACTION_REF || ty.to_string() == PHYSICAL_ACTION_REF {
                        initializer_idents.push(field.ident.clone().unwrap());
                        action_idents.push(field.ident.clone().unwrap());
                        action_types.push(Type::Path(TypePath {
                            qself: None,
                            path: path.clone(),
                        }));
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

                #(let #port_idents = #port_idents.downcast_ref::<#port_types>()
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

                #(let #port_mut_idents = #port_mut_idents.downcast_mut::<#port_mut_types>()
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

pub struct Reaction {
    ident: Ident,
    generics: Generics,
    fields: Vec<ReactionFieldInner>,
    inner: TriggerInner,
    /// Whether the reaction has a startup trigger
    trigger_startup: bool,
    /// Whether the reaction has a shutdown trigger
    trigger_shutdown: bool,
}

impl TryFrom<ReactionReceiver> for Reaction {
    type Error = darling::Error;

    fn try_from(value: ReactionReceiver) -> Result<Self, Self::Error> {
        let inner = TriggerInner::new(&value)?;

        let fields = value
            .data
            .take_struct()
            .ok_or(darling::Error::unsupported_shape(
                "Only structs are supported",
            ))?;

        let inner_fields = fields
            .into_iter()
            .map(|field| field.try_into())
            .collect::<Result<Vec<ReactionFieldInner>, _>>()?;

        let mut fields_map: HashMap<_, ReactionFieldInner> = inner_fields
            .into_iter()
            .map(|mut field| {
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
                    (path.clone(), field)
                } else {
                    panic!("Unexpected reaction field");
                }
            })
            .collect();

        // Update/apply the struct_fields with any triggers clauses
        for trigger in value.triggers.iter() {
            match trigger {
                TriggerAttr::Action(path) => {
                    fields_map
                        .entry(path.clone())
                        .and_modify(|field| {
                            if let ReactionFieldInner::FieldDefined {
                                ref mut triggers, ..
                            } = field
                            {
                                *triggers = true;
                            } else {
                                panic!("Trigger action path already used");
                            }
                        })
                        .or_insert(ReactionFieldInner::TriggerAction {
                            action: path.clone(),
                        });
                }

                TriggerAttr::Port(path) => {
                    fields_map
                        .entry(path.clone())
                        .and_modify(|field| {
                            if let ReactionFieldInner::FieldDefined {
                                ref mut triggers, ..
                            } = field
                            {
                                *triggers = true;
                            } else {
                                panic!("Trigger port path already used");
                            }
                        })
                        .or_insert(ReactionFieldInner::TriggerPort { port: path.clone() });
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

        Ok(Self {
            ident: value.ident,
            generics: value.generics,
            fields: fields_map.into_values().collect(),
            inner,
            trigger_startup,
            trigger_shutdown,
        })
    }
}

impl ToTokens for Reaction {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;
        let generics = &self.generics;
        let struct_fields = &self.fields;
        let trigger_inner = &self.inner;

        let trigger_startup = if self.trigger_startup {
            quote! {
                let mut __reaction = __reaction.with_action(
                    __startup_action,
                    0,
                    ::boomerang::builder::TriggerMode::TriggersOnly
                )?;
            }
        } else {
            quote! {}
        };

        let trigger_shutdown = if self.trigger_shutdown {
            quote! {
                let mut __reaction = __reaction.with_action(
                    __shutdown_action,
                    0,
                    ::boomerang::builder::TriggerMode::TriggersOnly
                )?;
            }
        } else {
            quote! {}
        };

        tokens.extend(quote! {
            #[automatically_derived]
            impl #generics ::boomerang::builder::Reaction for #ident #generics {
                fn build<'builder>(
                    name: &str,
                    reactor: &Self::Reactor,
                    builder: &'builder mut ::boomerang::builder::ReactorBuilderState,
                ) -> Result<
                    ::boomerang::builder::ReactionBuilderState<'builder>,
                    ::boomerang::builder::BuilderError
                >
                {
                    #trigger_inner
                    let __startup_action = builder.get_startup_action();
                let __shutdown_action = builder.get_shutdown_action();
                    let mut __reaction = builder.add_reaction(name, Box::new(__trigger_inner));
                    #trigger_startup
                    #trigger_shutdown
                    #(#struct_fields;)*
                    Ok(__reaction)
                }
            }
        });
    }
}

#[cfg(feature = "disable")]
#[test]
fn test_reaction() {
    let good_input = r#"
#[derive(Reaction)]
#[reaction(
    reactor = "MyReactor",
    triggers(action = "x"),
    triggers(port = "child.y"),
    triggers(startup)
)]
struct ReactionT<'a> {
    #[reaction(triggers)]
    t: &'a runtime::Action,
    #[reaction(effects, path = "child.y.z")]
    xyc: &'a mut runtime::Port<u32>,
    #[reaction(uses)]
    fff: &'a runtime::Port<()>,
}"#;

    let parsed = syn::parse_str(good_input).unwrap();
    let receiver = ReactionReceiver::from_derive_input(&parsed).unwrap();

    //let fields = receiver.data.take_struct().unwrap();
}
