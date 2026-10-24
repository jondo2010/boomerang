use quote::quote;

mod component;
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
/// The `#[reactor]` macro turns a plain Rust function into a reactor declaration. The reactor function takes
/// any number of other arguments.
/// Direct attributes always emit the hosted API.
/// Use [`component!`] or explicit component macros for metadata and payload emission.
///
/// Here’s how you would define and use a simple Boomerang reactor which has one input, a delay parameter, and a boolean state:
/// ```rust,no_run
/// # use boomerang::prelude::*;
///
/// #[reactor]
/// pub fn MyComponent(
///     #[input] x: u32,
///     #[param(default = Duration::seconds(1))] delay: Duration,
///     #[state] is_good: bool,
/// ) -> impl Reactor {
///    // Your reactor implementation goes here
/// }
/// ```
///
/// Action arguments use `#[logical_action]` or `#[physical_action]`; the argument type is the
/// action payload type. Both attributes accept an optional `min_delay = <duration>`, where a
/// positive duration includes a unit (for example, `10 msec`) and zero may be written as `0`.
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
/// pub fn MyComponent() -> impl Reactor<MyState> {
///    // Your reactor implementation goes here
/// }
/// ```
///
/// ### Construct state before logical startup
///
/// Use `state_init = path` to select a constructor for `state = T`.
/// Without `init_context`, the constructor has signature `fn() -> T`.
/// Add `init_context` to generate a named context and pass it to the constructor:
///
/// ```rust,no_run
/// use boomerang::prelude::*;
///
/// #[derive(Debug)]
/// pub struct InputState;
/// impl InputState {
///     fn new(init: InputInit) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
///         init.send.prepare_action(&init.sample, 42, None)?.try_send()?;
///         Ok(Self)
///     }
/// }
///
/// #[reactor(
///     state = InputState,
///     state_init = InputState::new,
///     init_context(name = InputInit, actions = [sample])
/// )]
/// pub fn Input(#[physical_action] sample: u32) -> impl Reactor {
///     reaction! { receive(sample) {
///         let _ = ctx.get_action_value(&mut sample);
///     } }
/// }
/// ```
///
/// `init_context` requires `state` and `state_init`. With this option, the
/// constructor has signature `fn(GeneratedContext) -> Result<T, E>`.
/// `E` must convert to `Box<dyn std::error::Error + Send + Sync>`.
/// The macro passes the generated context by value. For a generic reactor,
/// the context has the same generic parameters and bounds.
///
/// | Option | Generated fields |
/// | --- | --- |
/// | `name = InputInit` | Sets the generated structure name. |
/// | Always present | `send: SendContext` |
/// | `actions = [sample]`, without `source` | `sample: AsyncActionRef<T>` for payload type `T` |
/// | `source = "plant"` | `source: InputSourceHandle` |
/// | `actions = [sample]`, with `source` | `sample: SourceAction<T>` |
/// | `clock_control = true` | `clock: ManualClock` |
///
/// Each action must be a local physical action. Duplicate names and the names
/// `source`, `clock`, `send`, and `__boomerang_marker` are rejected. A source declaration applies to
/// all listed actions; one context cannot mix ordinary and source action handles.
/// Compiled source declarations require at least one action.
///
/// Generated local hosted launchers call these initializers automatically.
/// With the live builder, use `Reactor::build_initialized`. Methods that accept
/// supplied state do not call the initializer. The live builder supports ordinary
/// action contexts. Sources and manual clock control require compiled local
/// hosted Federate execution and the runtime features `hosted-source-input` and
/// `hosted-manual-clock`, respectively. They are independent options.
/// A source also needs a deployment source policy. Manual clock control needs a
/// selected manual clock and permits only one owning initializer per Federate.
///
/// Capability validation finishes before context initializers run. They run before logical
/// startup and must return without waiting for reactions. Use nonblocking sends
/// during initialization. Keep guards that cancel and join workers in the
/// returned state. Clean up workers if construction fails before returning state.
#[proc_macro_error2::proc_macro_error]
#[proc_macro_attribute]
pub fn reactor(
    args: proc_macro::TokenStream,
    s: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let args = syn::parse_macro_input!(args as reactor::ReactorArgs);

    match syn::parse::<reactor::Model>(s) {
        Ok(model) => {
            let args_model = reactor::ArgsModel(args, model, false);
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
///
/// #[reactor]
/// fn MyReactor(
///     #[output] x: u32,
/// ) -> impl Reactor {
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
/// `timer! { <name>(<offset>, <period>) }`
///
/// - `offset` specifies an optional (logical) delay from the start of execution before this timer starts triggering.
/// - `period` specifies the optional
///
/// ## Example
/// ```rust,no_run
/// # use boomerang::prelude::*;
///
/// #[reactor]
/// fn MyReactor() -> impl Reactor {
///    // Create a timer named `t1` that triggers every 50 milliseconds
///    timer! { t1(0 sec, 50 msec) };
/// }
/// ```
#[proc_macro_error2::proc_macro_error]
#[proc_macro]
pub fn timer(s: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let model = syn::parse_macro_input!(s as timer::Model);
    quote! { #model }.into()
}

/// Declares one named component in an inline Rust module.
///
/// The module must contain exactly one root `#[reactor]` declaration. Helper
/// items remain ordinary owning-crate code and are omitted in metadata builds.
/// The invoking package's additive `metadata` and `payload` features select APIs in `module::__boomerang::{metadata,payload}`.
/// Both features can coexist.
/// Without either feature, the macro emits hosted APIs.
/// Metadata also provides typed topology constructors through `module::definition()`.
/// Declare these features in the component package.
/// Gate implementation-only dependencies behind `payload`.
/// Use resolver 2 or newer to isolate build dependencies.
/// Use [`component_metadata!`] in build scripts to select metadata independently of package payload features.
/// Code outside the declaration must follow the same host/target isolation rules.
/// Root attributes other than documentation are rejected; put conditional and lint
/// attributes on the component module instead.
#[proc_macro_error2::proc_macro_error]
#[proc_macro]
pub fn component(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match component::expand(input.into(), reactor::ComponentEmission::Features) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Emits portable component metadata and typed topology constructors from one inline module.
///
/// Accepts the same declaration as [`component!`], independently of package features.
/// Parses but omits implementation imports, helper items, state initializers, and reaction bodies.
/// Access the descriptor through `module::__boomerang::metadata::descriptor()`.
/// Construct topology with `module::definition()`.
/// Structural port types must be available on the host.
/// Use fully qualified paths for types from portable dependencies.
/// Metadata emission omits implementation imports inside the declaration.
/// Metadata requires `boomerang/hosted`.
/// Use `default-features = false` for payload-only normal dependencies that require no hosted APIs.
///
/// A shared source macro can invoke this entry point from a build script and
/// [`component_payload!`] from the target. Explicit emission does not inspect package features.
/// Do not invoke both entry points for the same module in one compilation.
/// To emit both APIs, use [`component!`] with both package features or use distinct enclosing modules.
///
/// ```
/// use boomerang::component_metadata;
/// component_metadata! {
///     pub mod controller {
///         use boomerang::prelude::*;
///         // Implementation-only items are parsed but never emitted for metadata.
///         use target_only_dependency::implementation;
///         #[reactor(contract = "example.controller", contract_version = 1,
///             bounds(queue_capacity = 16, payload_bytes = 1024,
///                    state_bytes = 512, scratch_bytes = 256))]
///         pub fn Controller(#[output] command: u32) -> impl Reactor {
///             reaction! { control(startup) -> command { *command = Some(implementation()); } }
///         }
///     }
/// }
/// assert_eq!(controller::__boomerang::metadata::descriptor().contract_version(), 1);
/// let _definition = controller::definition();
/// ```
#[proc_macro_error2::proc_macro_error]
#[proc_macro]
pub fn component_metadata(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match component::expand(input.into(), reactor::ComponentEmission::Metadata) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Emits target payload bindings from the same inline declaration as [`component_metadata!`].
///
/// Keeps implementation items in their owning module.
/// The declaration determines compatibility information in `module::__boomerang::payload::binding_manifest()` without CLI inputs.
/// This entry point selects payload even when the invoking package enables metadata features.
#[proc_macro_error2::proc_macro_error]
#[proc_macro]
pub fn component_payload(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match component::expand(input.into(), reactor::ComponentEmission::Payload) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}
