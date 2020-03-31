#![feature(hash_set_entry)]

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

mod builder;
mod parse;
mod util;

use builder::*;

use darling::{FromDeriveInput, ToTokens};
use parse::{ReactorReceiver, TimerAttr};
use proc_macro::TokenStream;
use quote::quote;
use std::{convert::TryFrom, iter::FromIterator};
use syn::parse_macro_input;

// #[proc_macro_attribute]
// pub fn timer(args: TokenStream, input: TokenStream) -> TokenStream {
// println!("timer: {}", args.to_string());
// println!("timer: {}", input.to_string());
// let ast = parse_macro_input!(args as AttributeArgs);
// input
// }

#[doc(hidden)]
#[proc_macro_derive(Reactor, attributes(reactor))]
pub fn derive(input: TokenStream) -> TokenStream {
    #[cfg(feature = "logging")]
    INIT_LOGGER.call_once(|| {
        env_logger::init().unwrap();
    });

    let ast = parse_macro_input!(input as syn::DeriveInput);
    let reactor = ReactorReceiver::from_derive_input(&ast).unwrap();
    let builder = builder::ReactorBuilder::try_from(reactor).unwrap();

    quote!(#builder).into()
}
