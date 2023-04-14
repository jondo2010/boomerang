use super::*;

#[test]
fn test_duplicate_ports() {
    let mut env_builder = EnvBuilder::new();
    let reactor_key = env_builder
        .add_reactor("test_reactor", None, ())
        .finish()
        .unwrap();
    let _ = env_builder
        .add_port::<()>("port0", PortType::Input, reactor_key)
        .unwrap();

    assert!(matches!(
        env_builder
            .add_port::<()>("port0", PortType::Output, reactor_key)
            .expect_err("Expected duplicate"),
        BuilderError::DuplicatePortDefinition {
            reactor_name,
            port_name
        } if reactor_name == "test_reactor" && port_name == "port0"
    ));
}

#[test]
fn test_duplicate_actions() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, ());

    reactor_builder
        .add_logical_action::<()>("action0", None)
        .unwrap();

    assert!(matches!(
        reactor_builder
            .add_logical_action::<()>("action0", None)
            .expect_err("Expected duplicate"),
        BuilderError::DuplicateActionDefinition {
            reactor_name,
            action_name,
        } if reactor_name== "test_reactor" && action_name == "action0"
    ));

    assert!(matches!(
        reactor_builder
            .add_timer(
                "action0",
                Some(Duration::from_micros(0)),
                Some(Duration::from_micros(0)),
            )
            .expect_err("Expected duplicate"),
        BuilderError::DuplicateActionDefinition {
            reactor_name,
            action_name,
        } if reactor_name == "test_reactor" && action_name == "action0"
    ));
}

#[test]
fn test_reactions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, ());

    let r0_key = reactor_builder
        .add_reaction("test", Box::new(|_ctx, _r, _i, _o, _a| {}))
        .finish()
        .unwrap();

    let r1_key = reactor_builder
        .add_reaction("test", Box::new(|_ctx, _r, _i, _o, _a| {}))
        .finish()
        .unwrap();

    let _reactor_key = reactor_builder.finish().unwrap();

    assert_eq!(env_builder.reactor_builders.len(), 1);
    assert_eq!(env_builder.reaction_builders.len(), 2);
    assert_eq!(
        env_builder.reaction_builders.keys().collect::<Vec<_>>(),
        vec![r0_key, r1_key]
    );

    // assert_eq!(env_builder.reactors[reactor_key].reactions.len(), 2);

    let env: runtime::Env = env_builder.try_into().unwrap();
    assert_eq!(env.reactions.len(), 2);
}

#[cfg(feature = "disabled")]
mod experiment {
    trait IntoReaction {
        type Reactor;
        // type Actions;
        fn into_reaction_fn(self) -> Box<dyn runtime::ReactionFn>;
    }

    struct SA<'a> {
        a: &'a mut runtime::Action,
    }

    #[derive(Debug)]
    struct Test {}
    impl Test {
        fn reaction_a(&mut self, ctx: &mut runtime::Context, actions: SA) {}

        fn reaction_b(&mut self, ctx: &mut runtime::Context, actions: SA) {}

        fn reaction_c<const N: usize>(&mut self, ctx: &mut runtime::Context) {}
    }

    trait TestA {
        fn reaction_a(&mut self, ctx: &mut runtime::Context, actions: SA);
    }

    impl TestA for Test {
        fn reaction_a(&mut self, ctx: &mut runtime::Context, actions: SA) {
            todo!()
        }
    }

    impl<F> IntoReaction for F
    where
        F: for<'a> Fn(&mut Test, &mut runtime::Context, SA<'a>) + Send + Sync + 'static,
    {
        type Reactor = Test;
        // type Actions = SA<'a>;
        fn into_reaction_fn(self) -> Box<dyn runtime::ReactionFn> {
            Box::new(
                move |ctx, reactor, inputs, outputs, trig_actions, sched_actions| {
                    let reactor = reactor.downcast_mut().unwrap();
                    let [a]: &mut [_; 1usize] =
                        std::convert::TryInto::try_into(sched_actions).unwrap();
                    let actions = SA { a };
                    (self)(reactor, ctx, actions);
                },
            )
        }
    }

    #[test]
    fn tester() {
        let t = Test {};
        let x = IntoReaction::into_reaction_fn(Test::reaction_a);
        let y = Test::reaction_b.into_reaction_fn();
    }
}

#[cfg(feature = "disabled")]
#[test]
fn test_actions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, ());

    let action_a = reactor_builder
        .add_logical_action::<()>("a", Some(Duration::from_secs(1)))
        .unwrap();
    let action_b = reactor_builder.add_logical_action::<()>("b", None).unwrap();

    // Triggered by a+b, schedules b
    let reaction_a = reactor_builder
        .add_reaction(
            "ra",
            Box::new(|_, _, _, _, _, sa| {
                let [a]: &mut [_; 1] = ::std::convert::TryInto::try_into(sa).unwrap();

                let x = SA { a };
            }),
        )
        .with_trigger_action(action_a, 0)
        .with_schedulable_action(action_b, 0)
        .with_trigger_action(action_b, 1)
        .finish()
        .unwrap();

    // Triggered by a, schedules a
    let reaction_b = reactor_builder
        .add_reaction("rb", Box::new(|_, _, _, _, _, _| {}))
        .with_trigger_action(action_a, 0)
        .with_schedulable_action(action_a, 0)
        .finish()
        .unwrap();

    let _reactor_key = reactor_builder.finish().unwrap();
    let (env, dep_info) = env_builder.try_into().unwrap();

    runtime::check_consistency(&env, &dep_info);

    assert_eq!(env.actions[action_a].get_name(), "a");
    assert_eq!(env.actions[action_a].get_is_logical(), true);

    // An action both triggered by and scheduled-by should only show up in the
    // reaction_sched_actions
    assert_eq!(dep_info.reaction_trig_actions[reaction_a], vec![action_a]);
    assert_eq!(dep_info.reaction_sched_actions[reaction_a], vec![action_b]);
    assert_eq!(dep_info.reaction_trig_actions[reaction_b], vec![]);
    assert_eq!(dep_info.reaction_sched_actions[reaction_b], vec![action_a]);
    assert_eq!(
        dep_info.action_triggers[action_a],
        vec![reaction_a, reaction_b]
    );
    assert_eq!(dep_info.action_triggers[action_b], vec![reaction_a]);
}
