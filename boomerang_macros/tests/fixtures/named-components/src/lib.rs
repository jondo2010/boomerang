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
    assert_eq!(
        first.fingerprint().to_bytes(),
        [
            0x65, 0x9c, 0x34, 0x77, 0xa3, 0xb6, 0x44, 0x3e, 0xa2, 0x0c, 0x46, 0x1f, 0x03, 0xe9,
            0xa0, 0x60, 0xba, 0x01, 0x12, 0x8e, 0x1f, 0xa9, 0x0a, 0xc3, 0x25, 0xb7, 0x76, 0xb3,
            0x10, 0xfa, 0xb3, 0x1a,
        ]
    );
    assert_eq!(
        second.fingerprint().to_bytes(),
        [
            0x8c, 0xa4, 0x7d, 0x63, 0xf4, 0x95, 0xf0, 0x95, 0x8c, 0x2f, 0xfb, 0x55, 0xbf, 0x16,
            0xa1, 0x85, 0x61, 0x34, 0x62, 0x77, 0x54, 0x40, 0xf2, 0x09, 0x41, 0xb5, 0x7f, 0x29,
            0x43, 0x1a, 0x2c, 0x8b,
        ]
    );
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
    assert_eq!(
        parent::keyboard::__boomerang::payload::binding_manifest()
            .descriptor_fingerprint()
            .to_bytes(),
        [
            0x65, 0x9c, 0x34, 0x77, 0xa3, 0xb6, 0x44, 0x3e, 0xa2, 0x0c, 0x46, 0x1f, 0x03, 0xe9,
            0xa0, 0x60, 0xba, 0x01, 0x12, 0x8e, 0x1f, 0xa9, 0x0a, 0xc3, 0x25, 0xb7, 0x76, 0xb3,
            0x10, 0xfa, 0xb3, 0x1a,
        ]
    );
    assert_eq!(
        parent::alternate::__boomerang::payload::binding_manifest()
            .descriptor_fingerprint()
            .to_bytes(),
        [
            0x8c, 0xa4, 0x7d, 0x63, 0xf4, 0x95, 0xf0, 0x95, 0x8c, 0x2f, 0xfb, 0x55, 0xbf, 0x16,
            0xa1, 0x85, 0x61, 0x34, 0x62, 0x77, 0x54, 0x40, 0xf2, 0x09, 0x41, 0xb5, 0x7f, 0x29,
            0x43, 0x1a, 0x2c, 0x8b,
        ]
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
