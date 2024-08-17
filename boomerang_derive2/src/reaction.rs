use darling::{
    ast::{self, NestedMeta},
    util, FromDeriveInput, FromField, FromMeta,
};
use quote::{format_ident, quote, ToTokens};
use syn::{Generics, Ident, Path, Type, TypeReference};

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum TriggerAttr {
    Startup,
    Shutdown,
    Action(Ident),
    Port(Path),
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

#[derive(Debug, FromField)]
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

    path: Option<Ident>,
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reaction), supports(struct_named))]
pub struct ReactionReceiver {
    ident: Ident,
    generics: Generics,
    data: ast::Data<util::Ignored, ReactionField>,
    /// The reactor owning this reaction
    reactor: Type,

    /// Connection definitions
    #[darling(default, multiple)]
    //#[darling(default, map = "MetaList::into")]
    pub triggers: Vec<TriggerAttr>,
}

impl ToTokens for ReactionReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;
        let generics = &self.generics;
        let reactor = &self.reactor;
        let _ = self.triggers.iter().map(|trigger| match trigger {
            TriggerAttr::Startup => quote! { .with_trigger_action(__startup_action, 0)},
            TriggerAttr::Shutdown => quote! { .with_trigger_action(__shutdown_action, 0)},
            TriggerAttr::Action(action) => quote! { .with_trigger_action(reactor.#action, 0)},
            TriggerAttr::Port(port) => quote! { .with_trigger_port(reactor.#port, 0) },
        });

        let struct_fields = self
            .data
            .as_ref()
            .take_struct()
            .expect("Only structs are supported")
            .fields
            .into_iter()
            .enumerate()
            .map(|(idx, field)| {
                if let Type::Reference(TypeReference { elem, .. }) = &field.ty {
                    let key = field
                        .path
                        .as_ref()
                        .or(field.ident.as_ref())
                        .expect("No key found");
                    let triggers = field.triggers;
                    let effects = field.effects | field.uses;

                    quote! {
                        let __reaction = <#elem as ::boomerang::builder::ReactionField>::build(
                            __reaction,
                            reactor.#key,
                            #idx,
                            #triggers,
                            #effects
                        )?
                    }
                } else {
                    panic!("Only references are supported");
                }
            });

        tokens.extend(quote! {
            #[automatically_derived]
            impl #generics ::boomerang::builder::Reaction for #ident #generics {
                type BuilderReactor = #reactor;

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
                    let __reaction = builder.add_reaction(name, __wrapper);
                    #(#struct_fields;)*
                    Ok(__reaction)
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
    triggers(startup)
)]
struct ReactionT<'a> {
    #[reaction(triggers)]
    t: &'a runtime::Action,
    #[reaction(effects, path = "c")]
    xyc: &'a mut runtime::Port<u32>,
    #[reaction(uses)]
    fff: &'a runtime::Port<()>,
}"#;

    let parsed = syn::parse_str(good_input).unwrap();
    let receiver = ReactionReceiver::from_derive_input(&parsed).unwrap();

    dbg!(&receiver.triggers);

    //let fields = receiver.data.take_struct().unwrap();

    println!("{}", receiver.to_token_stream());
}
