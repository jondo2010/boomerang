//! Nested execution replaces the removed reaction-owner lookup unit test.
//! Distinct state types detect incorrect owner selection through actual payload adapters.

use boomerang_builder::{compiler::*, *};

/// One component with different state types at the root and child reactors.
pub struct Nested;
impl ComponentDefinition for Nested {
    type Ports = ();
    fn contract(&self) -> (&str, u64) {
        ("example.nested", 1)
    }
    fn declare(&self, context: &mut ComponentTopology<'_>) -> Result<(), TopologyAuthoringError> {
        let root = context.root().clone();
        let child = ReactorId::new(format!("{root}/child"))?;
        context
            .builder()
            .add_reactor(boomerang_builder::compiler::Reactor::new(
                child.clone(),
                ComponentInstanceId::new(root.to_string())?,
                Some(root.clone()),
                None,
                StableEnclaveId::new("first")?,
                Some(PlacementGroupId::new("placement/first")?),
                None,
            ))?;
        for owner in [root, child] {
            for (position, name, kind) in [
                (0, "__startup", ActionKind::Startup),
                (1, "__shutdown", ActionKind::Shutdown),
            ] {
                // The authoring builder already supplies lifecycle actions for the root.
                if owner.to_string().ends_with("/child") {
                    context.builder().add_action(
                        ActionId::new(format!("{owner}/{name}"))?,
                        owner.clone(),
                        kind,
                        position,
                        None,
                    )?;
                }
            }
            context.builder().add_reaction(
                ReactionId::new(format!("{owner}/start"))?,
                owner.clone(),
                [ReactionRelation::new(
                    ReactionRelationTarget::Action(ActionId::new(format!("{owner}/__startup"))?),
                    ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
                    0,
                )],
                ReactionOptions::default(),
            )?;
        }
        Ok(())
    }
}

/// Composite descriptor maps root and child exports to their exact state owners.
pub fn descriptor() -> ComponentDescriptor {
    let root = ReactorSlotId::new("Root").unwrap();
    let child = ReactorSlotId::new("Root/child").unwrap();
    let owners = [root.clone(), child.clone()];
    ComponentDescriptor::try_new(
        ContractId::new("example.nested").unwrap(),
        1,
        // Keep the composite structure intact; only its metadata export-contract version differs.
        if cfg!(feature = "abi-mismatch") {
            COMPONENT_DESCRIPTOR_MACRO_ABI - 1
        } else {
            COMPONENT_DESCRIPTOR_MACRO_ABI
        },
        vec![
            ReactorSlot {
                id: root.clone(),
                parent: None,
                capabilities: None,
            },
            ReactorSlot {
                id: child,
                parent: Some(root),
                capabilities: None,
            },
        ],
        vec![],
        vec![],
        owners
            .iter()
            .map(|owner| ReactionSlot {
                id: ReactionSlotId::new(format!("{owner}/start")).unwrap(),
                reactor: owner.clone(),
            })
            .collect(),
        vec![],
        owners
            .iter()
            .map(|owner| StateSlot {
                id: StateSlotId::new(format!("{owner}/marker")).unwrap(),
                reactor: owner.clone(),
            })
            .collect(),
        vec![],
        owners
            .iter()
            .map(|owner| DescriptorRelationship {
                reaction: ReactionSlotId::new(format!("{owner}/start")).unwrap(),
                kind: DescriptorRelationshipKind::Trigger,
                target: DescriptorRelationshipTarget::Lifecycle(DescriptorLifecycle::Startup),
                mode_transition: None,
                declaration_position: 0,
            })
            .collect(),
        vec![],
        vec![],
        DescriptorBounds {
            queue_capacity: DescriptorBound::Known(4),
            payload_bytes: DescriptorBound::Known(64),
            state_bytes: DescriptorBound::Known(64),
            scratch_bytes: DescriptorBound::Known(64),
        },
    )
    .unwrap()
}
