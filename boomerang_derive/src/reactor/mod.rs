use std::time::Duration;

mod output;
#[cfg(test)]
mod tests;

use crate::util;
use darling::{ast, FromDeriveInput, FromField, FromMeta};
use quote::format_ident;

#[derive(Debug, FromMeta, PartialEq, Eq)]
pub struct TimerAttr {
    pub rename: Option<syn::Ident>,
    #[darling(default, map = "util::handle_duration")]
    pub offset: Option<Duration>,
    #[darling(default, map = "util::handle_duration")]
    pub period: Option<Duration>,
}

#[derive(Debug, FromMeta, PartialEq, Eq)]
pub struct PortAttr {
    pub rename: Option<syn::Ident>,
}

#[derive(Debug, FromMeta, PartialEq, Eq, Copy, Clone, Default)]
pub enum ActionAttrPolicy {
    #[default]
    Defer,
    Drop,
}

#[derive(Debug, FromMeta, PartialEq, Eq)]
pub struct ActionAttr {
    pub rename: Option<syn::Ident>,
    #[darling(default)]
    pub physical: bool,
    #[darling(default, map = "util::handle_duration")]
    pub min_delay: Option<Duration>,
    #[darling(default, map = "util::handle_duration")]
    pub mit: Option<Duration>,
    #[darling(default)]
    pub policy: Option<ActionAttrPolicy>,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum TriggerAttr {
    Startup,
    Shutdown,
    Action(syn::Ident),
    Timer(syn::Ident),
    Port(syn::Path),
}

impl FromMeta for TriggerAttr {
    fn from_meta(item: &syn::Meta) -> darling::Result<Self> {
        (match *item {
            syn::Meta::Path(ref path) => path.segments.first().map_or_else(
                || Err(darling::Error::unsupported_shape("something wierd")),
                |path| match path.ident.to_string().as_ref() {
                    "startup" => Ok(TriggerAttr::Startup),
                    "shutdown" => Ok(TriggerAttr::Shutdown),
                    __other => Err(darling::Error::unknown_field_with_alts(
                        __other,
                        &["startup", "shutdown"],
                    )
                    .with_span(&path.ident)),
                },
            ),
            syn::Meta::List(ref value) => Self::from_list(
                &value
                    .nested
                    .iter()
                    .cloned()
                    .collect::<Vec<syn::NestedMeta>>()[..],
            ),
            syn::Meta::NameValue(ref value) => value
                .path
                .segments
                .first()
                .map(|path| match path.ident.to_string().as_ref() {
                    "action" => {
                        let value = darling::FromMeta::from_value(&value.lit)?;
                        Ok(TriggerAttr::Action(value))
                    }
                    "timer" => {
                        let value = darling::FromMeta::from_value(&value.lit)?;
                        Ok(TriggerAttr::Timer(value))
                    }
                    "port" => {
                        let value = darling::FromMeta::from_value(&value.lit)?;
                        Ok(TriggerAttr::Port(value))
                    }
                    __other => Err(darling::Error::unknown_field_with_alts(
                        __other,
                        &["action", "timer", "port"],
                    )
                    .with_span(&path.ident)),
                })
                .expect("oopsie"),
        })
        .map_err(|e| e.with_span(item))
    }

    fn from_string(value: &str) -> darling::Result<Self> {
        let value = darling::FromMeta::from_string(value)?;
        Ok(TriggerAttr::Port(value))
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum EffectAttr {
    Action(syn::Ident),
    Port(util::NamedField),
}

impl FromMeta for EffectAttr {
    fn from_meta(item: &syn::Meta) -> darling::Result<Self> {
        (match *item {
            syn::Meta::NameValue(ref value) => value
                .path
                .segments
                .first()
                .ok_or_else(|| darling::Error::unknown_field_path(&value.path).with_span(item))
                .and_then(|path| match path.ident.to_string().as_ref() {
                    "action" => {
                        let value = darling::FromMeta::from_value(&value.lit)?;
                        Ok(EffectAttr::Action(value))
                    }
                    "port" => {
                        let value = darling::FromMeta::from_value(&value.lit)?;
                        Ok(EffectAttr::Port(value))
                    }
                    __other => Err(darling::Error::unknown_field_with_alts(
                        __other,
                        &["action", "port"],
                    )
                    .with_span(&path.ident)),
                }),
            _ => Err(darling::Error::unexpected_type("oops").with_span(item)),
        })
        .map_err(|e| e.with_span(item))
    }
}

#[derive(Debug, FromMeta, PartialEq, Eq)]
pub struct ChildAttr {
    /// The instance name of the child
    pub rename: Option<syn::Ident>,
    /// An expression resulting in a Reactor
    pub state: syn::Expr,
}

/// Attribute on a Reaction field in a ReactorBuilder struct
#[derive(Debug, FromMeta, PartialEq, Eq)]
pub struct ReactionAttr {
    pub function: syn::Path,
}

/// Attributes on fields in a Reactor
#[derive(Debug, FromField, PartialEq, Eq)]
#[darling(attributes(reactor), forward_attrs(doc, cfg, allow))]
pub struct RawReactorField {
    pub ident: Option<syn::Ident>,
    pub vis: syn::Visibility,
    pub ty: syn::Type,
    pub timer: Option<TimerAttr>,
    pub input: Option<PortAttr>,
    pub output: Option<PortAttr>,
    pub action: Option<ActionAttr>,
    pub child: Option<ChildAttr>,
    pub reaction: Option<ReactionAttr>,
}

pub enum ReactorField<'a> {
    Timer {
        ident: &'a syn::Ident,
        name: String,
        period: Duration,
        offset: Duration,
    },
    Input {
        ident: &'a syn::Ident,
        name: String,
        ty: &'a syn::Type,
    },
    Output {
        ident: &'a syn::Ident,
        name: String,
        ty: &'a syn::Type,
    },
    Action {
        ident: &'a syn::Ident,
        name: String,
        ty: &'a syn::Type,
        physical: bool,
        min_delay: &'a Option<Duration>,
        mit: Duration,
        policy: ActionAttrPolicy,
    },
    Child {
        ident: &'a syn::Ident,
        name: String,
        state: &'a syn::Expr,
        ty: &'a syn::Type,
    },
    Reaction {
        ident: &'a syn::Ident,
        path: syn::Path,
    },
}

impl<'a> ReactorField<'a> {
    pub fn get_ident(&self) -> &syn::Ident {
        match self {
            ReactorField::Timer { ident, .. } => ident,
            ReactorField::Input { ident, .. } => ident,
            ReactorField::Output { ident, .. } => ident,
            ReactorField::Action { ident, .. } => ident,
            ReactorField::Child { ident, .. } => ident,
            ReactorField::Reaction { ident, .. } => ident,
        }
    }
}

impl<'a> From<&'a RawReactorField> for ReactorField<'a> {
    fn from(field: &'a RawReactorField) -> Self {
        match field {
            RawReactorField {
                ident: Some(ident),
                timer:
                    Some(TimerAttr {
                        rename,
                        offset,
                        period,
                    }),
                input: None,
                output: None,
                action: None,
                child: None,
                reaction: None,
                ..
            } => {
                let name = rename.as_ref().unwrap_or(ident).to_string();
                ReactorField::Timer {
                    ident,
                    name,
                    period: period.unwrap_or_default(),
                    offset: offset.unwrap_or_default(),
                }
            }

            RawReactorField {
                ident: Some(ident),
                ty,
                timer: None,
                input: Some(PortAttr { rename }),
                output: None,
                action: None,
                child: None,
                reaction: None,
                ..
            } => {
                let name = rename.as_ref().unwrap_or(ident).to_string();
                ReactorField::Input { ident, name, ty }
            }

            RawReactorField {
                ident: Some(ident),
                ty,
                timer: None,
                input: None,
                output: Some(PortAttr { rename }),
                action: None,
                child: None,
                reaction: None,
                ..
            } => {
                let name = rename.as_ref().unwrap_or(ident).to_string();
                ReactorField::Output { ident, name, ty }
            }

            RawReactorField {
                ident: Some(ident),
                ty,
                timer: None,
                input: None,
                output: None,
                action:
                    Some(ActionAttr {
                        rename,
                        physical,
                        min_delay,
                        mit,
                        policy,
                    }),
                child: None,
                reaction: None,
                ..
            } => {
                // let ty = quote! {<#ty as ::boomerang::runtime::AssociatedItem>::Inner};
                let name = rename.as_ref().unwrap_or(ident).to_string();
                ReactorField::Action {
                    ident,
                    name,
                    ty,
                    physical: *physical,
                    min_delay,
                    mit: mit.unwrap_or_default(),
                    policy: policy.unwrap_or_default(),
                }
            }

            RawReactorField {
                ident: Some(ident),
                ty,
                timer: None,
                input: None,
                output: None,
                action: None,
                child: Some(ChildAttr { rename, state }),
                reaction: None,
                ..
            } => {
                let name = rename.as_ref().unwrap_or(ident).to_string();
                ReactorField::Child {
                    ident,
                    name,
                    state,
                    ty,
                }
            }

            RawReactorField {
                ident: Some(ident),
                timer: None,
                input: None,
                output: None,
                action: None,
                child: None,
                reaction: Some(ReactionAttr { function }),
                ..
            } => {
                let mut path = function.clone();
                let seg = path.segments.last_mut().unwrap();
                seg.ident = format_ident!("__build_{}", seg.ident);
                ReactorField::Reaction { ident, path }
            }

            _ => {
                panic!("Shouldn't happen.")
            }
        }
    }
}

#[derive(Debug, FromMeta, Eq, PartialEq)]
pub struct ConnectionAttr {
    from: syn::Expr,
    to: syn::Expr,
    #[darling(default, map = "util::handle_duration")]
    pub after: Option<Duration>,
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reactor), supports(struct_named))]
pub struct ReactorReceiver {
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    // pub attrs: Vec<syn::Attribute>,
    pub data: ast::Data<darling::util::Ignored, RawReactorField>,
    /// Type of the reactor state
    state: Option<syn::Type>,
    /// Connection definitions
    #[darling(default, multiple, rename = "connection")]
    pub connections: Vec<ConnectionAttr>,
}

#[cfg(feature = "disabled")]
impl ReactorReceiver {
    fn validate_port(&self, named_field: &NamedField, errors: &mut darling::error::Accumulator) {
        let NamedField(reactor, port) = named_field;
        let slf = proc_macro2::Ident::new("self", proc_macro2::Span::call_site());

        if *reactor == slf {
            if self
                .inputs
                .iter()
                .chain(self.outputs.iter())
                .find(|&input| &input.name == port)
                .is_none()
            {
                errors.push(
                    darling::Error::custom(format!(
                        "Port '{}' not found in Reactor definition.",
                        port
                    ))
                    .with_span(port),
                );
            }
        } else {
            if self
                .children
                .iter()
                .find(|&child| &child.name == reactor)
                .is_none()
            {
                let children = itertools::join(self.children.iter().map(|child| &child.name), ", ");
                errors.push(
                    darling::Error::custom(format!(
                        "Child Reactor '{}' not found in [{}]",
                        reactor, children
                    ))
                    .with_span(port),
                );
            }
        }
    }

    fn validate_action(
        &self,
        action: &proc_macro2::Ident,
        errors: &mut darling::error::Accumulator,
    ) {
        if self.actions.iter().find(|&a| a.name == *action).is_none() {
            errors.push(
                darling::Error::custom(format!("Action '{}' not found.", action)).with_span(action),
            );
        };
    }

    pub fn validate(self) -> Result<Self, darling::Error> {
        let mut errors = darling::Error::accumulator();

        for reaction in self.reactions.iter() {
            for trigger in reaction.triggers.iter() {
                match trigger {
                    TriggerAttr::Startup => {}
                    TriggerAttr::Shutdown => {}
                    TriggerAttr::Action(ref action) => {
                        self.validate_action(action, &mut errors);
                    }
                    TriggerAttr::Timer(ref timer) => {
                        if self.timers.iter().find(|&t| t.name == *timer).is_none() {
                            errors.push(
                                darling::Error::custom(format!("Timer '{}' not found.", timer))
                                    .with_span(timer),
                            );
                        }
                    }
                    TriggerAttr::Port(ref port) => {
                        self.validate_port(port, &mut errors);
                    }
                }
            }

            for effect in reaction.effects.iter() {
                match effect {
                    EffectAttr::Action(ref action) => {
                        self.validate_action(action, &mut errors);
                    }
                    EffectAttr::Port(ref port) => {
                        self.validate_port(port, &mut errors);
                    }
                }
            }
        }

        for connection in self.connections.iter() {
            self.validate_port(&connection.from, &mut errors);
            self.validate_port(&connection.to, &mut errors);
        }

        errors.finish_with(self)
    }
}
