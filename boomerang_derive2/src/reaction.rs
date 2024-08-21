use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
};

use darling::{
    ast::{self, NestedMeta},
    util, FromDeriveInput, FromField, FromMeta,
};
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse_quote, punctuated::Punctuated, token::Dot, Expr, ExprField, Generics, Ident, LitStr,
    PatLit, Path, Type, TypeReference,
};

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

    #[darling(default)]
    triggers: bool,

    #[darling(default)]
    effects: bool,

    #[darling(default)]
    uses: bool,

    path: Option<Expr>,
}

impl ReactionField {
    /// Builds an expression for the path of this ReactionField.
    ///
    /// If `path` is specified (not-none), then this overrides the fallback to using `ident`;
    fn path(&self) -> Expr {
        if let Some(path) = &self.path {
            parse_quote! { reactor.#path }
        } else {
            let ident = self.ident.as_ref().unwrap();
            parse_quote! { reactor.#ident }
        }
    }
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reaction), supports(struct_named))]
pub struct ReactionReceiver {
    ident: Ident,
    generics: Generics,
    data: ast::Data<util::Ignored, ReactionField>,

    /// Connection definitions
    #[darling(default, multiple)]
    //#[darling(default, map = "MetaList::into")]
    pub triggers: Vec<TriggerAttr>,
}

impl ReactionReceiver {
    fn reduce(
        &self,
        // The ident of the Reaction builder
        reaction_ident: &Ident,
        startup_ident: &Ident,
        shutdown_ident: &Ident,
    ) -> impl Iterator<Item = proc_macro2::TokenStream> + '_ {
        let reaction_ident = reaction_ident.clone();

        let mut struct_fields: HashMap<_, ReactionField> = self
            .data
            .as_ref()
            .take_struct()
            .expect("Only structs are supported")
            .fields
            .iter()
            .map(|&field| {
                let path = field.path();
                (path, field.clone())
            })
            .collect();

        let mut startup_shutdown = vec![];

        // Update/apply the struct_fields with any triggers clauses
        for trigger in self.triggers.iter() {
            match trigger {
                TriggerAttr::Startup => {
                    startup_shutdown.push(quote! {
                        let mut #reaction_ident = #reaction_ident.with_trigger_action(#startup_ident, 0)?
                    });
                }
                TriggerAttr::Shutdown => {
                    startup_shutdown.push(quote! {
                        let mut #reaction_ident = #reaction_ident.with_trigger_action(#shutdown_ident, 0)?
                    });
                }
                TriggerAttr::Action(path) => {
                    struct_fields
                        .entry(path.clone())
                        .or_insert(ReactionField {
                            ident: None,
                            ty: parse_quote! { runtime::ActionRef<'a> },
                            triggers: true,
                            effects: false,
                            uses: false,
                            path: Some(path.clone()),
                        })
                        .triggers = true;
                }

                TriggerAttr::Port(path) => {
                    struct_fields
                        .entry(path.clone())
                        .or_insert(ReactionField {
                            ident: None,
                            ty: parse_quote! { runtime::Port<'a, u32> },
                            triggers: true,
                            effects: false,
                            uses: false,
                            path: Some(path.clone()),
                        })
                        .triggers = true;
                }
            }
        }

        let struct_fields =
            struct_fields
                .into_iter()
                .enumerate()
                .map(move |(idx, (path, field))| {
                    if let Type::Reference(TypeReference { elem, .. }) = &field.ty {
                        let triggers = field.triggers;
                        let effects = field.effects;
                        let uses = field.uses;

                        quote! {
                            <#elem as ::boomerang::builder::ReactionField>::build(
                                &mut #reaction_ident,
                                #path,
                                #idx,
                                #triggers,
                                #uses,
                                #effects,
                            )?;
                        }
                    } else {
                        panic!("Only references are supported");
                    }
                });

        startup_shutdown
            .into_iter()
            .chain(struct_fields.into_iter())
    }
}

impl ToTokens for ReactionReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let builder = format_ident!("__reaction");
        let startup = format_ident!("__startup_action");
        let shutdown = format_ident!("__shutdown_action");

        let ident = &self.ident;
        let generics = &self.generics;
        let struct_fields = self.reduce(&builder, &startup, &shutdown);

        tokens.extend(quote! {
            #[automatically_derived]
            impl #generics ::boomerang::builder::Reaction for #ident #generics {
                fn build<'builder>(
                    name: &str,
                    reactor: &Self::BuilderReactor,
                    builder: &'builder mut ::boomerang::builder::ReactorBuilderState,
                ) -> Result<
                    ::boomerang::builder::ReactionBuilderState<'builder>,
                    ::boomerang::builder::BuilderError
                >
                {
                    let __wrapper: Box<dyn ::boomerang::runtime::ReactionFn> = Box::new( move |ctx: &mut ::boomerang::runtime::Context, state: &mut dyn runtime::ReactorState, inputs, outputs, actions: &mut [&mut runtime::Action]| { });
                    let __startup_action = builder.get_startup_action();
                    let __shutdown_action = builder.get_shutdown_action();
                    let mut #builder = builder.add_reaction(name, __wrapper);
                    #(#struct_fields;)*
                    Ok(#builder)
                }

                fn marshall(
                    inputs: &[::boomerang::runtime::IPort],
                    outputs: &mut [::boomerang::runtime::OPort],
                    actions: &mut [&mut ::boomerang::runtime::Action],
                ) -> Self
                {
                    todo!();
                }
            }
        });
    }
}

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

    println!("{}", receiver.to_token_stream());
}
