use cargo_boomerang::run_descriptor_driver;

mod support;

#[test]
fn repeated_descriptor_analysis_uses_cargo_freshness() {
    let target = tempfile::tempdir().unwrap();
    let (first, second) = support::with_target_directory(target.path(), || {
        let run = || run_descriptor_driver(support::fixture_workspace(), "production").unwrap();
        (run(), run())
    });

    assert!(first.compiled_artifacts() > 0);
    assert_eq!(second.compiled_artifacts(), 0);
    assert_eq!(
        serde_json::to_vec(first.topology()).unwrap(),
        serde_json::to_vec(second.topology()).unwrap(),
    );
}

#[test]
fn driver_selects_only_bound_descriptors_and_emits_topology() {
    let target = support::shared_target("analysis");
    let (output, result) = support::with_target_directory(&target, || {
        (
            run_descriptor_driver(support::fixture_workspace(), "production").unwrap(),
            run_descriptor_driver(support::fixture_workspace(), "payload-alias"),
        )
    });

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

    let error = result.err().unwrap();
    assert!(error.to_string().contains("reserved payload facet"));
}
