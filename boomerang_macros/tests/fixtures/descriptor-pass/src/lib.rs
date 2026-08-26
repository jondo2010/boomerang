use boomerang::prelude::*;

#[reactor(contract = "example.sensor", contract_version = 1)]
pub fn Sensor(
    #[input] sample: u32,
    #[output] filtered: u32,
    #[state(default = state_constructor_must_not_compile())] count: usize,
) -> impl Reactor {
    reaction! {
        filter (sample) -> filtered {
            reaction_payload_must_not_compile();
        }
    }
}

#[cfg(feature = "__boomerang_descriptor")]
pub fn descriptor() -> boomerang::builder::ComponentDescriptor {
    __boomerang::descriptor()
}

#[cfg(all(test, feature = "__boomerang_descriptor"))]
mod tests {
    #[test]
    fn descriptor_contains_only_source_observable_structure() {
        let descriptor = super::descriptor();
        assert_eq!(descriptor.contract_id().as_str(), "example.sensor");
        assert_eq!(descriptor.contract_version(), 1);
        assert_eq!(descriptor.reactor_slots().len(), 1);
        assert_eq!(descriptor.port_slots().len(), 2);
        assert_eq!(descriptor.reaction_slots().len(), 1);
        assert_eq!(descriptor.relationships().len(), 2);
        assert_eq!(descriptor.state_slots().len(), 1);
        assert_eq!(descriptor.state_slots()[0].id.to_string(), "Sensor/count");
        assert!(descriptor.codec_slots().is_empty());
        assert!(descriptor.placement_groups().is_empty());
        assert!(descriptor.enclaves().is_empty());
        assert_eq!(
            descriptor.descriptor_fingerprint_input().state_slots(),
            descriptor.state_slots()
        );
    }
}
