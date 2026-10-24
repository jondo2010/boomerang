boomerang::component! {
    pub mod sensor {
        use boomerang::prelude::*;

        fn initial_count() -> usize {
            3
        }

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
    }
}

boomerang::component! {
    pub mod custom {
        //! Custom-state payload export fixture.

        use boomerang::prelude::*;

        /// State constructed through an explicit `state_init` path.
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct CustomState {
            /// Initial count observed by the payload test.
            pub count: usize,
        }

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
}

boomerang::component! {
    pub mod shaped {
        //! Fixed-array and dynamic-bank payload reference fixture.

        use boomerang::prelude::*;

        /// Declares every currently supported own-port reference shape.
        #[reactor(
            contract = "example.shaped",
            contract_version = 1,
            bounds(
                queue_capacity = 1,
                payload_bytes = 1,
                state_bytes = 1,
                scratch_bytes = 1
            )
        )]
        pub fn Shaped(
            #[input] array_in: [u16; 2],
            #[input(len = 2)] bank_in: u64,
            #[output] array_out: [u32; 3],
            #[output(len = 3)] bank_out: u8,
            #[input] single_in: u32,
            #[output] single_out: u64,
            #[state] calls: u32,
        ) -> impl Reactor {
            reaction! {
                shaped (array_in, startup, bank_in) single_in -> array_out, bank_out, single_out {
                    assert_eq!(*array_in[0], Some(11));
                    assert_eq!(*array_in[1], Some(12));
                    assert_eq!(bank_in.len(), 2);
                    assert_eq!(*bank_in.get(0).unwrap(), Some(21));
                    assert_eq!(*bank_in.get(1).unwrap(), Some(22));
                    assert_eq!(*single_in, Some(31));
                    for (index, port) in array_out.iter_mut().enumerate() { **port = Some(41 + index as u32); }
                    assert_eq!(bank_out.len(), 3);
                    for (index, mut port) in bank_out.iter_mut().enumerate() { *port = Some(51 + index as u8); }
                    *single_out = Some(61);
                    state.calls += 1;
                }
            }
        }
    }
}

boomerang::component! {
    pub mod lifetime_collision {
        //! User-lifetime collision and private generated-state fixture.

        use boomerang::prelude::*;

        /// Declares the lifetime name formerly hardcoded by payload generation.
        ///
        /// The generated state retains the user's lifetime, which matches the adapter's first candidate.
        #[reactor(
            contract = "example.lifetime",
            contract_version = 1,
            bounds(
                queue_capacity = 1,
                payload_bytes = 1,
                state_bytes = 1,
                scratch_bytes = 1
            )
        )]
        fn Lifetime<'__boomerang_store>(#[state(default = "")] marker: &'__boomerang_store str) -> impl Reactor {
            reaction! {
                tick (startup) { state.marker = "invoked"; }
            }
        }
    }
}

boomerang::component! {
    pub mod private_empty {
        //! Private zero-state reactor fixture.

        use boomerang::prelude::*;

        /// Provides a generated empty state alias to the payload launcher.
        #[reactor(
            contract = "example.private-empty",
            contract_version = 1,
            bounds(
                queue_capacity = 1,
                payload_bytes = 1,
                state_bytes = 1,
                scratch_bytes = 1
            )
        )]
        fn Empty() -> impl Reactor {
            reaction! {
                start (startup) {}
            }
        }
    }
}

boomerang::component! {
    pub mod actions {
        use boomerang::prelude::*;

        #[reactor(
            contract = "example.actions",
            contract_version = 1,
            bounds(
                queue_capacity = 1,
                payload_bytes = 1,
                state_bytes = 1,
                scratch_bytes = 1
            )
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
}

#[cfg(feature = "invalid-action-attribute")]
#[boomerang::prelude::reactor]
fn InvalidActionAttribute(#[logical_action(delay = 1 sec)] tick: ()) -> impl Reactor {}
#[cfg(feature = "invalid-action-duration")]
#[boomerang::prelude::reactor]
fn InvalidActionDuration(#[physical_action(min_delay = 1 fortnight)] tick: ()) -> impl Reactor {}
#[cfg(feature = "action-delay-overflow")]
#[boomerang::prelude::reactor]
fn Delay(#[logical_action(min_delay = 9223372036854775808 nsec)] tick: ()) -> impl Reactor {}
#[cfg(feature = "action-duration-unit-overflow")]
#[boomerang::prelude::reactor]
fn Unit(#[logical_action(min_delay = 18446744073709551615 weeks)] tick: ()) -> impl Reactor {}

#[cfg(feature = "missing-state-init")]
boomerang::component! {
    mod missing_state_init {
        use boomerang::prelude::*;

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
}

#[cfg(feature = "payload-lexical-relation")]
boomerang::component! {
    mod lexical_relation {
        use boomerang::prelude::*;

        #[reactor(
            contract = "example.lexical",
            contract_version = 1,
            bounds(
                queue_capacity = 1,
                payload_bytes = 1,
                state_bytes = 1,
                scratch_bytes = 1
            )
        )]
        fn LexicalRelation() -> impl Reactor {
            reaction! {
                (child.output) {
                    let _ = child_output;
                }
            }
        }
    }
}

#[cfg(feature = "metadata")]
pub fn descriptor() -> boomerang::builder::ComponentDescriptor {
    sensor::__boomerang::metadata::descriptor()
}

#[cfg(all(test, feature = "metadata"))]
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
            descriptor.fingerprint().to_bytes(),
            [
                0xa0, 0x8b, 0x58, 0x1b, 0xab, 0x17, 0x0b, 0xd1, 0x40, 0xc8, 0xb9, 0x0d, 0x12, 0x45,
                0x71, 0xaf, 0x50, 0xa8, 0x04, 0x45, 0x37, 0xc8, 0xf3, 0x47, 0x28, 0xfb, 0xac, 0xed,
                0xb0, 0x78, 0x3e, 0x1d
            ]
        );
    }

    #[test]
    fn descriptor_contains_standard_action_slots_and_relationships() {
        let descriptor = super::actions::__boomerang::metadata::descriptor();
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
        assert!(descriptor
            .relationships()
            .iter()
            .all(|relationship| matches!(
                relationship.target,
                boomerang::builder::DescriptorRelationshipTarget::Action(_)
            )));
    }
}

#[cfg(all(test, feature = "payload"))]
mod payload_tests {
    #[test]
    fn payload_manifest_initializers_and_adapters_match_declaration() {
        assert_eq!(
            super::sensor::__boomerang::payload::binding_manifest()
                .descriptor_fingerprint()
                .to_bytes(),
            [
                0xa0, 0x8b, 0x58, 0x1b, 0xab, 0x17, 0x0b, 0xd1, 0x40, 0xc8, 0xb9, 0x0d, 0x12, 0x45,
                0x71, 0xaf, 0x50, 0xa8, 0x04, 0x45, 0x37, 0xc8, 0xf3, 0x47, 0x28, 0xfb, 0xac, 0xed,
                0xb0, 0x78, 0x3e, 0x1d
            ],
        );
        assert_eq!(
            super::sensor::__boomerang::payload::binding_manifest().macro_abi(),
            boomerang::runtime::binding::COMPONENT_DESCRIPTOR_MACRO_ABI,
        );

        let state: super::sensor::MatchState = super::sensor::__boomerang::payload::state_Match();
        assert_eq!(state.r#loop, 3);
        let custom_state: super::custom::CustomState =
            super::custom::__boomerang::payload::state_Custom();
        assert_eq!(custom_state.count, 11);
        super::adapter_execution::verify();
    }
}

/// Generic parameters and bounds must reach initialization contexts and payload adapters.
#[cfg(any(feature = "metadata", feature = "payload"))]
boomerang::component! {
    pub mod generic_init {
        use boomerang::prelude::*;

        #[derive(Debug)]
        pub struct GenericState;

        fn initialize<T, U, const N: usize>(
            context: GenericInit<T, U, N>,
        ) -> Result<GenericState, std::io::Error>
        where
            T: boomerang::runtime::ReactorData + Copy + Default,
            U: boomerang::runtime::ReactorData,
        {
            let _: boomerang::runtime::AsyncActionRef<u32> = context.sample;
            Ok(GenericState)
        }

        #[reactor(
            contract = "example.generic",
            contract_version = 1,
            bounds(queue_capacity = 2, payload_bytes = 16, state_bytes = 1, scratch_bytes = 1),
            state = GenericState,
            state_init = initialize::<T, U, N>,
            init_context(name = GenericInit, actions = [sample])
        )]
        pub fn Generic<T, U, const N: usize>(
            #[input] _input: u32,
            #[input] _unused_in_context: u32,
            #[physical_action] sample: u32,
        ) -> impl Reactor
        where
            T: boomerang::runtime::ReactorData + Copy + Default,
            U: boomerang::runtime::ReactorData,
        {
            reaction! { observe(sample) {} }
        }

        #[cfg(feature = "payload")]
        pub fn bind_generic(
            context: &boomerang::runtime::initialization::StateInitContext<'_, '_>,
            actions: &[boomerang::runtime::image::ActionIndex],
        ) -> Result<GenericState, boomerang::runtime::initialization::StateInitializationError> {
            __boomerang::payload::state_Generic::<u32, String, 2>(context, actions)
        }
    }
}

#[cfg(feature = "payload")]
mod typed_adapters {
    const _: boomerang::runtime::PayloadType<u32> =
        super::sensor::__boomerang::payload::port_Match_2fasync;
    const _: boomerang::runtime::PayloadType<u32> =
        super::actions::__boomerang::payload::action_Actions_2flogical_5fnow;
    const _: boomerang::runtime::PayloadType<u16> =
        super::actions::__boomerang::payload::action_Actions_2fphysical_5flater;

    /// Keeps a private reactor's generated state type nameable by the launcher.
    #[allow(dead_code)]
    fn generated_state_is_public(
        state: super::lifetime_collision::LifetimeState<'static>,
    ) -> &'static str {
        state.marker
    }

    /// Keeps a private reactor's generated empty state alias nameable by the launcher.
    #[allow(dead_code)]
    fn generated_empty_state_is_public(
        state: super::private_empty::EmptyState,
    ) -> super::private_empty::EmptyState {
        state
    }

    /// Keeps the custom state type nameable from the launcher crate.
    #[allow(dead_code)]
    fn custom_state_is_public(state: super::custom::CustomState) -> usize {
        state.count
    }
}

#[cfg(all(test, feature = "payload"))]
mod adapter_execution;
