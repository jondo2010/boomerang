use super::*;
use syn::parse_quote;

#[test]
#[cfg(feature = "disabled")]
fn test_reaction() {
    let input = syn::parse_str(
        r#"
#[derive(Reactor)]
#[reactor(
    action(name="a"),
    reaction(function="Foo::bar", triggers(timer="tim1", "hello1.x"), effects(port="y", action="a")),
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
                effects: vec![
                    EffectAttr::Port(parse_quote!(self.y)),
                    EffectAttr::Action(parse_quote!(a)),
                ]
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
pub struct Foo {
    #[reactor(timer(rename="t1", offset="100 msec", period="1000 msec"),)]
    t: BuilderActionKey,
    #[reactor(timer(period="10 sec"),)]
    t2: BuilderActionKey,
}"#,
    )
    .unwrap();
    let receiver = ReactorReceiver::from_derive_input(&input).unwrap();

    // let attrs = receiver.data.take_struct().unwrap();
    dbg!(receiver);

    // assert_eq!(
    // attrs.fields[0],
    // TimerAttr {
    // rename: Some(parse_quote!(t1)),
    // offset: Some(Duration::from_millis(100)),
    // period: Some(Duration::from_millis(1000)),
    // });
    //
    // assert_eq!(
    // attrs.fields,
    // vec![
    // ,
    // TimerAttr {
    // name: parse_quote!(t2),
    // offset: None,
    // period: Some(Duration::from_secs(10)),
    // }
    // ]
    // );
}

#[test]
#[cfg(feature = "disabled")]
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
#[cfg(feature = "disabled")]
fn test_child() {
    let input = syn::parse_str(
        r#"
#[derive(Reactor)]
#[reactor(
    child(name="my_bar", reactor="Bar{}"),
    child(name="first_instance", reactor="HelloCpp::new(Duration::from_secs(4), \"Hello from first_instance.\")"),
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
    assert_eq!(
        receiver.children[1],
        ChildAttr {
            reactor: parse_quote!(HelloCpp::new(
                Duration::from_secs(4),
                "Hello from first_instance."
            )),
            name: parse_quote!(first_instance)
        }
    )
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
    assert_eq!(receiver.connections.len(), 3);
    assert_eq!(
        receiver.connections[0],
        ConnectionAttr {
            from: parse_quote!(x.y),
            to: parse_quote!(inp),
            after: None,
        }
    );
    assert_eq!(
        receiver.connections[1],
        ConnectionAttr {
            from: parse_quote!(o),
            to: parse_quote!(i),
            after: Some(Duration::from_secs(1)),
        }
    );
    assert_eq!(
        receiver.connections[2],
        ConnectionAttr {
            from: parse_quote!(in1),
            to: parse_quote!(gain.in1),
            after: None,
        }
    );
}

#[test]
#[cfg(feature = "disabled")]
fn test_missing_timer() {
    let input = syn::parse_str(
        r#"
#[derive(Debug, Reactor)]
#[reactor(
    reaction(
        function = "Count::reaction_t",
        triggers(timer = "t"),
    ),
)]
pub struct Foo {}
            "#,
    )
    .unwrap();
    let ret = ReactorReceiver::from_derive_input(&input).and_then(|x| x.validate());
    ret.expect_err("Testing expected error");
}

#[test]
fn test0() {
    let input = syn::parse_str(
        r#"
#[derive(Reactor)]
struct Foo {
    #[reactor(reaction(function = "Count::reaction_t"))]
    reaction_t: runtime::ReactionKey,
}"#,
    )
    .unwrap();
    let _ret = ReactorReceiver::from_derive_input(&input).unwrap();
}
