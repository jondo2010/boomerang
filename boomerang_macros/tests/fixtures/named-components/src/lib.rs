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
            0x8a, 0x76, 0x83, 0x76, 0x76, 0x4c, 0x84, 0x4b, 0x78, 0x22, 0x6b, 0x61, 0x03, 0x45,
            0xfb, 0x76, 0xc5, 0x08, 0x0b, 0x4b, 0x07, 0xb7, 0x9c, 0x99, 0xef, 0xab, 0xfb, 0x0e,
            0xeb, 0x53, 0x55, 0x18
        ]
    );
    assert_eq!(
        second.fingerprint().to_bytes(),
        [
            0x2c, 0x88, 0xee, 0x6f, 0x62, 0xff, 0x41, 0x23, 0xa7, 0x0c, 0xba, 0xef, 0x5e, 0xe8,
            0x66, 0x86, 0x50, 0x91, 0x6e, 0x81, 0xb5, 0x1f, 0xe0, 0xc5, 0x6e, 0xd5, 0xc3, 0xf4,
            0x59, 0xdb, 0x08, 0x1a
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
            0x8a, 0x76, 0x83, 0x76, 0x76, 0x4c, 0x84, 0x4b, 0x78, 0x22, 0x6b, 0x61, 0x03, 0x45,
            0xfb, 0x76, 0xc5, 0x08, 0x0b, 0x4b, 0x07, 0xb7, 0x9c, 0x99, 0xef, 0xab, 0xfb, 0x0e,
            0xeb, 0x53, 0x55, 0x18
        ]
    );
    assert_eq!(
        parent::alternate::__boomerang::payload::binding_manifest()
            .descriptor_fingerprint()
            .to_bytes(),
        [
            0x2c, 0x88, 0xee, 0x6f, 0x62, 0xff, 0x41, 0x23, 0xa7, 0x0c, 0xba, 0xef, 0x5e, 0xe8,
            0x66, 0x86, 0x50, 0x91, 0x6e, 0x81, 0xb5, 0x1f, 0xe0, 0xc5, 0x6e, 0xd5, 0xc3, 0xf4,
            0x59, 0xdb, 0x08, 0x1a
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
