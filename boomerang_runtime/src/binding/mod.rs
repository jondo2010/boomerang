//! Target-facing, dependency-free descriptor and payload compatibility values and const checks.
//!
//! Fingerprint hashing and encoding belong to host-side tooling, not this module.

/// Version of the Cargo environment protocol supplying payload compatibility inputs.
pub const PAYLOAD_COMPILE_INPUT_SCHEMA: u32 = 1;

/// Authoritative macro ABI understood by descriptors and payload facets in this release.
pub const COMPONENT_DESCRIPTOR_MACRO_ABI: u32 = 3;

/// Cargo environment key containing the host-expected decimal macro ABI.
pub const PAYLOAD_MACRO_ABI_COMPILE_INPUT: &str = "BOOMERANG_PAYLOAD_INPUT_V1_MACRO_ABI";

/// Cargo environment key prefix for a contract-specific host descriptor fingerprint.
pub const PAYLOAD_FINGERPRINT_COMPILE_INPUT_PREFIX: &str =
    "BOOMERANG_PAYLOAD_INPUT_V1_FINGERPRINT_";

/// Returns the fingerprint input key for one canonical consuming payload facet.
pub fn payload_fingerprint_compile_input_key(
    canonical_manifest_dir: &str,
    contract: &str,
    contract_version: u64,
    reactor_root: &str,
) -> String {
    use std::fmt::Write as _;

    let version = contract_version.to_string();
    let mut key = String::from(PAYLOAD_FINGERPRINT_COMPILE_INPUT_PREFIX);
    let mut separator = "";
    for (category, value) in [
        ('M', canonical_manifest_dir),
        ('C', contract),
        ('V', version.as_str()),
        ('R', reactor_root),
    ] {
        write!(key, "{separator}{category}{}_", value.len())
            .expect("writing into a String cannot fail");
        separator = "_";
        for byte in value.bytes() {
            write!(key, "{byte:02x}").expect("writing into a String cannot fail");
        }
    }
    key
}

/// Fingerprint of one canonical component implementation descriptor.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct DescriptorFingerprint([u8; 32]);

impl DescriptorFingerprint {
    /// Constructs a fingerprint from its complete byte representation.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the complete byte representation.
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Compares two fingerprints during const evaluation.
    pub const fn matches(self, other: Self) -> bool {
        let mut index = 0;
        while index < self.0.len() {
            if self.0[index] != other.0[index] {
                return false;
            }
            index += 1;
        }
        true
    }
}

/// Target-safe compatibility values for one generated payload facet.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BindingManifest {
    /// Canonical descriptor fingerprint computed by the host macro.
    descriptor_fingerprint: DescriptorFingerprint,
    /// Descriptor macro ABI expected by the generated payload facet.
    macro_abi: u32,
}

impl BindingManifest {
    /// Constructs a manifest from host-generated compatibility values.
    pub const fn new(descriptor_fingerprint: DescriptorFingerprint, macro_abi: u32) -> Self {
        Self {
            descriptor_fingerprint,
            macro_abi,
        }
    }

    /// Returns the canonical descriptor fingerprint for this payload facet.
    pub const fn descriptor_fingerprint(self) -> DescriptorFingerprint {
        self.descriptor_fingerprint
    }

    /// Returns the descriptor macro ABI expected by this payload facet.
    pub const fn macro_abi(self) -> u32 {
        self.macro_abi
    }
}

/// Const-asserts that a launcher's expected fingerprint matches its payload facet.
pub const fn assert_descriptor_fingerprint(
    expected: DescriptorFingerprint,
    actual: DescriptorFingerprint,
) {
    assert!(expected.matches(actual), "descriptor fingerprint mismatch");
}

#[cfg(test)]
mod tests {
    use super::{
        assert_descriptor_fingerprint, payload_fingerprint_compile_input_key,
        DescriptorFingerprint, COMPONENT_DESCRIPTOR_MACRO_ABI, PAYLOAD_COMPILE_INPUT_SCHEMA,
        PAYLOAD_MACRO_ABI_COMPILE_INPUT,
    };

    #[test]
    fn descriptor_fingerprint_comparison_is_const_capable() {
        const VALUE: DescriptorFingerprint = DescriptorFingerprint::new([0x5a; 32]);
        const SAME: bool = VALUE.matches(DescriptorFingerprint::new([0x5a; 32]));
        const DIFFERENT: bool = VALUE.matches(DescriptorFingerprint::new([0xa5; 32]));
        const _: () = assert_descriptor_fingerprint(VALUE, DescriptorFingerprint::new([0x5a; 32]));

        const { assert!(SAME) };
        const { assert!(!DIFFERENT) };
        assert_eq!(VALUE.to_bytes(), [0x5a; 32]);
    }

    #[test]
    fn payload_compile_input_keys_are_canonical_and_collision_free() {
        assert_eq!(PAYLOAD_COMPILE_INPUT_SCHEMA, 1);
        assert_eq!(COMPONENT_DESCRIPTOR_MACRO_ABI, 3);
        assert_eq!(
            PAYLOAD_MACRO_ABI_COMPILE_INPUT,
            "BOOMERANG_PAYLOAD_INPUT_V1_MACRO_ABI"
        );
        let key = payload_fingerprint_compile_input_key("/pkg", "example.sensor", 1, "Match");
        assert_eq!(key, "BOOMERANG_PAYLOAD_INPUT_V1_FINGERPRINT_M4_2f706b67_C14_6578616d706c652e73656e736f72_V1_31_R5_4d61746368");
        for distinct in [
            payload_fingerprint_compile_input_key("/other", "example.sensor", 1, "Match"),
            payload_fingerprint_compile_input_key("/pkg", "example.other", 1, "Match"),
            payload_fingerprint_compile_input_key("/pkg", "example.sensor", 2, "Match"),
            payload_fingerprint_compile_input_key("/pkg", "example.sensor", 1, "Other"),
        ] {
            assert_ne!(key, distinct);
        }
        assert_eq!(
            key,
            payload_fingerprint_compile_input_key("/pkg", "example.sensor", 1, "Match")
        );
    }
}
