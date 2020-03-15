//! This crate provides Boomerangs's macros.
//!
//! ```edition2018
//! # use boomerang_derive::Reactor;
//! #
//! #[derive(Reactor)]
//! # struct MyReactor;
//! #
//! # fn main() {}
//! ```

use proc_macro::TokenStream;

#[proc_macro_derive(Reactor, attributes(reactor))]
pub fn derive_reactor(item: TokenStream) -> TokenStream {
    println!("item: \"{}\"", item.to_string());
    "fn answer() -> u32 { 42 }".parse().unwrap()
}

#[proc_macro_attribute]
pub fn reaction(attr: TokenStream, item: TokenStream) -> TokenStream {
    println!("attr: \"{}\"", attr.to_string());
    println!("item: \"{}\"", item.to_string());
    item
}
