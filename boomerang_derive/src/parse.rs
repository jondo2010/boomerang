//! Details of implementation for derive macros and attributes
//! The `Reactor` derive macro is parsed into a `ReactorReceiver` struct for further
//! processing.

use crate::util::{ExprField, ExprFieldList, Type};
use darling::{ast, util, FromDeriveInput, FromField, FromMeta, ToTokens};
use derive_more::Display;
use std::time::Duration;

fn handle_duration(value: String) -> Option<Duration> {
    Some(parse_duration::parse(&value).unwrap())
}

#[derive(Debug, FromMeta, PartialEq, Eq, Hash, Ord, PartialOrd, Display)]
#[display(fmt = "Timer: '{}'", "name.to_string()")]
pub struct TimerAttr {
    pub name: syn::Ident,
    #[darling(default, map = "handle_duration")]
    pub offset: Option<Duration>,
    #[darling(default, map = "handle_duration")]
    pub period: Option<Duration>,
}

#[derive(Debug, FromMeta, PartialEq, Eq, Hash, Display)]
#[display(fmt = "Port: '{}'", "name.to_string()")]
pub struct PortAttr {
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

#[derive(Debug, FromMeta, Eq, PartialEq, Hash, Display)]
#[display(fmt = "'{}'", "function.to_token_stream().to_string()")]
pub struct ReactionAttr {
    pub function: syn::Path,
    #[darling(default, map = "ExprFieldList::into")]
    pub triggers: Vec<syn::ExprField>,
    #[darling(default, map = "ExprFieldList::into")]
    pub uses: Vec<syn::ExprField>,
    #[darling(default, map = "ExprFieldList::into")]
    pub effects: Vec<syn::ExprField>,
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

#[derive(Debug, FromMeta, PartialEq)]
pub struct ChildAttr {
    pub reactor: syn::Path,
    pub name: String,
    #[darling(default, map = "ExprFieldList::into")]
    pub inputs: Vec<syn::ExprField>,
    #[darling(default, map = "ExprFieldList::into")]
    pub outputs: Vec<syn::ExprField>,
}

#[derive(Debug, FromMeta, PartialEq)]
pub struct ConnectionAttr {
    #[darling(map = "ExprField::into")]
    pub from: syn::ExprField,
    #[darling(map = "ExprField::into")]
    pub to: syn::ExprField,
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
        reaction(function="Foo::bar", triggers("tim1", "hello1.x"), uses(), effects("y")),
        reaction(function="Foo::rab", triggers("i")),
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
                    triggers: vec![parse_quote!(self.tim1), parse_quote!(hello1.x)],
                    uses: vec![],
                    effects: vec![parse_quote!(self.y)]
                },
                ReactionAttr {
                    function: parse_quote!(Foo::rab),
                    triggers: vec![parse_quote!(self.i)],
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
        input(name="in1", type="u32"),
        output(name="out1", type="Vec<u32>"),
    )]
    pub struct Foo {}"#,
        )
        .unwrap();
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        assert_eq!(
            receiver.inputs,
            vec![PortAttr {
                name: parse_quote!(in1),
                ty: parse_quote!(u32),
            },]
        );
        assert_eq!(
            receiver.outputs,
            vec![PortAttr {
                name: parse_quote!(out1),
                ty: parse_quote!(Vec<u32>),
            }]
        )
    }

    #[test]
    fn test_child() {
        let input = syn::parse_str(
            r#"
    #[derive(Reactor)]
    #[reactor(
        child(reactor="Bar", name="my_bar", inputs("x.y", "y"), outputs("b")),
    )]
    pub struct Foo {}"#,
        )
        .unwrap();
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        assert_eq!(
            receiver.children[0],
            ChildAttr {
                reactor: parse_quote!(Bar),
                name: "my_bar".into(),
                inputs: vec![parse_quote!(x.y), parse_quote!(self.y)],
                outputs: vec![parse_quote!(self.b)],
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
    )]
    pub struct Foo {}"#,
        )
        .unwrap();
        let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
        assert_eq!(
            receiver.connections,
            vec![ConnectionAttr {
                from: parse_quote!(x.y),
                to: parse_quote!(self.inp),
            }]
        );
    }
}
