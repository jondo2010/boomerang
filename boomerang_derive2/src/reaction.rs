use darling::{ast, util, FromDeriveInput, FromField};
use quote::{quote, ToTokens};
use syn::{Ident, Type};

#[derive(Debug, FromField)]
#[darling(attributes(reaction), forward_attrs(doc, cfg, allow))]
pub struct ReactionField {
    ident: Option<Ident>,
    ty: Type,

    #[darling(default)]
    triggers: bool,
    #[darling(default)]
    effects: bool,

    path: Option<String>,
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reaction), supports(struct_named))]
pub struct ReactionReceiver {
    ident: Ident,
    generics: syn::Generics,
    attrs: Vec<syn::Attribute>,
    data: ast::Data<util::Ignored, ReactionField>,
    /// The reactor owning this reaction
    reactor: syn::Type,
}

impl quote::ToTokens for ReactionReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;
        let generics = &self.generics;
        let reactor = &self.reactor;

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
                    todo!();
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
#[reaction(reactor = "MyReactor")]
struct ReactionT<'a> {
    #[reaction(triggers)]
    t: &'a runtime::Action,
    #[reaction(effects, path = "c")]
    xyc: &'a mut runtime::Port<u32>,
}"#;

    let parsed = syn::parse_str(good_input).unwrap();
    let receiver = ReactionReceiver::from_derive_input(&parsed).unwrap();

    if let ast::Data::Struct(fields) = receiver.data {
        for field in fields {
            dbg!(field);
        }
    }

    //println!("{}", receiver.to_token_stream());
}
