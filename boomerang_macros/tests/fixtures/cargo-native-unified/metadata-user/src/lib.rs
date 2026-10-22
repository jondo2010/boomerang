pub fn verify() {
    assert_eq!(
        cargo_native_component::controller::__boomerang::metadata::descriptor()
            .contract_id()
            .as_str(),
        "example.controller"
    );
    let _ = cargo_native_component::controller::definition();
}
