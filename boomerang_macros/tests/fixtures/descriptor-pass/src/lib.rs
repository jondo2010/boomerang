use boomerang::prelude::*;

#[cfg(not(feature = "__boomerang_descriptor"))]
fn initial_count() -> usize {
    3
}

#[cfg(not(feature = "__boomerang_descriptor"))]
fn target_only_reaction_payload() {}

#[reactor(
    contract = "example.sensor",
    contract_version = 1,
    bounds(
        queue_capacity = 16,
        payload_bytes = 1024,
        state_bytes = 512,
        scratch_bytes = 256,
    )
)]
pub fn r#match(
    #[input] r#async: u32,
    #[output] r#type: u32,
    #[output] r#yield: u32,
    #[state(default = initial_count())] r#loop: usize,
) -> impl Reactor {
    reaction! {
        r#move (r#async, startup) -> r#type, r#yield {
            target_only_reaction_payload();
            state.r#loop += r#async.unwrap_or_default() as usize;
            r#type.set(ctx);
            *r#yield = Some(state.r#loop as u32);
        }
    }
    mode! { r#type {
        reaction! {
            (reset) -> history(r#type) {
                target_only_reaction_payload();
                r#type.set(ctx);
            }
        }
    } }
}

pub mod custom {
    //! Custom-state payload export fixture.

    use super::*;

    /// State constructed through an explicit `state_init` path.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CustomState {
        /// Initial count observed by the payload test.
        pub count: usize,
    }

    #[cfg(feature = "__boomerang_payload")]
    fn init_custom_state() -> CustomState {
        CustomState { count: 11 }
    }

    #[reactor(
        contract = "example.custom",
        contract_version = 1,
        bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1),
        state = CustomState,
        state_init = init_custom_state
    )]
    pub fn Custom() -> impl Reactor {
        reaction! {
            start (startup) {
                state.count += 1;
            }
        }
    }
}

pub mod shaped {
    //! Fixed-array and dynamic-bank payload reference fixture.

    use super::*;

    /// Declares every currently supported own-port reference shape.
    #[reactor(
        contract = "example.shaped",
        contract_version = 1,
        bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
    )]
    pub fn Shaped(
        #[input] array_in: [u16; 2],
        #[input(len = 2)] bank_in: u64,
        #[output] array_out: [u32; 3],
        #[output(len = 3)] bank_out: u8,
    ) -> impl Reactor {
        reaction! {
            shaped (array_in, bank_in) -> array_out, bank_out {}
        }
    }
}

pub mod lifetime_collision {
    //! User-lifetime collision and private generated-state fixture.

    use super::*;

    /// Declares the lifetime name formerly hardcoded by payload generation.
    ///
    /// The generated `LifetimeState::marker` field retains the user's `'store` lifetime.
    #[reactor(
        contract = "example.lifetime",
        contract_version = 1,
        bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
    )]
    fn Lifetime<'store>(#[state(default = "")] marker: &'store str) -> impl Reactor {
        reaction! {
            tick (startup) {}
        }
    }
}

pub mod private_empty {
    //! Private zero-state reactor fixture.

    use super::*;

    /// Provides a generated empty state alias to the payload launcher.
    #[reactor(
        contract = "example.private-empty",
        contract_version = 1,
        bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
    )]
    fn Empty() -> impl Reactor {
        reaction! {
            start (startup) {}
        }
    }
}

pub mod actions {
    use super::*;

    #[reactor(
        contract = "example.actions",
        contract_version = 1,
        bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
    )]
    pub fn Actions(
        #[logical_action(min_delay = 0)] logical_now: u32,
        #[logical_action(min_delay = 10 msec)] logical_later: u32,
        #[physical_action(min_delay = 0)] physical_now: u16,
        #[physical_action(min_delay = 7 nsec)] physical_later: u16,
    ) -> impl Reactor {
        reaction! {
            act (logical_now, physical_now) -> logical_later, physical_later {}
        }
    }
}

#[cfg(feature = "invalid-action-attribute")]
#[reactor]
fn InvalidActionAttribute(#[logical_action(delay = 1 sec)] tick: ()) -> impl Reactor {}
#[cfg(feature = "invalid-action-duration")]
#[reactor]
fn InvalidActionDuration(#[physical_action(min_delay = 1 fortnight)] tick: ()) -> impl Reactor {}
#[cfg(feature = "action-delay-overflow")]
#[reactor]
fn Delay(#[logical_action(min_delay = 9223372036854775808 nsec)] tick: ()) -> impl Reactor {}
#[cfg(feature = "action-duration-unit-overflow")]
#[reactor]
fn Unit(#[logical_action(min_delay = 18446744073709551615 weeks)] tick: ()) -> impl Reactor {}

#[cfg(feature = "missing-state-init")]
mod missing_state_init {
    use super::*;

    /// Custom state intentionally missing a payload initializer.
    #[derive(Clone)]
    struct CustomState;

    #[reactor(
        contract = "example.missing-state-init",
        contract_version = 1,
        bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1),
        state = CustomState
    )]
    fn MissingStateInit() -> impl Reactor {}
}

#[cfg(feature = "payload-lexical-relation")]
mod lexical_relation {
    use super::*;

    #[reactor(
        contract = "example.lexical",
        contract_version = 1,
        bounds(queue_capacity = 1, payload_bytes = 1, state_bytes = 1, scratch_bytes = 1)
    )]
    fn LexicalRelation() -> impl Reactor {
        reaction! {
            (child.output) {
                let _ = child_output;
            }
        }
    }
}

#[cfg(feature = "__boomerang_descriptor")]
pub fn descriptor() -> boomerang::builder::ComponentDescriptor {
    __boomerang::descriptor()
}

#[cfg(all(test, feature = "__boomerang_descriptor"))]
mod descriptor_tests {
    #[test]
    fn descriptor_contains_only_source_observable_structure() {
        let descriptor = super::descriptor();
        assert_eq!(descriptor.contract_id().as_str(), "example.sensor");
        assert_eq!(descriptor.contract_version(), 1);
        assert_eq!(descriptor.reactor_slots().len(), 1);
        assert_eq!(descriptor.reactor_slots()[0].id.to_string(), "Match");
        assert_eq!(descriptor.port_slots().len(), 3);
        assert_eq!(descriptor.port_slots()[0].id.to_string(), "Match/async");
        assert_eq!(descriptor.port_slots()[1].id.to_string(), "Match/type");
        assert_eq!(descriptor.port_slots()[2].id.to_string(), "Match/yield");
        assert_eq!(descriptor.reaction_slots().len(), 2);
        assert_eq!(descriptor.reaction_slots()[0].id.to_string(), "Match/move");
        assert_eq!(descriptor.reaction_slots()[1].id.to_string(), "Match/#g0");
        assert_eq!(descriptor.mode_slots()[0].id.to_string(), "Match/type");
        assert_eq!(descriptor.relationships().len(), 7);
        assert_eq!(descriptor.state_slots().len(), 1);
        assert_eq!(descriptor.state_slots()[0].id.to_string(), "Match/loop");
        assert!(descriptor.codec_slots().is_empty());
        assert!(descriptor.placement_groups().is_empty());
        assert!(descriptor.enclaves().is_empty());
        assert_eq!(
            descriptor.bounds(),
            boomerang::builder::DescriptorBounds {
                queue_capacity: boomerang::builder::DescriptorBound::Known(16),
                payload_bytes: boomerang::builder::DescriptorBound::Known(1024),
                state_bytes: boomerang::builder::DescriptorBound::Known(512),
                scratch_bytes: boomerang::builder::DescriptorBound::Known(256),
            }
        );
        assert_eq!(
            descriptor.descriptor_fingerprint_input().state_slots(),
            descriptor.state_slots()
        );
        assert_eq!(
            descriptor
                .descriptor_fingerprint_input()
                .fingerprint()
                .to_bytes(),
            [
                173, 248, 107, 207, 105, 80, 159, 129, 225, 21, 134, 108, 49, 224, 42, 183,
                112, 195, 43, 150, 102, 68, 163, 191, 240, 50, 132, 133, 213, 59, 136, 241,
            ]
        );
    }

    #[test]
    fn descriptor_contains_standard_action_slots_and_relationships() {
        let descriptor = super::actions::__boomerang::descriptor();
        assert_eq!(
            descriptor
                .action_slots()
                .iter()
                .map(|slot| slot.id.to_string())
                .collect::<Vec<_>>(),
            [
                "Actions/logical_later",
                "Actions/logical_now",
                "Actions/physical_later",
                "Actions/physical_now",
            ]
        );
        assert_eq!(descriptor.relationships().len(), 4);
        assert!(descriptor.relationships().iter().all(|relationship| matches!(
            relationship.target,
            boomerang::builder::DescriptorRelationshipTarget::Action(_)
        )));
    }
}

#[cfg(all(test, feature = "__boomerang_payload"))]
mod payload_compile_input_tests {
    #[test]
    fn payload_mode_embeds_host_provided_compatibility_values() {
        assert_eq!(
            super::__boomerang::BINDING_MANIFEST
                .descriptor_fingerprint()
                .to_bytes(),
            [
                173, 248, 107, 207, 105, 80, 159, 129, 225, 21, 134, 108, 49, 224, 42, 183,
                112, 195, 43, 150, 102, 68, 163, 191, 240, 50, 132, 133, 213, 59, 136, 241,
            ],
        );
        assert_eq!(
            super::__boomerang::BINDING_MANIFEST.macro_abi(),
            boomerang::runtime::binding::COMPONENT_DESCRIPTOR_MACRO_ABI,
        );

        let state: super::MatchState = super::__boomerang::state_Match();
        assert_eq!(state.r#loop, 3);
        let custom_state: super::custom::CustomState = super::custom::__boomerang::state_Custom();
        assert_eq!(custom_state.count, 11);
    }
}
