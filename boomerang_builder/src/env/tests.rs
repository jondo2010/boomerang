use runtime::BaseAction;

use crate::{reaction_closure, TimerSpec, TriggerMode};

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
                    period: Some(runtime::Duration::ZERO),
                    offset: Some(runtime::Duration::ZERO),
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
        .add_reaction("test", Box::new(reaction_closure!()))
        .finish();

    assert!(matches!(res, Err(BuilderError::ReactionBuilderError(_))));
}

#[test]
fn test_reactions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, ());

    let startup = reactor_builder.get_startup_action();

    let r0_key = reactor_builder
        .add_reaction("test", reaction_closure!())
        .with_action(startup, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .finish()
        .unwrap();

    let r1_key = reactor_builder
        .add_reaction("test", reaction_closure!())
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

#[test]
fn test_actions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, ());

    let action_a = reactor_builder
        .add_logical_action::<()>("a", Some(runtime::Duration::seconds(1)))
        .unwrap();
    let action_b = reactor_builder.add_logical_action::<()>("b", None).unwrap();

    // Triggered by a+b, schedules b
    let reaction_a = reactor_builder
        .add_reaction("ra", reaction_closure!())
        .with_action(action_a, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .with_action(action_b, 1, TriggerMode::TriggersAndEffects)
        .unwrap()
        .finish()
        .unwrap();

    // Triggered by a, schedules a
    let reaction_b = reactor_builder
        .add_reaction("rb", reaction_closure!())
        .with_action(action_a, 0, TriggerMode::TriggersAndEffects)
        .unwrap()
        .finish()
        .unwrap();

    let _reactor_key = reactor_builder.finish().unwrap();
    let (env, dep_info, aliases) = env_builder.into_runtime_parts().unwrap();

    //runtime::check_consistency(&env, &dep_info);

    let reaction_a = aliases.reaction_aliases[reaction_a];
    let reaction_b = aliases.reaction_aliases[reaction_b];
    let action_a = aliases.action_aliases[action_a.into()];
    let action_b = aliases.action_aliases[action_b.into()];

    assert_eq!(
        env.actions[action_a]
            .downcast_ref::<runtime::Action>()
            .expect("Action")
            .name(),
        "a"
    );

    // action_a is TriggersOnly on reaction_a, so should not be in the `reaction_actions`
    itertools::assert_equal(dep_info.reaction_actions[reaction_a].iter(), [action_b]);

    itertools::assert_equal(
        dep_info.action_triggers[action_a].iter().map(|&(_, r)| r),
        [reaction_a, reaction_b],
    );

    itertools::assert_equal(dep_info.reaction_actions[reaction_b].iter(), [action_a]);
}
