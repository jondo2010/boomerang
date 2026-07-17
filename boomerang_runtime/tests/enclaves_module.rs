use boomerang_runtime::enclaves::{Enclave, EnclaveKey};

#[test]
fn enclave_types_are_available_from_their_own_module() {
    let enclaves = [Enclave::default()]
        .into_iter()
        .collect::<tinymap::TinyMap<EnclaveKey, Enclave>>();

    assert_eq!(enclaves.len(), 1);
}
