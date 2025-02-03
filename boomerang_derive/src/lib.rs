use darling::FromDeriveInput;
use quote::{quote, ToTokens};

mod reaction;
mod reactor;
mod reactor_macro;
mod util;

#[proc_macro_derive(Reaction, attributes(reaction))]
pub fn derive_reaction(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);
    let reaction: Result<reaction::Reaction, _> =
        reaction::ReactionReceiver::from_derive_input(&ast).and_then(TryFrom::try_from);

    match reaction {
        Ok(receiver) => receiver.to_token_stream(),
        Err(err) => err.write_errors(),
    }
    .into()
}

#[proc_macro_derive(Reactor, attributes(reactor))]
pub fn derive_reactor(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);
    let receiver: Result<reactor::Reactor, _> =
        reactor::ReactorReceiver::from_derive_input(&ast).and_then(TryFrom::try_from);

    match receiver {
        Ok(receiver) => receiver.to_token_stream(),
        Err(err) => err.write_errors(),
    }
    .into()
}

mod reactor2 {
    use attribute_derive::FromAttr;
    use proc_macro_error2::abort;
    use quote::ToTokens;
    use syn::{
        parse::Parse, spanned::Spanned, FnArg, Ident, ItemFn, Meta, Pat, PatIdent, ReturnType,
        Type, Visibility,
    };

    use crate::util::convert_from_snake_case;

    #[derive(Debug)]
    pub struct Model {
        //docs: Docs,
        //unknown_attrs: UnknownAttrs,
        vis: Visibility,
        name: Ident,
        pub props: Vec<Prop>,
        body: ItemFn,
        ret: ReturnType,
    }

    #[derive(Debug)]
    pub struct Prop {
        //docs: Docs,
        prop_opts: PropOpt,
        name: PatIdent,
        ty: Type,
    }

    #[derive(Clone, Debug, FromAttr)]
    #[attribute(ident = prop)]
    pub struct PropOpt {
        #[attribute(conflicts = [input, output])]
        optional: bool,
        #[attribute(example = "5 * 10", conflicts = [input, output])]
        default: Option<syn::Expr>,
        #[attribute(conflicts = [optional, default, output])]
        input: bool,
        #[attribute(conflicts = [optional, default, input])]
        output: bool,
        attrs: bool,
        name: Option<String>,
    }

    impl Prop {
        fn new(arg: &FnArg) -> Self {
            let typed = if let FnArg::Typed(ty) = arg {
                ty
            } else {
                abort!(arg, "receiver not allowed in `fn`");
            };

            let prop_opts = PropOpt::from_attributes(&typed.attrs).unwrap_or_else(|e| {
                // TODO: replace with `.unwrap_or_abort()`
                abort!(e.span(), e.to_string())
            });

            let name = match typed.pat.as_ref() {
                Pat::Ident(i) => {
                    if let Some(name) = &prop_opts.name {
                        PatIdent {
                            attrs: vec![],
                            by_ref: None,
                            mutability: None,
                            ident: Ident::new(name, i.span()),
                            subpat: None,
                        }
                    } else {
                        i.clone()
                    }
                }
                _ => {
                    abort!(
                        typed.pat,
                        "only `prop: bool` style types are allowed within the #[reactor] macro"
                    );
                }
            };

            Self {
                prop_opts,
                name,
                ty: *typed.ty.clone(),
            }
        }
    }

    impl Parse for Model {
        fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
            let mut item = ItemFn::parse(input)?;

            let props = item.sig.inputs.iter().map(Prop::new).collect::<Vec<_>>();

            // Remove the `#[doc =""]` and `#[builder(_)]` attributes from the function signature
            item.attrs.retain(|attr| match &attr.meta {
                Meta::NameValue(attr) if (attr.path.is_ident("doc")) => false,
                Meta::List(attr) if (attr.path.is_ident("prop")) => false,
                _ => true,
            });

            Ok(Self {
                vis: item.vis.clone(),
                name: convert_from_snake_case(&item.sig.ident),
                props,
                ret: item.sig.output.clone(),
                body: item,
            })
        }
    }

    impl ToTokens for Model {
        fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
            let Self {
                vis,
                name,
                props,
                body,
                ret,
            } = self;

            todo!();
            //quote! {}
        }
    }
}

#[proc_macro_error2::proc_macro_error]
#[proc_macro_attribute]
pub fn reactor2(
    args: proc_macro::TokenStream,
    s: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    //let mut dummy = syn::parse::<DummyModel>(s.clone());
    let parse_result = syn::parse::<reactor2::Model>(s);

    if let Ok(model) = parse_result {
        let expanded = model.into_token_stream();

        quote! {
            #expanded
        }
    } else {
        quote! {}
    }
    .into()
}

#[proc_macro_error2::proc_macro_error]
#[proc_macro]
pub fn reactor(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let model = syn::parse_macro_input!(tokens as reactor_macro::Reactor);

    quote! {
        #model
    }
    .into()
}
