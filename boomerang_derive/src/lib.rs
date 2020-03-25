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
mod util;

use darling::{FromDeriveInput, ToTokens};
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, AttributeArgs};

use detail::ReactorReceiver;

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
            match (&f.input, &f.output) {
                (false, false) => impl_details.extend(quote!(
                    //#field_ident : Default::default(),
                )),
                (true, false ) => todo!(),
                (false, true ) => impl_details.extend(quote!(
                    //#field_ident : Rc::new(RefCell::new(Port::default())),
                )),
                /*
                (false, false, Some(timer)) => {
                    let offset =
                        syn::parse_str::<syn::Expr>(&timer.offset).expect("Invalid expression");
                    let period =
                        syn::parse_str::<syn::Expr>(&timer.period).expect("Invalid expression");
                    impl_details.extend(quote!(
                        #field_ident : Rc::new(RefCell::new(Trigger:: #ty ::new(
                            /* reactions:*/ vec![],
                            /* offset:*/ #offset,
                            /* period:*/ Some(#period),
                            /* is_physical:*/ false,
                            /* policy:*/ QueuingPolicy::NONE,
                        ))),
                        //let t = S::Timer::new();
                    ));
                }
                (false, false, None, Some(reaction)) => {
                    impl_details.extend(quote!(
                        #field_ident : {
                            let this_clone = this.clone();
                            let closure = Box::new(RefCell::new(move |sched: &mut S| {
                                HelloWorld::hello(&mut (*this_clone).borrow_mut(), sched);
                            }));
                            ::std::rc::Rc::new(Reaction::new(
                                "hello_reaction",
                                /* reactor */ closure,
                                /* index */ 0,
                                /* chain_id */ 0,
                                /* triggers */ vec![(this.borrow().output.clone(), vec![reply_in_trigger])],
                            ))
                        };
                    ));
                }
                */
                _ => panic!(format!(
                    "Reactor attributes input/output/timer must be mutually exclusive: {:?}",
                    (&f.input, &f.output)
                )),
            }
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

#[proc_macro_attribute]
pub fn timer(args: TokenStream, input: TokenStream) -> TokenStream {
    println!("timer: {}", args.to_string());
    println!("timer: {}", input.to_string());
    let ast = parse_macro_input!(args as AttributeArgs);
    input
}

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

    // println!("{}", quote!(#reactor).to_string());

    quote!(#reactor).into()
}
