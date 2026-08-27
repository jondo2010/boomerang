//! Target-facing, dependency-free descriptor and payload compatibility values and const checks.
//!
//! Fingerprint hashing and encoding belong to host-side tooling, not this module.

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
    use super::{assert_descriptor_fingerprint, DescriptorFingerprint};

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
}
