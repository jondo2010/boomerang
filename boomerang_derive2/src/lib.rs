use darling::FromDeriveInput;
use quote::ToTokens;

mod reaction;

#[proc_macro_derive(Reaction, attributes(reaction))]
pub fn derive_reaction(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    #[cfg(feature = "logging")]
    INIT_LOGGER.call_once(|| {
        env_logger::init().unwrap();
    });
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);
    let receiver = reaction::ReactionReceiver::from_derive_input(&ast);
    //.and_then(|receiver| receiver.validate());

    match receiver {
        Ok(receiver) => receiver.to_token_stream(),
        Err(err) => err.write_errors(),
    }
    .into()
}
