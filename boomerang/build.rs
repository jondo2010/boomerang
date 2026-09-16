fn main() {
    println!("cargo:rustc-check-cfg=cfg(boomerang_facet, values(\"descriptor\", \"payload\"))");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_BOOMERANG_FACET");
    if let Ok(facet) = std::env::var("CARGO_CFG_BOOMERANG_FACET") {
        assert!(
            matches!(facet.as_str(), "descriptor" | "payload"),
            "invalid boomerang_facet: expected exactly one of descriptor or payload"
        );
        let opposite = if facet == "descriptor" {
            "CARGO_FEATURE___BOOMERANG_PAYLOAD"
        } else {
            "CARGO_FEATURE___BOOMERANG_DESCRIPTOR"
        };
        assert!(
            std::env::var_os(opposite).is_none(),
            "conflicting boomerang facet selectors"
        );
    }
}
