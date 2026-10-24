#[test]
fn dependency_features_are_additive() {
    metadata_user::verify();
    payload_user::verify();
}
