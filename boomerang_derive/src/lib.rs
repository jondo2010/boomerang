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
use std::convert::TryFrom;
use syn::parse_macro_input;

impl ToTokens for ReactorReceiver {
    /// # Panics
    /// This method panics if the field attributes input, output, timer are not mutually-exclusive.
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let type_ident = &self.ident;
        let (imp, ty, wher) = self.generics.split_for_impl();
        let fields = self
            .data
            .as_ref()
            .take_struct()
            .expect("Should never be enum")
            .fields;

        let mut impl_details = proc_macro2::TokenStream::new();

        for f in fields {
            let field_ident = &f.ident;
            let field_type = &f.ty;
        }

        tokens.extend(quote! {
            impl #imp #type_ident #ty #wher {
                //pub fn schedule(this: &Rc<RefCell<Self>>, scheduler: &mut S) {}
                pub fn new() -> Self {
                    use crate::*;
                    println!("Poo!");
                    Self {
                        #impl_details
                        ..Default::default()
                    }
                }
            }
        })
    }
}

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
    // println!("Reactor \"{}\"", reactor.ident.to_string());
    // reactor.data.map_struct_fields(|field| {
    // println!("Field: {:?}, {:?}", field.ident, field.timer);
    // });

    use petgraph::dot::{Dot, Config};

    let builder = builder::ReactorBuilder::try_from(reactor).unwrap();
    let graph = builder.get_dependency_graph();
    let dot = Dot::with_config(&graph, &[Config::EdgeNoLabel]);

    println!("{}", dot);

    // println!("{}", quote!(#reactor).to_string());

    // quote!(#reactor).into()
    quote!().into()
}
