//! Details of implementation for derive macros and attributes

use darling::{ast, util, FromDeriveInput, FromField, FromMeta};
use quote::{quote, ToTokens};

#[derive(Debug, Default)]
pub struct TimerField {
    pub offset: String,
    pub period: String,
}

/// Parse the timer attribute values
/// Example: #[reactor(timer("Duration::from_millis(100)", "Duration::from_millis(1000)"))]
impl FromMeta for TimerField {
    fn from_list(items: &[syn::NestedMeta]) -> darling::Result<Self> {
        match items {
            [syn::NestedMeta::Lit(ref offset), syn::NestedMeta::Lit(ref period)] => {
                let offset = String::from_value(offset)?;
                let period = String::from_value(period)?;

                Ok(TimerField { offset, period })
            }
            _ => Err(darling::Error::unsupported_shape(
                "timer attr should be (offset, period)",
            )),
        }
    }
}

#[derive(Debug, FromField)]
#[darling(attributes(reactor), forward_attrs(doc, cfg, allow))]
pub struct ReactorField {
    pub ident: Option<syn::Ident>,
    attrs: Vec<syn::Attribute>,
    vis: syn::Visibility,
    ty: syn::Type,

    #[darling(default)]
    pub input: Option<bool>,
    #[darling(default)]
    pub output: Option<bool>,
    #[darling(default)]
    pub timer: Option<TimerField>,
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reactor), supports(struct_named))]
pub struct ReactorReceiver {
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    pub data: ast::Data<util::Ignored, ReactorField>,
}

impl ToTokens for ReactorReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;
        let (imp, ty, wher) = self.generics.split_for_impl();
        let fields = self
            .data
            .as_ref()
            .take_struct()
            .expect("Should never be enum")
            .fields;

        tokens.extend(quote! {
            impl #imp #ident #ty #wher {
                fn poo(&self) {
                    println!("Poo!");
                }
            }
        })
    }
}

#[test]
fn test_parse() {
    let good_input = r#"#
    [derive(Reactor, Debug)]
    pub struct Foo {
        //#[reactor(input)]
        //bar: bool,
        //#[reactor(output)]
        //baz: i64,
        #[reactor(timer("Duration::from_millis(100)", "Duration::from_millis(1000)"))]
        foo: u32,
    }"#;
    let parsed = syn::parse_str(good_input).unwrap();
    let receiver = ReactorReceiver::from_derive_input(&parsed).unwrap();
    dbg!(receiver);
}
