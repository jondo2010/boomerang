use darling::FromDeriveInput;
use quote::ToTokens;

mod reaction;
mod reactor;
//mod util;

#[proc_macro_derive(Reaction, attributes(reaction))]
pub fn derive_reaction(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    #[cfg(feature = "logging")]
    INIT_LOGGER.call_once(|| {
        env_logger::init().unwrap();
    });

    let ast = syn::parse_macro_input!(input as syn::DeriveInput);
    let reaction: Result<reaction::Reaction, _> =
        reaction::ReactionReceiver::from_derive_input(&ast)
            .and_then(|receiver| receiver.try_into());

    match reaction {
        Ok(receiver) => receiver.to_token_stream(),
        Err(err) => err.write_errors(),
    }
    .into()
}

#[proc_macro_derive(Reactor, attributes(reactor))]
pub fn derive_reactor(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);
    let receiver = reactor::ReactorReceiver::from_derive_input(&ast);

    match receiver {
        Ok(receiver) => receiver.to_token_stream(),
        Err(err) => err.write_errors(),
    }
    .into()
}
