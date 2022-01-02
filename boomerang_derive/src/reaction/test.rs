use super::build_reaction_args;
use syn::{parse_quote, Attribute, ItemFn, NestedMeta};

#[test]
fn test_missing_timer() {
    let input: Attribute = parse_quote! {
        #[boomerang::reaction(triggers(startup, timer = "t"), uses(port = "x"))]
    };
    // let attr = ReactionAttr2::from_nested_meta(&input).unwrap();
    dbg!(&input.tokens.to_string());
    let attr: NestedMeta = syn::parse2(input.tokens).unwrap();
    dbg!(&attr);

    let x: ItemFn = syn::parse_quote! {
        fn reaction_startup(
            &mut self,
            _ctx: &runtime::Context,
            #[reaction(trigger, rename = "in")] inp: &runtime::Port<u32>,
            #[reaction(effect, rename = "out")] out: &mut runtime::Port<u32>,
        ) {}
    };

    // let ret = ReactorReceiver::from_derive_input(&input).and_then(|x| x.validate());
    // ret.expect_err("Testing expected error");
}

#[test]
fn test_build_reaction_args() {
    let mut input: ItemFn = syn::parse_quote! {
        fn reaction_startup(
            &mut self,
            _ctx: &runtime::Context,
            #[reactor::port(triggers, rename = "in")] inp: &runtime::Port<u32>,
            #[reactor::port(effects, rename = "out")] out: &mut runtime::Port<u32>,
            #[reactor::port(uses)] extra: &runtime::Port<bool>,
            #[reactor::action(triggers, effects)] a: &runtime::Action<()>,
        ) {}
    };

    let args = build_reaction_args(&mut input);
    dbg!(args);
}
