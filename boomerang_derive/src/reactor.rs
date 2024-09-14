use std::time::Duration;

use darling::{ast, FromDeriveInput, FromField, FromMeta};
use quote::quote;
use quote::ToTokens;
use syn::Type;
use syn::TypePath;

use crate::util::OptionalDuration;
use crate::util::{extract_path_ident, handle_duration};

const TIMER_ACTION_KEY: &str = "TimerActionKey";
const TYPED_ACTION_KEY: &str = "TypedActionKey";
const TYPED_PORT_KEY: &str = "TypedPortKey";
const TYPED_REACTION_KEY: &str = "TypedReactionKey";
const PHYSICAL_ACTION_KEY: &str = "PhysicalActionKey";

#[derive(Default, Clone, Debug, FromMeta, PartialEq, Eq)]
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
    //#[darling(default)]
    //pub physical: bool,
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
    Port,
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
            ReactorFieldKind::Port => {
                quote! { let __inner = (); }
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

        let field_inner_type = extract_path_ident(&ty).ok_or_else(|| {
            darling::Error::custom("Unable to extract path ident ").with_span(&ty)
        })?;

        // Heuristic to determine the field type based on the inner type and attributes
        match &ty {
            Type::Path(TypePath { path: _, .. }) => match field_inner_type.to_string().as_ref() {
                TIMER_ACTION_KEY => {
                    let timer = value.timer.unwrap_or_default();
                    Ok(ReactorField {
                        ident,
                        name,
                        ty,
                        kind: ReactorFieldKind::Timer {
                            period: timer.period,
                            offset: timer.offset,
                        },
                    })
                }

                TYPED_ACTION_KEY | PHYSICAL_ACTION_KEY => {
                    let min_delay = value.action.as_ref().and_then(|attr| attr.min_delay);
                    let policy = value.action.as_ref().and_then(|attr| attr.policy.clone());
                    Ok(ReactorField {
                        ident,
                        name,
                        ty,
                        kind: ReactorFieldKind::Action {
                            min_delay,
                            policy: policy.unwrap_or_default(),
                        },
                    })
                }

                TYPED_PORT_KEY => Ok(ReactorField {
                    ident,
                    name,
                    ty,
                    kind: ReactorFieldKind::Port,
                }),

                TYPED_REACTION_KEY => Ok(ReactorField {
                    ident,
                    name,
                    ty,
                    kind: ReactorFieldKind::Reaction,
                }),

                _ if matches!(value.child, Some(..)) => Ok(ReactorField {
                    ident,
                    name,
                    ty,
                    kind: ReactorFieldKind::Child {
                        state: value.child.unwrap(),
                    },
                }),

                _ => Err(darling::Error::custom("Unrecognized field type").with_span(&ident)),
            },

            _ => Err(
                darling::Error::custom("Unrecognized field type. Expected a path.").with_span(&ty),
            ),
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
    /// Reaction declarations
    #[darling(default, multiple, rename = "reaction")]
    pub reactions: Vec<syn::Expr>,
    /// Connection declarations
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

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::*;

    #[test]
    fn test_struct_attrs() {
        let input = r#"
#[derive(Reactor, Clone)]
#[reactor(
    state = MyType::Foo::<f32>,
    connection(from = "inp", to = "gain.inp"),
    connection(from = "gain.out", to = "out", after = "1 usec"),
    reaction = Reaction1,
    reaction = Reaction2,
)]
struct Test {}"#;

        let parsed = syn::parse_str(input).unwrap();
        let receiver = ReactorReceiver::from_derive_input(&parsed).unwrap();

        assert_eq!(receiver.ident.to_string(), "Test");
        assert_eq!(receiver.state, parse_quote! {MyType::Foo::<f32>});
        assert_eq!(receiver.connections.len(), 2);
        assert_eq!(
            receiver.connections[0],
            ConnectionAttr {
                from: parse_quote! {inp},
                to: parse_quote! {gain.inp},
                after: None
            }
        );
        assert_eq!(
            receiver.connections[1],
            ConnectionAttr {
                from: parse_quote! {gain.out},
                to: parse_quote! {out},
                after: Some(Duration::from_micros(1))
            }
        );
        assert_eq!(receiver.reactions.len(), 2);
        assert_eq!(receiver.reactions[0], parse_quote! {Reaction1});
        assert_eq!(receiver.reactions[1], parse_quote! {Reaction2});
    }

    #[test]
    fn test_actions() {
        let good_input = r#"
#[derive(Reactor, Clone)]
#[reactor(state = u32)]
struct Count {
    #[reactor(rename = "foo", timer(period = "1 usec"))]
    timer: TimerActionKey,
    action: TypedActionKey<u32>,
    #[reactor(action(min_delay = "1 usec"))]
    phys_action: PhysicalActionKey,
}"#;

        let parsed = syn::parse_str(good_input).unwrap();
        let receiver = ReactorReceiver::from_derive_input(&parsed).unwrap();

        let fields = receiver.data.take_struct().unwrap();

        let fields = fields
            .into_iter()
            .map(ReactorField::try_from)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(
            fields[0],
            ReactorField {
                ident: parse_quote! {timer},
                name: parse_quote! {foo},
                ty: parse_quote! {TimerActionKey},
                kind: ReactorFieldKind::Timer {
                    period: Some(Duration::from_micros(1)),
                    offset: None,
                }
            }
        );

        assert_eq!(
            fields[1],
            ReactorField {
                ident: parse_quote! {action},
                name: parse_quote! {action},
                ty: parse_quote! {TypedActionKey<u32>},
                kind: ReactorFieldKind::Action {
                    min_delay: None,
                    policy: ActionAttrPolicy::Defer,
                }
            }
        );

        assert_eq!(
            fields[2],
            ReactorField {
                ident: parse_quote! {phys_action},
                name: parse_quote! {phys_action},
                ty: parse_quote! {PhysicalActionKey},
                kind: ReactorFieldKind::Action {
                    min_delay: Some(Duration::from_micros(1)),
                    policy: ActionAttrPolicy::Defer,
                }
            }
        );
    }
}
