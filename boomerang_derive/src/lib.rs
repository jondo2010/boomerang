//! This crate provides Boomerangs's macros.
//!
//! ```edition2018
//! use boomerang_derive::Reactor;
//! 
//! #[derive(Reactor)]
//! struct MyReactor {}
//! 
//! fn main() {}
//! ```

mod detail;

use darling::FromDeriveInput;
use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

use detail::ReactorReceiver;

#[doc(hidden)]
#[proc_macro_derive(Reactor, attributes(reactor))]
pub fn derive(input: TokenStream) -> TokenStream {
    #[cfg(feature = "logging")]
    INIT_LOGGER.call_once(|| {
        env_logger::init().unwrap();
    });

    let ast = parse_macro_input!(input as syn::DeriveInput);
    let reactor = ReactorReceiver::from_derive_input(&ast).unwrap();
    // println!("Reactor \"{}\"", reactor.ident.to_string());
    // reactor.data.map_struct_fields(|field| {
    // println!("Field: {:?}, {:?}", field.ident, field.timer);
    // });

    quote!(#reactor).into()
}
