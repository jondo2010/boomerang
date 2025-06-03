use quote::quote;

mod ports;
mod reaction;
mod reactor;
mod time;
mod timer;
mod util;

#[proc_macro_error2::proc_macro_error]
#[proc_macro_attribute]
pub fn reactor_ports(
    _attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let model = syn::parse_macro_input!(input as ports::Model);
    quote! { #model }.into()
}

/// Annotates a function so that it can be used as a Boomerang reactor.
///
/// The `#[reactor]` macro allows you to annotate plain Rust functions as reactor builders. The reactor function takes
/// any number of other arguments.
///
/// Here’s how you would define and use a simple Boomerang reactor which has one input, a delay parameter, and a boolean state:
/// ```rust,no_run
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
/// ```rust,no_run
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
    let args = syn::parse_macro_input!(args as reactor::ReactorArgs);

    match syn::parse::<reactor::Model>(s) {
        Ok(model) => {
            let args_model = reactor::ArgsModel(args, model);
            quote! { #args_model }
        }
        Err(e) => {
            proc_macro_error2::abort!(e.span(), e);
        }
    }
    .into()
}

/// Creates a reaction within a reactor function.
///
/// ```rust,no_run
/// # use boomerang::prelude::*;
/// fn MyReactor(
///     #[output] x: u32,
/// ) -> impl Reactor2 {
///     reaction! {
///         starting_reaction (startup) -> x {
///             // Your reaction implementation goes here
///         }
///     }
/// }
/// ```
#[proc_macro_error2::proc_macro_error]
#[proc_macro]
pub fn reaction(s: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let model = syn::parse_macro_input!(s as reaction::Model);
    quote! { #model }.into()
}

/// Creates a timer within a reactor function.
///
/// ## Usage
/// ```rust,no_run
/// # use boomerang::prelude::*;
/// fn MyReactor() -> impl Reactor2 {
///    // Create a timer named `t1` that triggers every 50 milliseconds
///    timer! { t1(0, 50 msec) };
/// }
/// ```
#[proc_macro_error2::proc_macro_error]
#[proc_macro]
pub fn timer(s: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let model = syn::parse_macro_input!(s as timer::Model);
    quote! { #model }.into()
}
