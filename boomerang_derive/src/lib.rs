#![feature(hash_set_entry)]

//! This crate provides Boomerangs' derive macro.
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

use darling::FromDeriveInput;
use proc_macro::TokenStream;
use quote::quote;
use std::convert::TryFrom;
use syn::parse_macro_input;

#[doc(hidden)]
#[proc_macro_derive(Reactor, attributes(reactor))]
pub fn derive(input: TokenStream) -> TokenStream {
    #[cfg(feature = "logging")]
    INIT_LOGGER.call_once(|| {
        env_logger::init().unwrap();
    });
    let ast = parse_macro_input!(input as syn::DeriveInput);
    let receiver = parse::ReactorReceiver::from_derive_input(&ast).unwrap();
    let builder = builder::ReactorBuilder::try_from(receiver).unwrap();
    quote!(#builder).into()
}

struct S
{
    poo: Box<dyn FnMut(&mut u32) -> bool>,
}

#[test]
fn test2() {
    let mut x = std::rc::Rc::new(0.0);

    let mut s = S {
        poo: Box::new(|x: &mut u32| *x > 0),
    };
}
