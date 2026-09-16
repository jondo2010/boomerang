pub mod parent {
    boomerang::component! {
        pub mod r#keyboard {
            #![deny(unsafe_code)]
            use boomerang::prelude::*;
            mod helper;
            #[cfg(feature = "sentinel")]
            compile_error!("helper sentinel reached");
            #[derive(Clone, Debug)]
            pub struct State { pub value: u32 }
            pub fn initial() -> State { State { value: helper::value() } }
            #[reactor(state = State, state_init = initial, contract = "named.same", contract_version = 1,
                bounds(queue_capacity = 4, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
            pub fn Root() -> impl Reactor {
                reaction! { start(startup) {
                    #[cfg(feature = "sentinel")]
                    compile_error!("reaction sentinel reached");
                    state.value += helper::value();
                } }
            }
        }
    }
    boomerang::component! {
        pub mod alternate {
            use boomerang::prelude::*;
            #[reactor(contract = "named.same", contract_version = 1,
                bounds(queue_capacity = 4, payload_bytes = 64, state_bytes = 64, scratch_bytes = 64))]
            pub fn Root(#[state] value: u32) -> impl Reactor {
                reaction! { start(startup) { state.value += 1; } }
            }
        }
    }
}

#[cfg(feature = "duplicate")]
boomerang::component! { pub mod parent { #[reactor] pub fn Root() -> impl Reactor {} } }
#[cfg(feature = "invalid")]
boomerang::component! { pub mod invalid { pub fn helper() {} } }
#[cfg(all(test, feature = "descriptor-assert"))]
#[test]
fn descriptors_are_scoped() {
    let first = parent::keyboard::__boomerang::descriptor();
    let second = parent::alternate::__boomerang::descriptor();
    assert_eq!(first.contract_id(), second.contract_id());
    assert_eq!(first.reactor_slots()[0].id, second.reactor_slots()[0].id);
    assert_ne!(
        first.descriptor_fingerprint_input().fingerprint(),
        second.descriptor_fingerprint_input().fingerprint()
    );
    assert_eq!(
        first
            .descriptor_fingerprint_input()
            .fingerprint()
            .to_bytes(),
        [
            0xe0, 0x05, 0x5d, 0xb4, 0xbe, 0x88, 0xcc, 0x8e, 0x0a, 0xba, 0xdd, 0x0f, 0x43, 0x14,
            0xcf, 0xa1, 0x76, 0xd1, 0xb4, 0x6c, 0x9b, 0x65, 0x73, 0x0b, 0xef, 0x68, 0x0f, 0x26,
            0x76, 0xa0, 0x8e, 0x6f
        ]
    );
    assert_eq!(
        second
            .descriptor_fingerprint_input()
            .fingerprint()
            .to_bytes(),
        [
            0x1a, 0xf1, 0x93, 0x67, 0xa6, 0xfe, 0x49, 0x03, 0xa6, 0xf7, 0xa2, 0x8a, 0x6a, 0x63,
            0x05, 0x13, 0xa1, 0x60, 0xc6, 0xd9, 0x39, 0x7e, 0x27, 0x77, 0xd3, 0x1c, 0x78, 0xe0,
            0x5f, 0x0c, 0x12, 0xa5
        ]
    );
}
#[cfg(all(test, feature = "payload-assert"))]
#[test]
fn payloads_use_distinct_scoped_inputs_and_owned_helpers() {
    assert_eq!(parent::keyboard::__boomerang::state_Root().value, 7);
    assert_eq!(parent::alternate::__boomerang::state_Root().value, 0);
    assert_eq!(
        KEYBOARD_MANIFEST.descriptor_fingerprint().to_bytes(),
        [
            0xe0, 0x05, 0x5d, 0xb4, 0xbe, 0x88, 0xcc, 0x8e, 0x0a, 0xba, 0xdd, 0x0f, 0x43, 0x14,
            0xcf, 0xa1, 0x76, 0xd1, 0xb4, 0x6c, 0x9b, 0x65, 0x73, 0x0b, 0xef, 0x68, 0x0f, 0x26,
            0x76, 0xa0, 0x8e, 0x6f
        ]
    );
    assert_eq!(
        ALTERNATE_MANIFEST.descriptor_fingerprint().to_bytes(),
        [
            0x1a, 0xf1, 0x93, 0x67, 0xa6, 0xfe, 0x49, 0x03, 0xa6, 0xf7, 0xa2, 0x8a, 0x6a, 0x63,
            0x05, 0x13, 0xa1, 0x60, 0xc6, 0xd9, 0x39, 0x7e, 0x27, 0x77, 0xd3, 0x1c, 0x78, 0xe0,
            0x5f, 0x0c, 0x12, 0xa5
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

#[cfg(any(feature = "payload-assert", feature = "subset-assert"))]
pub const KEYBOARD_MANIFEST: boomerang::runtime::binding::BindingManifest =
    parent::keyboard::__boomerang::binding_manifest();
#[cfg(feature = "payload-assert")]
pub const ALTERNATE_MANIFEST: boomerang::runtime::binding::BindingManifest =
    parent::alternate::__boomerang::binding_manifest();

#[cfg(all(test, feature = "subset-assert"))]
#[test]
fn only_the_selected_component_requires_inputs() {
    assert_eq!(
        KEYBOARD_MANIFEST.descriptor_fingerprint().to_bytes()[0],
        0xe0
    );
    assert_eq!(parent::keyboard::__boomerang::state_Root().value, 7);
}
