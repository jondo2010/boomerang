#[test]
fn test_build_partition_map() {
    let crate::tests::PingPong {
        env_builder,
        main,
        ping,
        pong,
        ping_input: _,
        ping_output: _,
        pong_input: _,
        pong_output: _,
    } = crate::tests::create_ping_pong();

    let partition_map = env_builder.build_partition_map();
    assert_eq!(partition_map.len(), 3);

    assert_eq!(partition_map[main], main);
    assert_eq!(partition_map[ping], ping);
    assert_eq!(partition_map[pong], pong);
}
