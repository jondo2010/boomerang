//! Complete canonical graphs captured from checkpoint 8d6a0e5, before replacing
//! Assembly projection with generated component topology constructors.
use boomerang::builder::compiler::ApplicationTopology;

#[test]
fn generated_compositions_preserve_the_compiled_topology_contract() {
    for (actual, expected) in [
        (
            snake::topology().unwrap(),
            include_str!("fixtures/snake-topology.json"),
        ),
        (
            snake::keyboard_topology().unwrap(),
            include_str!("fixtures/keyboard-topology.json"),
        ),
    ] {
        let expected: ApplicationTopology = serde_json::from_str(expected).unwrap();
        assert_eq!(actual, expected);
    }
}
