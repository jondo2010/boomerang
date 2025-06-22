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

/// Annotate a struct as a builder for a Reactor.
///
/// ## Example
/// ```rust
/// # use boomerang::prelude::*;
/// struct State {
///     success: bool,
/// }
///
/// #[derive(Reactor)]
/// #[reactor(state = "State")]
/// struct HelloWorld;
/// ```
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
