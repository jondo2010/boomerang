//! Target-neutral application compiler models.
#![deny(missing_docs)]

mod identity;
mod model;

pub use identity::{
    ApplicationId, BindingSlotId, BoundaryId, ComponentInstanceId, ContractId, InvalidStableId,
    PlacementGroupId, StableEnclaveId,
};
pub use model::{
    ApplicationTopology, ApplicationTopologyBuilder, ComponentInstance, TopologyBuildError,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_instance_uses_stable_component_and_contract_ids() {
        let component = ComponentInstance::new("vehicle/sensor", "sensor.v1").unwrap();

        assert_eq!(component.id().as_str(), "vehicle/sensor");
        assert_eq!(component.contract().as_str(), "sensor.v1");
    }

    #[test]
    fn stable_id_types_share_canonical_path_validation() {
        assert_eq!(ApplicationId::new("vehicle").unwrap().as_str(), "vehicle");
        assert_eq!(
            ComponentInstanceId::new("vehicle/sensor").unwrap().as_str(),
            "vehicle/sensor"
        );
        assert_eq!(ContractId::new("sensor.v1").unwrap().as_str(), "sensor.v1");
        assert_eq!(
            PlacementGroupId::new("vehicle/io").unwrap().as_str(),
            "vehicle/io"
        );
        assert_eq!(
            StableEnclaveId::new("vehicle/io/read").unwrap().as_str(),
            "vehicle/io/read"
        );
        assert_eq!(
            BoundaryId::new("vehicle/io/sample").unwrap().as_str(),
            "vehicle/io/sample"
        );
        assert_eq!(
            BindingSlotId::<()>::new("sensor/read").unwrap().as_str(),
            "sensor/read"
        );

        for invalid in [
            "",
            "/sensor",
            "sensor/",
            "sensor//read",
            ".",
            "..",
            "a/./b",
            "a/../b",
        ] {
            assert!(
                ComponentInstanceId::new(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn topology_builder_rejects_duplicate_component_ids() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        builder
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v1").unwrap())
            .unwrap();

        let error = builder
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v2").unwrap())
            .unwrap_err();

        assert!(matches!(
            error,
            TopologyBuildError::DuplicateComponentId { component_id }
                if component_id.as_str() == "vehicle/sensor"
        ));
    }

    #[test]
    fn topology_debug_uses_stable_identity_not_dense_keys() {
        let mut builder = ApplicationTopologyBuilder::new("vehicle").unwrap();
        builder
            .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v1").unwrap())
            .unwrap();

        let debug = format!("{:?}", builder.finish());

        assert!(debug.contains("vehicle/sensor"));
        assert!(!debug.contains("ComponentKey"));
    }
}
