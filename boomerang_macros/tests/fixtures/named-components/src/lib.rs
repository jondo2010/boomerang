pub mod parent {
    boomerang::component! {
    pub mod r#keyboard {
        #![deny(unsafe_code)]
        use boomerang::prelude::*;
        mod helper;
        #[cfg(feature = "sentinel")]
        compile_error!("helper sentinel reached");
        #[derive(Clone, Debug)]
        pub struct State {
            /// Value returned by the private owning-module helper.
            pub value: u32,
        }
        pub fn initial() -> State {
            State {
                value: self::helper::value(),
            }
        }
        #[reactor(state = State, state_init = initial, contract = "named.same", contract_version = 1,
                    bounds(queue_capacity = 4, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
        pub fn Root() -> impl Reactor {
            reaction! { start(startup) {
                #[cfg(feature = "sentinel")]
                compile_error!("reaction sentinel reached");
                state.value += self::helper::value();
            } }
        }
    }
}
    boomerang::component! {
    pub mod alternate {
        use boomerang::prelude::*;
        #[reactor(
            contract = "named.same",
            contract_version = 1,
            bounds(
                queue_capacity = 4,
                payload_bytes = 64,
                state_bytes = 64,
                scratch_bytes = 64
            )
        )]
        pub fn Root(#[state] value: u32) -> impl Reactor {
            reaction! { start(startup) { state.value += 1; } }
        }
    }
}
}

#[cfg(feature = "duplicate")]
boomerang::component! {
    pub mod parent {
        #[reactor]
        pub fn Root() -> impl Reactor {}
    }
}
#[cfg(feature = "invalid")]
boomerang::component! {
    pub mod invalid {
        pub fn helper() {}
    }
}
#[cfg(all(test, feature = "metadata"))]
#[test]
fn descriptors_are_scoped() {
    let first = parent::keyboard::__boomerang::metadata::descriptor();
    let second = parent::alternate::__boomerang::metadata::descriptor();
    assert_eq!(first.contract_id(), second.contract_id());
    assert_eq!(first.reactor_slots()[0].id, second.reactor_slots()[0].id);
    assert_ne!(first.fingerprint(), second.fingerprint());
}
#[cfg(all(test, feature = "payload"))]
#[test]
fn payloads_derive_distinct_fingerprints_and_use_owned_helpers() {
    assert_eq!(
        parent::keyboard::__boomerang::payload::state_Root().value,
        7
    );
    assert_eq!(
        parent::alternate::__boomerang::payload::state_Root().value,
        0
    );
    assert_ne!(
        <parent::keyboard::__boomerang::payload::Payload as boomerang::runtime::binding::ComponentPayload>::DESCRIPTOR_FINGERPRINT,
        <parent::alternate::__boomerang::payload::Payload as boomerang::runtime::binding::ComponentPayload>::DESCRIPTOR_FINGERPRINT,
    );
}

#[cfg(feature = "cfg-root")]
boomerang::component! {
    pub mod attributed {
        use boomerang::prelude::*;
        #[cfg(any())]
        #[reactor]
        pub fn Root() -> impl Reactor {}
    }
}

#[cfg(feature = "cfg-attr-root")]
boomerang::component! {
    pub mod attributed {
        use boomerang::prelude::*;
        #[cfg_attr(all(), cfg(any()))]
        #[reactor]
        pub fn Root() -> impl Reactor {}
    }
}

#[cfg(feature = "deprecated-root")]
boomerang::component! {
    pub mod attributed {
        use boomerang::prelude::*;
        #[deprecated]
        #[reactor]
        pub fn Root() -> impl Reactor {}
    }
}

#[cfg(feature = "multiple")]
boomerang::component! {
    pub mod multiple {
        #[reactor]
        fn First() -> impl Reactor {}
        #[reactor]
        fn Second() -> impl Reactor {}
    }
}
