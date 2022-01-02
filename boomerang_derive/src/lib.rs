#![feature(assert_matches)]
#![feature(hash_set_entry)]

//! This crate provides Boomerangs' derive macro.

mod reaction;
mod reactor;
mod util;

use darling::{FromDeriveInput, ToTokens};

use quote::quote;
use syn::{parse_macro_input, AttributeArgs, ItemFn};

#[doc(hidden)]
#[proc_macro_derive(Reactor, attributes(reactor))]
pub fn derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    #[cfg(feature = "logging")]
    INIT_LOGGER.call_once(|| {
        env_logger::init().unwrap();
    });
    let ast = parse_macro_input!(input as syn::DeriveInput);
    let receiver = reactor::ReactorReceiver::from_derive_input(&ast);
    //.and_then(|receiver| receiver.validate());

    match receiver {
        Ok(receiver) => quote!(#receiver),
        Err(err) => err.write_errors(),
    }
    .into()
}

#[proc_macro_attribute]
pub fn reaction(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(input as ItemFn);

    match reaction::ReactionReceiver::new(args, input) {
        Ok(recvr) => recvr.to_token_stream(),
        Err(err) => err.write_errors(),
    }
    .into()
}
