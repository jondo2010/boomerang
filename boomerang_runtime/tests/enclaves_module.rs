use boomerang_runtime::enclaves::{Enclave, EnclaveKey};

#[test]
fn enclave_types_are_available_from_their_own_module() {
    let enclaves = [Enclave::default()]
        .into_iter()
        .collect::<tinymap::TinyMap<EnclaveKey, Enclave>>();

    assert_eq!(enclaves.len(), 1);
}

#[test]
fn enclave_owner_allocates_dense_keys() {
    let mut enclaves = tinymap::TinyMap::<EnclaveKey, Enclave>::new();
    let first = enclaves.insert(Enclave::default());
    let second = enclaves.insert(Enclave::default());

    assert_eq!(tinymap::Key::index(&first), 0);
    assert_eq!(tinymap::Key::index(&second), 1);
}
