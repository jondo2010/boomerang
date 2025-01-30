use darling::FromDeriveInput;
use quote::{quote, ToTokens};

//mod fn_reactor;
mod reaction;
mod reactor;
mod reactor2;
mod reactor_macro;
mod reactor_ports;
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

#[proc_macro_error2::proc_macro_error]
#[proc_macro_attribute]
pub fn reactor_ports(
    _attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let model = syn::parse_macro_input!(input as reactor_ports::Model);
    quote!(#model).into()
}

/// Annotates a function so that it can be used as a Boomerang reactor.
///
/// The `#[reactor]` macro allows you to annotate plain Rust functions as reactor builders. The reactor function takes
/// any number of other arguments.
///
/// Here’s how you would define and use a simple Boomerang reactor which has one input, and a delay parameter:
/// ```rust
/// # use boomerang::prelude::*;
///
/// #[reactor]
/// pub fn MyComponent(
///     #[input] x: u32,
///     #[default(Duration::seconds(1))] delay: Duration,
/// ) -> impl IntoView {
///    // Your reactor implementation goes here
/// }
/// ```
#[proc_macro_error2::proc_macro_error]
#[proc_macro_attribute]
pub fn reactor(
    args: proc_macro::TokenStream,
    s: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut dummy = syn::parse::<reactor2::DummyModel>(s.clone());
    let parse_result = syn::parse::<reactor2::Model>(s);

    if let (
        Ok(ref mut unexpanded),
        Ok(model)
    ) = (
        &mut dummy,
        parse_result
    ) {
        let expanded = model.into_token_stream();
        if !matches!(unexpanded.vis, syn::Visibility::Public(_)) {
            //unexpanded.vis = syn::Visibility::Public(syn::token::Pub { span: unexpanded.vis.span(), })
        }
        //unexpanded.sig.ident = unmodified_fn_name_from_fn_name(&unexpanded.sig.ident);
        quote! {
            #[allow(non_snake_case)]
            #expanded

            //#[doc(hidden)]
            //#[allow(non_snake_case, dead_code, clippy::too_many_arguments, clippy::needless_lifetimes)]
            //#unexpanded
        }
    } else {
        match dummy {
            Ok(mut dummy) => {
                //dummy.sig.ident = unmodified_fn_name_from_fn_name(&dummy.sig.ident);
                quote! {
                    #[doc(hidden)]
                    #[allow(non_snake_case, dead_code, clippy::too_many_arguments, clippy::needless_lifetimes)]
                    #dummy
                }
            }
            Err(e) => {
                proc_macro_error2::abort!(e.span(), e);
            }
        }
    }.into()
}
