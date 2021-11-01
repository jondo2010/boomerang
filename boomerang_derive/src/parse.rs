//! Details of implementation for derive macros and attributes
//! The `Reactor` derive macro is parsed into a `ReactorReceiver` struct for further
//! processing.

use crate::util::{handle_ident, Expr, MetaList, NamedField, Type};
use darling::{ast, util, FromDeriveInput, FromField, FromMeta, ToTokens};
use derive_more::Display;
use std::time::Duration;

fn handle_duration(value: String) -> Option<Duration> {
    Some(parse_duration::parse(&value).unwrap())
}

#[derive(Debug, FromMeta, PartialEq, Eq, Hash, Ord, PartialOrd, Display)]
#[display(fmt = "{}", "name.to_string()")]
pub struct TimerAttr {
    #[darling(map = "handle_ident")]
    pub name: syn::Ident,
    #[darling(default, map = "handle_duration")]
    pub offset: Option<Duration>,
    #[darling(default, map = "handle_duration")]
    pub period: Option<Duration>,
}

#[derive(Debug, FromMeta, PartialEq, Eq, Hash, Display)]
#[display(fmt = "{}", "name.to_string()")]
pub struct PortAttr {
    #[darling(map = "handle_ident")]
    pub name: syn::Ident,
    #[darling(rename = "type", map = "Type::into")]
    pub ty: syn::Type,
}

impl PartialOrd for PortAttr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.name.partial_cmp(&other.name)
    }
}

impl Ord for PortAttr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

#[derive(Debug, FromMeta, PartialEq, Eq, Hash, Display)]
pub enum ActionAttrPolicy {
    Defer,
    Drop,
}

fn type_default() -> syn::Type {
    syn::parse_quote!(())
}

#[derive(Debug, FromMeta, PartialEq, Eq, Hash, Display)]
#[display(fmt = "{}", "name.to_string()")]
pub struct ActionAttr {
    #[darling(map = "handle_ident")]
    pub name: syn::Ident,
    #[darling(default = "type_default", rename = "type", map = "Type::into")]
    pub ty: syn::Type,
    #[darling(default)]
    pub physical: bool,
    #[darling(default, map = "handle_duration")]
    pub min_delay: Option<Duration>,
    #[darling(default, map = "handle_duration")]
    pub mit: Option<Duration>,
    #[darling(default)]
    pub policy: Option<ActionAttrPolicy>,
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub enum TriggerAttr {
    Startup,
    Shutdown,
    Timer(syn::Ident),
    Port(NamedField),
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
                        &["timer", "port"],
                    )
                    .with_span(&path.ident)),
                })
                .unwrap(),
        })
        .map_err(|e| e.with_span(item))
    }

    fn from_string(value: &str) -> darling::Result<Self> {
        let value = darling::FromMeta::from_string(value)?;
        Ok(TriggerAttr::Port(value))
    }
}

#[derive(Debug, FromMeta, Eq, PartialEq, Hash, Display)]
#[display(fmt = "{}", "function.segments.last().unwrap().ident.to_string()")]
pub struct ReactionAttr {
    pub function: syn::Path,
    #[darling(default, map = "MetaList::into")]
    pub triggers: Vec<TriggerAttr>,
    #[darling(default, map = "MetaList::into")]
    pub uses: Vec<NamedField>,
    #[darling(default, map = "MetaList::into")]
    pub effects: Vec<NamedField>,
}

impl PartialOrd for ReactionAttr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let f1 = self.function.to_token_stream().to_string();
        let f2 = other.function.to_token_stream().to_string();
        f1.partial_cmp(&f2)
    }
}

impl Ord for ReactionAttr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let f1 = self.function.to_token_stream().to_string();
        let f2 = other.function.to_token_stream().to_string();
        f1.cmp(&f2)
    }
}

#[derive(Debug, FromMeta, PartialEq, Eq)]
pub struct ChildAttr {
    /// The instance name of the child
    #[darling(map = "handle_ident")]
    pub name: syn::Ident,
    /// An expression resulting in a Reactor
    #[darling(map = "Expr::into")]
    pub reactor: syn::Expr,
}

impl PartialOrd for ChildAttr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.name.partial_cmp(&other.name)
    }
}

impl Ord for ChildAttr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

#[derive(Debug, FromMeta, PartialEq)]
pub struct ConnectionAttr {
    pub from: NamedField,
    pub to: NamedField,
    #[darling(default, map = "handle_duration")]
    pub after: Option<Duration>,
}

#[derive(Debug, FromField)]
#[darling(attributes(reactor), forward_attrs(doc, cfg, allow))]
pub struct ReactorField {
    pub ident: Option<syn::Ident>,
    pub vis: syn::Visibility,
    pub ty: syn::Type,
    attrs: Vec<syn::Attribute>,
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reactor), supports(struct_named))]
pub struct ReactorReceiver {
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    // pub attrs: Vec<syn::Attribute>,
    pub data: ast::Data<util::Ignored, ReactorField>,

    /// Timer definitions for this reactor definition
    #[darling(default, multiple, rename = "timer")]
    pub timers: Vec<TimerAttr>,
    /// Input port definitions
    #[darling(multiple, rename = "input")]
    pub inputs: Vec<PortAttr>,
    /// Output port definitions
    #[darling(multiple, rename = "output")]
    pub outputs: Vec<PortAttr>,
    /// Action definitions
    #[darling(multiple, rename = "action")]
    pub actions: Vec<ActionAttr>,
    /// Reaction definitions for this reactor definition
    #[darling(default, multiple, rename = "reaction")]
    pub reactions: Vec<ReactionAttr>,
    /// Child reactor instance definitions
    #[darling(default, multiple, rename = "child")]
    pub children: Vec<ChildAttr>,
    /// Connection definitions
    #[darling(default, multiple, rename = "connection")]
    pub connections: Vec<ConnectionAttr>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;
    #[test]
    fn test_reaction() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    reaction(function="Foo::bar", triggers(timer="tim1", "hello1.x"), effects("y")),
    reaction(function="Foo::rab", triggers(startup, shutdown, "self.i"))
)]
pub struct Foo {}"#,
        )
        .unwrap();
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        assert_eq!(
            receiver.reactions,
            vec![
                ReactionAttr {
                    function: parse_quote!(Foo::bar),
                    triggers: vec![
                        TriggerAttr::Timer(parse_quote!(tim1)),
                        TriggerAttr::Port(parse_quote!(hello1.x)),
                    ],
                    uses: vec![],
                    effects: vec![parse_quote!(self.y)]
                },
                ReactionAttr {
                    function: parse_quote!(Foo::rab),
                    triggers: vec![
                        TriggerAttr::Startup,
                        TriggerAttr::Shutdown,
                        TriggerAttr::Port(parse_quote!(self.i))
                    ],
                    uses: vec![],
                    effects: vec![]
                }
            ]
        );
    }

    #[test]
    fn test_timer() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    timer(name="t1", offset="100 msec", period="1000 msec"),
    timer(name="t2", period="10 sec"),
)]
pub struct Foo {}"#,
        )
        .unwrap();
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        assert_eq!(
            receiver.timers,
            vec![
                TimerAttr {
                    name: parse_quote!(t1),
                    offset: Some(Duration::from_millis(100)),
                    period: Some(Duration::from_millis(1000)),
                },
                TimerAttr {
                    name: parse_quote!(t2),
                    offset: None,
                    period: Some(Duration::from_secs(10)),
                }
            ]
        );
    }

    #[test]
    fn test_ports() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    input(name="in", type="u32"),
    input(name="in1", type="u32"),
    output(name="out1", type="Vec<u32>"),
    action(name="action1", physical="true", min_delay="1 sec", policy="drop"),
    action(name="action2", mit="1 msec"),
)]
pub struct Foo {}"#,
        )
        .unwrap();
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        assert_eq!(
            receiver.inputs,
            vec![
                PortAttr {
                    name: syn::Ident::new("in", proc_macro2::Span::call_site()),
                    ty: parse_quote!(u32),
                },
                PortAttr {
                    name: parse_quote!(in1),
                    ty: parse_quote!(u32),
                },
            ]
        );
        assert_eq!(
            receiver.outputs,
            vec![PortAttr {
                name: parse_quote!(out1),
                ty: parse_quote!(Vec<u32>),
            }]
        );
        assert_eq!(
            receiver.actions,
            vec![
                ActionAttr {
                    name: parse_quote!(action1),
                    physical: true,
                    min_delay: Some(Duration::from_secs(1)),
                    mit: None,
                    policy: Some(ActionAttrPolicy::Drop),
                    ty: parse_quote!(()),
                },
                ActionAttr {
                    name: parse_quote!(action2),
                    physical: false,
                    min_delay: None,
                    mit: Some(Duration::from_millis(1)),
                    policy: None,
                    ty: parse_quote!(()),
                }
            ]
        )
    }

    #[test]
    fn test_child() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    child(name="my_bar", reactor="Bar{}"),
)]
pub struct Foo {}"#,
        )
        .unwrap();
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        assert_eq!(
            receiver.children[0],
            ChildAttr {
                reactor: parse_quote!(Bar {}),
                name: parse_quote!(my_bar),
            }
        );
    }

    #[test]
    fn test_connection() {
        let input = syn::parse_str(
            r#"
#[derive(Reactor)]
#[reactor(
    connection(from="x.y", to="inp"),
    connection(from="o", to="i", after="1 sec"),
    connection(from="in1", to="gain.in1"),
)]
pub struct Foo {}"#,
        )
        .unwrap();
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        assert_eq!(
            receiver.connections,
            vec![
                ConnectionAttr {
                    from: parse_quote!(x.y),
                    to: parse_quote!(self.inp),
                    after: None,
                },
                ConnectionAttr {
                    from: parse_quote!(self.o),
                    to: parse_quote!(self.i),
                    after: Some(Duration::from_secs(1)),
                },
                ConnectionAttr {
                    from: parse_quote!(self.in1),
                    to: parse_quote!(gain.in1),
                    after: None,
                }
            ]
        );
    }
}
