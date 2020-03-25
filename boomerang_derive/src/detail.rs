//! Details of implementation for derive macros and attributes

use crate::util::{ExprField, ExprFieldList, StringList};
use darling::{ast, util, FromDeriveInput, FromField, FromMeta};

#[derive(Debug, Default, FromMeta, PartialEq)]
pub struct TimerField {
    pub name: String,
    pub offset: String,
    pub period: String,
}

#[derive(Debug, FromMeta, PartialEq)]
pub struct ReactionField {
    pub function: syn::Path,
    #[darling(default, map = "ExprFieldList::into")]
    pub triggers: Vec<syn::ExprField>,
    #[darling(default, map = "StringList::into")]
    pub uses: Vec<String>,
    #[darling(default, map = "StringList::into")]
    pub effects: Vec<String>,
}

#[derive(Debug, FromMeta, PartialEq)]
pub struct ChildField {
    pub reactor: syn::Path,
    pub name: String,
    #[darling(default, map = "ExprFieldList::into")]
    pub inputs: Vec<syn::ExprField>,
    #[darling(default, map = "StringList::into")]
    pub outputs: Vec<String>,
}

#[derive(Debug, FromMeta, PartialEq)]
pub struct ConnectionField {
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

    #[darling(default)]
    pub input: bool,
    #[darling(default)]
    pub output: bool,
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
    pub timers: Vec<TimerField>,
    /// Reaction definitions for this reactor definition
    #[darling(default, multiple, rename = "reaction")]
    pub reactions: Vec<ReactionField>,
    /// Child reactor instance definitions
    #[darling(default, multiple, rename = "child")]
    pub children: Vec<ChildField>,
    /// Connection definitions
    #[darling(default, multiple, rename = "connection")]
    pub connections: Vec<ConnectionField>,
}

#[test]
fn test_reaction() {
    use syn::parse_quote;
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
            ReactionField {
                function: parse_quote!(Foo::bar),
                triggers: vec![parse_quote!(self.tim1), parse_quote!(hello1.x)],
                uses: vec![],
                effects: vec!["y".into()]
            },
            ReactionField {
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
        timer(name="t", offset="Duration::from_millis(100)", period="Duration::from_millis(1000)")
    )]
    pub struct Foo {}"#,
    )
    .unwrap();
    let receiver = ReactorReceiver::from_derive_input(&input).unwrap();
    assert_eq!(
        receiver.timers[0],
        TimerField {
            name: "t".into(),
            offset: "Duration::from_millis(100)".into(),
            period: "Duration::from_millis(1000)".into()
        }
    );
}

#[test]
fn test_child() {
    use syn::parse_quote;
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
        ChildField {
            reactor: parse_quote!(Bar),
            name: "my_bar".into(),
            inputs: vec![parse_quote!(x.y), parse_quote!(self.y)],
            outputs: vec!["b".into()],
        }
    );
}

#[test]
fn test_connection() {
    use syn::parse_quote;
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
        vec![ConnectionField {
            from: parse_quote!(x.y),
            to: parse_quote!(self.inp),
        }]
    );
}
