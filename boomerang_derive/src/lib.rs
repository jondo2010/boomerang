use darling::FromDeriveInput;
use quote::{quote, ToTokens};

//mod fn_reactor;
mod reaction;
mod reactor;
mod reactor2;
mod reactor_macro;
mod reactor_ports;
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

#[proc_macro_error2::proc_macro_error]
#[proc_macro_attribute]
pub fn reactor_ports(
    _attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let model = syn::parse_macro_input!(input as reactor_ports::Model);
    quote!(#model).into()
}

/// Annotates a function so that it can be used as a Boomerang reactor.
///
/// The `#[reactor]` macro allows you to annotate plain Rust functions as reactor builders. The reactor function takes
/// any number of other arguments.
///
/// Here’s how you would define and use a simple Boomerang reactor which has one input, a delay parameter, and a boolean state:
/// ```rust
/// # use boomerang::prelude::*;
///
/// #[reactor]
/// pub fn MyComponent(
///     #[input] x: u32,
///     #[default(Duration::seconds(1))] delay: Duration,
///     #[state] is_good: bool,
/// ) -> impl IntoView {
///    // Your reactor implementation goes here
/// }
/// ```
///
/// ### Using your own `state` struct
///
/// By default the macro will generate a state struct definition for you (e.g., `MyComponentState`) consisting of all
/// the function arguments tagged with `#[state]` attributes.
///
/// If you want to instead use your own state struct, you can do so with the `state` argument to the `reactor` macro:
/// ```rust
/// # use boomerang::prelude::*;
///
/// struct MyState {
///    is_good: bool,
/// }
///
/// #[reactor(state = MyState)]
/// pub fn MyComponent() -> impl Reactor2<MyState> {
///    // Your reactor implementation goes here
/// }
/// ```
#[proc_macro_error2::proc_macro_error]
#[proc_macro_attribute]
pub fn reactor(
    args: proc_macro::TokenStream,
    s: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let args = syn::parse_macro_input!(args as reactor2::ReactorArgs);

    match syn::parse::<reactor2::Model>(s) {
        Ok(model) => {
            let args_model = reactor2::ArgsModel(args, model);
            quote! {
                #args_model
            }
        }
        Err(e) => {
            proc_macro_error2::abort!(e.span(), e);
        }
    }
    .into()
}
