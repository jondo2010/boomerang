use crate::{TimerSpec, TriggerMode};

use super::*;

#[test]
fn test_duplicate_ports() {
    let mut env_builder = EnvBuilder::new();
    let reactor_key = env_builder
        .add_reactor("test_reactor", None, None, ())
        .finish()
        .unwrap();
    let _ = env_builder
        .add_input_port::<()>("port0", reactor_key)
        .unwrap();

    assert!(matches!(
        env_builder
            .add_output_port::<()>("port0", reactor_key)
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
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, ());

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
                TimerSpec {
                    period: Some(Duration::ZERO),
                    offset: Some(Duration::ZERO),
                }
            )
            .expect_err("Expected duplicate"),
        BuilderError::DuplicateActionDefinition {
            reactor_name,
            action_name,
        } if reactor_name == "test_reactor" && action_name == "action0"
    ));
}

/// Assert that building a reaction without any triggers is an error
#[test]
fn test_reactions_with_trigger() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, ());

    let res = reactor_builder
        .add_reaction("test", Box::new(|_ctx, _r, _i, _o, _a| {}))
        .finish();

    assert!(matches!(res, Err(BuilderError::ReactionBuilderError(_))));
}

#[test]
fn test_reactions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, ());

    let startup = reactor_builder.get_startup_action();

    let r0_key = reactor_builder
        .add_reaction("test", Box::new(|_ctx, _r, _i, _o, _a| {}))
        .with_action(startup, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .finish()
        .unwrap();

    let r1_key = reactor_builder
        .add_reaction("test", Box::new(|_ctx, _r, _i, _o, _a| {}))
        .with_action(startup, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .finish()
        .unwrap();

    let _reactor_key = reactor_builder.finish().unwrap();

    assert_eq!(env_builder.reactor_builders.len(), 1);
    assert_eq!(env_builder.reaction_builders.len(), 2);
    assert_eq!(
        env_builder.reaction_builders.keys().collect::<Vec<_>>(),
        vec![r0_key, r1_key]
    );

    let (env, _, _) = env_builder.into_runtime_parts().unwrap();
    assert_eq!(env.reactions.len(), 2);
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
        .with_effect_action(action_b, 0)
        .with_trigger_action(action_b, 1)
        .finish()
        .unwrap();

    // Triggered by a, schedules a
    let reaction_b = reactor_builder
        .add_reaction("rb", Box::new(|_, _, _, _, _, _| {}))
        .with_trigger_action(action_a, 0)
        .with_effect_action(action_a, 0)
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
