use darling::FromDeriveInput;
use quote::ToTokens;

mod reaction;
mod reactor;
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
    use syn::{Ident, ItemFn, ReturnType, Visibility};

    pub struct Model {
        //docs: Docs,
        //unknown_attrs: UnknownAttrs,
        vis: Visibility,
        name: Ident,
        //props: Vec<Prop>,
        body: ItemFn,
        ret: ReturnType,
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
}
