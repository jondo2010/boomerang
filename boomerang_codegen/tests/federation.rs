//! Executes independently built ordinary Cargo federation applications.
mod cargo_support;
mod federation_support;

#[test]
fn independent_cargo_artifacts_exchange_and_reject_mixed_deployments() {
    federation_support::exercise();
}
