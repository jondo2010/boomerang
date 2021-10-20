use super::*;
use crate::builder::tests::*;

#[test]
fn test_duplicate_ports() {
    let mut env_builder = EnvBuilder::<SchedulerDummy>::new();
    let (reactor_key, _, _) = env_builder
        .add_reactor("test_reactor", None, TestReactorDummy)
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
    let mut env_builder = EnvBuilder::<SchedulerDummy>::new();
    let (reactor_key, _, _) = env_builder
        .add_reactor("test_reactor", None, TestReactorDummy)
        .finish()
        .unwrap();

    env_builder
        .add_logical_action::<()>("action0", None, reactor_key)
        .unwrap();

    assert!(matches!(
        env_builder
            .add_logical_action::<()>("action0", None, reactor_key)
            .expect_err("Expected duplicate"),
        BuilderError::DuplicateActionDefinition {
            reactor_name,
            action_name,
        } if reactor_name== "test_reactor" && action_name == "action0"
    ));

    assert!(matches!(
        env_builder
            .add_timer(
                "action0",
                runtime::Duration::from_micros(0),
                runtime::Duration::from_micros(0),
                reactor_key
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
    let mut env_builder = EnvBuilder::<SchedulerDummy>::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, TestReactorDummy);

    let r0_key = reactor_builder
        .add_reaction("test", TestReactorDummy::reaction_dummy)
        .finish()
        .unwrap();

    let r1_key = reactor_builder
        .add_reaction("test", TestReactorDummy::reaction_dummy)
        .finish()
        .unwrap();

    let (_reactor_key, _, _) = reactor_builder.finish().unwrap();

    assert_eq!(env_builder.reactors.len(), 1);
    assert_eq!(env_builder.reaction_builders.len(), 2);
    assert_eq!(
        env_builder.reaction_builders.keys().collect::<Vec<_>>(),
        vec![r0_key, r1_key]
    );

    // assert_eq!(env_builder.reactors[reactor_key].reactions.len(), 2);

    let dep_edges = env_builder.reaction_dependency_edges().collect::<Vec<_>>();
    assert_eq!(dep_edges, vec![(r0_key, r1_key)]);

    let env: runtime::Env<_> = env_builder.try_into().unwrap();
    assert_eq!(env.reactions.len(), 2);
}

#[test]
fn test_actions1() {
    let mut env_builder = EnvBuilder::<SchedulerDummy>::new();
    let reactor_builder = env_builder.add_reactor("test_reactor", None, TestReactorDummy);
    // let r0_key = reactor_builder.add_reaction(|_, _, _, _, _| {}).finish().unwrap();
    let (reactor_key, _, _) = reactor_builder.finish().unwrap();
    let action_key = env_builder
        .add_logical_action::<()>("a", Some(runtime::Duration::from_secs(1)), reactor_key)
        .unwrap()
        .into();
    let env: runtime::Env<_> = env_builder.try_into().unwrap();

    assert_eq!(env.actions[action_key].get_name(), "a");
    assert_eq!(env.actions[action_key].get_is_logical(), true);
    assert_eq!(
        env.actions[action_key].get_min_delay(),
        runtime::Duration::from_secs(1)
    );
}
