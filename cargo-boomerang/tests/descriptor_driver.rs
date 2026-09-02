use std::path::PathBuf;

use cargo_boomerang::run_descriptor_driver;

fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}
#[test]
fn driver_selects_only_bound_descriptors_and_emits_topology() {
    let output = run_descriptor_driver(fixture_workspace(), "production").unwrap();

    assert_eq!(
        output
            .topology()
            .components()
            .map(|(id, _)| id.to_string())
            .collect::<Vec<_>>(),
        ["backup", "controller", "sensor"]
    );
    assert_eq!(
        output
            .topology()
            .components()
            .map(|(_, component)| (component.contract().as_str(), component.contract_version()))
            .collect::<Vec<_>>(),
        [
            ("vehicle.controller", 1),
            ("vehicle.controller", 1),
            ("vehicle.sensor", 1)
        ]
    );
    assert_eq!(
        output.selected_packages().collect::<Vec<_>>(),
        ["sensor-host", "vehicle-control"]
    );
    assert!(!output.build_log().contains("payload-only"));

    let result = run_descriptor_driver(fixture_workspace(), "payload-alias");
    let error = result.err().unwrap();
    assert!(error.to_string().contains("reserved payload facet"));
}
