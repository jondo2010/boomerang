use boomerang_runtime as runtime;

#[test]
fn test_build_partition_map() {
    let crate::tests::PingPong {
        assembly,
        main,
        ping,
        pong,
        ping_input: _,
        ping_output: _,
        pong_input: _,
        pong_output: _,
    } = crate::tests::create_ping_pong();

    let partition_map = assembly.build_partition_map();
    assert_eq!(partition_map.len(), 3);
    assert_eq!(partition_map[main], main);
    assert_eq!(partition_map[ping], ping);
    assert_eq!(partition_map[pong], pong);

    fn assert_enclaves(
        enclaves: tinymap::TinyMapRef<'_, runtime::EnclaveKey, runtime::Enclave>,
    ) -> usize {
        enclaves.len()
    }

    let crate::tests::PingPong { assembly, .. } = crate::tests::create_ping_pong();
    let runtime = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();
    assert_eq!(assert_enclaves(runtime.enclaves()), 3);
}
