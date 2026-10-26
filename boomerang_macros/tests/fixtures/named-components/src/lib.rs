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
            0xe7, 0x5c, 0x58, 0xe1, 0xed, 0x6b, 0x97, 0x51, 0xa2, 0xf5, 0xef, 0xac, 0xf1, 0x1e,
            0x6b, 0x91, 0x7a, 0x86, 0x09, 0x41, 0xc5, 0x9d, 0xbf, 0x5c, 0x2c, 0x81, 0x93, 0x0b,
            0x89, 0xde, 0xd7, 0x64,
        ]
    );
    assert_eq!(
        second.fingerprint().to_bytes(),
        [
            0xcc, 0xfa, 0xd3, 0x25, 0x15, 0x68, 0x0a, 0x46, 0xbe, 0x51, 0xab, 0x11, 0x48, 0x9a,
            0xd7, 0x06, 0x6f, 0x31, 0xcf, 0x92, 0x7d, 0x43, 0xef, 0x71, 0x96, 0x27, 0x5f, 0xc2,
            0xd0, 0x95, 0x31, 0xa1,
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
        <parent::keyboard::__boomerang::payload::Payload as boomerang::runtime::binding::ComponentPayload>::DESCRIPTOR_FINGERPRINT
            .to_bytes(),
        [
            0xe7, 0x5c, 0x58, 0xe1, 0xed, 0x6b, 0x97, 0x51, 0xa2, 0xf5, 0xef, 0xac, 0xf1, 0x1e,
            0x6b, 0x91, 0x7a, 0x86, 0x09, 0x41, 0xc5, 0x9d, 0xbf, 0x5c, 0x2c, 0x81, 0x93, 0x0b,
            0x89, 0xde, 0xd7, 0x64,
        ]
    );
    assert_eq!(
        <parent::alternate::__boomerang::payload::Payload as boomerang::runtime::binding::ComponentPayload>::DESCRIPTOR_FINGERPRINT
            .to_bytes(),
        [
            0xcc, 0xfa, 0xd3, 0x25, 0x15, 0x68, 0x0a, 0x46, 0xbe, 0x51, 0xab, 0x11, 0x48, 0x9a,
            0xd7, 0x06, 0x6f, 0x31, 0xcf, 0x92, 0x7d, 0x43, 0xef, 0x71, 0x96, 0x27, 0x5f, 0xc2,
            0xd0, 0x95, 0x31, 0xa1,
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
