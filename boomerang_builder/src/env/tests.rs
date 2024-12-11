#[test]
fn test_build_partition_map() {
    let env_builder = crate::tests::create_ping_pong();

    let main = env_builder.find_reactor_by_fqn("main").unwrap();
    let ping = env_builder.find_reactor_by_fqn("main::Ping").unwrap();
    let pong = env_builder.find_reactor_by_fqn("main::Pong").unwrap();

    let partition_map = env_builder.build_partition_map();
    assert_eq!(partition_map.len(), 3);

    assert_eq!(partition_map[main], main);
    assert_eq!(partition_map[ping], ping);
    assert_eq!(partition_map[pong], pong);
}
