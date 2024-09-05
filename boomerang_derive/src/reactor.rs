use std::time::Duration;

use darling::{ast, FromDeriveInput, FromField, FromMeta};
use quote::quote;
use quote::ToTokens;

fn handle_duration(value: String) -> Option<Duration> {
    Some(parse_duration::parse(&value).unwrap())
}

/// Generate a TokenStream from an Option<Duration>
pub(crate) fn duration_quote(duration: &Duration) -> proc_macro2::TokenStream {
    let secs = duration.as_secs();
    let nanos = duration.subsec_nanos();
    quote! {::boomerang::runtime::Duration::new(#secs, #nanos)}
}

pub struct OptionalDuration(pub Option<Duration>);

impl ToTokens for OptionalDuration {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.extend(match self.0 {
            Some(duration) => {
                let duration = duration_quote(&duration);
                quote! {Some(#duration)}
            }
            None => quote! {None},
        });
    }
}

#[derive(Clone, Debug, FromMeta, PartialEq, Eq)]
pub struct TimerAttr {
    #[darling(default, map = "handle_duration")]
    pub offset: Option<Duration>,
    #[darling(default, map = "handle_duration")]
    pub period: Option<Duration>,
}

#[derive(Clone, Debug, FromMeta, PartialEq, Eq)]
pub enum PortAttr {
    Input,
    Output,
}

#[derive(Clone, Debug, FromMeta, PartialEq, Eq, Default)]
pub enum ActionAttrPolicy {
    #[default]
    Defer,
    Drop,
}

#[derive(Clone, Debug, FromMeta, PartialEq, Eq)]
pub struct ActionAttr {
    #[darling(default)]
    pub physical: bool,
    #[darling(default, map = "handle_duration")]
    pub min_delay: Option<Duration>,
    #[darling(default)]
    pub policy: Option<ActionAttrPolicy>,
}

/// Attributes on fields in a Reactor
#[derive(Clone, Debug, FromField, PartialEq, Eq)]
#[darling(attributes(reactor), forward_attrs(doc, cfg, allow))]
pub struct FieldReceiver {
    pub ident: Option<syn::Ident>,
    pub vis: syn::Visibility,
    pub ty: syn::Type,
    pub rename: Option<syn::Ident>,
    pub timer: Option<TimerAttr>,
    pub port: Option<PortAttr>,
    pub action: Option<ActionAttr>,
    pub child: Option<syn::Expr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactorField {
    pub ident: syn::Ident,
    pub name: syn::Ident,
    pub ty: syn::Type,
    pub kind: ReactorFieldKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReactorFieldKind {
    Timer {
        period: Option<Duration>,
        offset: Option<Duration>,
    },
    Input,
    Output,
    Action {
        min_delay: Option<Duration>,
        policy: ActionAttrPolicy,
    },
    Child {
        state: syn::Expr,
    },
    Reaction,
}

impl ToTokens for ReactorField {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;
        let name = self.name.to_string();
        let ty = &self.ty;
        tokens.extend(match &self.kind {
            ReactorFieldKind::Timer { period, offset } => {
                let period = OptionalDuration(*period);
                let offset = OptionalDuration(*offset);
                quote! { let __inner = ::boomerang::builder::TimerSpec { period: #period, offset: #offset, }; }
            }
            ReactorFieldKind::Input => {
                quote! { let __inner = ::boomerang::builder::PortType::Input; }
            },
            ReactorFieldKind::Output => {
                quote! { let __inner = ::boomerang::builder::PortType::Output; }
            },
            ReactorFieldKind::Action { min_delay, policy: _ } => {
                let min_delay = OptionalDuration(*min_delay);
                quote! { let __inner = #min_delay; }
            },
            ReactorFieldKind::Child { state } => {
                quote! { let __inner = #state; }
            }
            ReactorFieldKind::Reaction => {
                quote! { let __inner = __reactor.clone(); }
            }
        });

        tokens.extend(quote! {
            let #ident = <#ty as ::boomerang::builder::ReactorField>::build(#name, __inner, &mut __builder)?;
        });
    }
}

impl TryFrom<FieldReceiver> for ReactorField {
    type Error = darling::Error;

    fn try_from(value: FieldReceiver) -> Result<Self, Self::Error> {
        let ident = value.ident.unwrap();
        let name = value.rename.unwrap_or_else(|| ident.clone());
        let ty = value.ty;

        match (value.timer, value.port, value.action, value.child) {
            (Some(timer), None, None, None) => Ok(ReactorField {
                ident,
                name,
                ty,
                kind: ReactorFieldKind::Timer {
                    period: timer.period,
                    offset: timer.offset,
                },
            }),
            (None, Some(port), None, None) => Ok(ReactorField {
                ident,
                name,
                ty,
                kind: match port {
                    PortAttr::Input => ReactorFieldKind::Input,
                    PortAttr::Output => ReactorFieldKind::Output,
                },
            }),
            (None, None, Some(action), None) => Ok(ReactorField {
                ident,
                name,
                ty,
                kind: ReactorFieldKind::Action {
                    min_delay: action.min_delay,
                    policy: action.policy.unwrap_or_default(),
                },
            }),
            (None, None, None, Some(child)) => Ok(ReactorField {
                ident,
                name,
                ty,
                kind: ReactorFieldKind::Child { state: child },
            }),
            (None, None, None, None) => {
                // Check if the type is a TypedReactionKey<...>
                if let syn::Type::Path(type_path) = &ty {
                    if let Some(segment) = type_path.path.segments.last() {
                        if segment.ident == "TypedReactionKey" {
                            return Ok(ReactorField {
                                ident,
                                name,
                                ty,
                                kind: ReactorFieldKind::Reaction,
                            });
                        }
                    }
                }

                // If not a TypedReactionKey, return an error
                Err(
                    darling::Error::custom("Reaction field must be of type TypedReactionKey<...>")
                        .with_span(&ty),
                )
            }
            _ => Err(darling::Error::unsupported_format(
                "Only one of timer, port, action, or child attributes can be specified",
            )),
        }
    }
}

#[derive(Debug, FromMeta, Eq, PartialEq)]
pub struct ConnectionAttr {
    from: syn::Expr,
    to: syn::Expr,
    #[darling(default, map = "handle_duration")]
    pub after: Option<Duration>,
}

impl ToTokens for ConnectionAttr {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let from_port = &self.from;
        let to_port = &self.to;
        tokens.extend(quote! {
            __builder.bind_port(__reactor.#from_port, __reactor.#to_port)?;
        });
    }
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reactor), supports(struct_named))]
pub struct ReactorReceiver {
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    // pub attrs: Vec<syn::Attribute>,
    pub data: ast::Data<darling::util::Ignored, FieldReceiver>,
    /// Type of the reactor state
    state: syn::Expr,
    /// Connection definitions
    #[darling(default, multiple, rename = "connection")]
    pub connections: Vec<ConnectionAttr>,
}

impl ToTokens for ReactorReceiver {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ident = &self.ident;
        let state = &self.state;
        let generics = &self.generics;

        let fields = self
            .data
            .as_ref()
            .take_struct()
            .unwrap()
            .fields
            .into_iter()
            .cloned()
            .map(ReactorField::try_from)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        let non_reaction_fields = fields
            .iter()
            .filter(|field| !matches!(field.kind, ReactorFieldKind::Reaction));

        let reaction_fields = fields
            .iter()
            .filter(|field| matches!(field.kind, ReactorFieldKind::Reaction));

        let field_idents = fields.iter().map(|field| {
            let ident = &field.ident;
            if let ReactorFieldKind::Reaction = field.kind {
                quote! { #ident: Default::default() }
            } else {
                quote! { #ident }
            }
        });

        let reaction_assignments = fields.iter().filter_map(|field| {
            let ident = &field.ident;
            if let ReactorFieldKind::Reaction = field.kind {
                Some(quote! {
                    __reactor.#ident = #ident;
                })
            } else {
                None
            }
        });

        let connections = &self.connections;

        tokens.extend(quote! {
            impl ::boomerang::builder::Reactor for #ident #generics {
                type State = #state;

                fn build<'__builder>(
                    name: &str,
                    state: Self::State,
                    parent: Option<::boomerang::builder::BuilderReactorKey>,
                    env: &'__builder mut ::boomerang::builder::EnvBuilder,
                ) -> Result<Self, ::boomerang::builder::BuilderError> {
                    let mut __builder = env.add_reactor(name, parent, state);

                    #(#non_reaction_fields)*

                    let mut __reactor = Self {
                        #(#field_idents),*
                    };

                    #(#reaction_fields)*

                    #(#reaction_assignments)*

                    #(#connections)*

                    Ok(__reactor)
                }
            }
        });
    }
}

#[test]
fn test_reactor() {
    let good_input = r#"
#[derive(Reactor, Clone)]
#[reactor(state = "u32")]
struct Count {
    #[reactor(rename = "foo", timer(period = "1 usec"))]
    timer: TimerActionKey,
    #[reactor(rename = "p0", port = "output")]
    output: TypedPortKey<u32>,

    #[reactor(action(min_delay = "1 usec", policy = "drop"))]
    action: TypedActionKey<u32, Physical>,

    #[reactor(child = "Timeout::new(runtime::Duration::from_secs(1))")]
    _timeout: TimeoutBuilder,
    //#[reactor(reaction)]
    //reaction_t: TypedReactionKey<ReactionT<'static>>,
    //#[reactor(reaction)]
    //reaction_shutdown: TypedReactionKey<ReactionShutdown>,
}"#;

    let parsed = syn::parse_str(good_input).unwrap();
    let receiver = ReactorReceiver::from_derive_input(&parsed).unwrap();

    let fields = receiver.data.take_struct().unwrap();

    let fields = fields
        .into_iter()
        .map(ReactorField::try_from)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    dbg!(&fields);
    //println!("{}", receiver.to_token_stream());
}
