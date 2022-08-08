use crate::{
    reaction::{ActionAttrs, ArgumentAttr, PortAttrs, ReactionArg},
    reactor::TriggerAttr,
};

use super::{ReactionAttr, ReactionReceiver};
use darling::FromMeta;
use itertools::Itertools;
use quote::format_ident;
use syn::{parse_quote, Attribute, ItemFn};

#[test]
fn test_reaction_receiver() {
    let input: Attribute = parse_quote! {
        #[boomerang::reaction(
            reactor = "TestBuilder",
            triggers(action = "tim", startup)
        )]
    };
    let input_meta = input.parse_meta().unwrap();
    let attr = ReactionAttr::from_meta(&input_meta).unwrap();

    assert_eq!(attr.reactor, parse_quote!(TestBuilder));
    assert_eq!(
        attr.triggers,
        vec![
            TriggerAttr::Action(format_ident!("tim")),
            TriggerAttr::Startup
        ]
    );

    let itemfn: ItemFn = syn::parse_quote! {
        #[boomerang::reaction(
            reactor = "TestBuilder"
        )]
        fn reaction_test(
            &mut self,
            _ctx: &runtime::Context,
            #[reactor::port(triggers)] x: &runtime::Port<i32>,
            #[reactor::port(effects, path = "out")] out: &mut runtime::Port<u32>,
            #[reactor::port(uses)] extra: &runtime::Port<bool>,
            #[reactor::action(triggers, effects, rename = "act")] a: &runtime::Action<()>,
        ) {}
    };

    let recv = ReactionReceiver::from_attr_itemfn(attr, itemfn).unwrap();
    assert_eq!(
        recv.args,
        vec![
            ReactionArg {
                ident: format_ident!("x"),
                attr: ArgumentAttr::Port {
                    attrs: PortAttrs::Triggers,
                    path: None
                },
                ty: parse_quote!(runtime::Port<i32>),
            },
            ReactionArg {
                ident: format_ident!("out"),
                attr: ArgumentAttr::Port {
                    attrs: PortAttrs::Effects,
                    path: Some(parse_quote!(out))
                },
                ty: parse_quote!(runtime::Port<u32>),
            },
            ReactionArg {
                ident: format_ident!("extra"),
                attr: ArgumentAttr::Port {
                    attrs: PortAttrs::Uses,
                    path: None
                },
                ty: parse_quote!(runtime::Port<bool>),
            },
            ReactionArg {
                ident: format_ident!("a"),
                attr: ArgumentAttr::Action {
                    attrs: ActionAttrs::TriggersAndEffects,
                    rename: Some(parse_quote!(act))
                },
                ty: parse_quote!(runtime::Action<()>),
            },
        ]
    );

    // Test output methods
    assert_eq!(
        recv.actions_idents().collect_vec(),
        vec![
            format_ident!("tim"),
            format_ident!("_startup"),
            format_ident!("a")
        ]
    );
}
