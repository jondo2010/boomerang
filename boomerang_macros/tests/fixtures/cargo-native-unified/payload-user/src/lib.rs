pub fn verify() {
    assert_eq!(
        cargo_native_component::controller::__boomerang::payload::state_Controller().count,
        21
    );
}
