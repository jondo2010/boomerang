//! Target-facing, dependency-free descriptor and payload compatibility values and const checks.
//!
//! Fingerprint hashing and encoding belong to host-side tooling, not this module.

/// Authoritative macro ABI understood by component descriptors and payloads in this release.
///
/// ABI 5 retains the erased `reaction_*` callbacks accepted by
/// [`crate::EnclaveBindings::bind_reaction`]. Payload macros own state downcasts,
/// reference extraction, argument order, and generic specialization.
/// Deployment generation supplies binding slots and image-owned mode effects.
/// `binding_manifest()` exports one descriptor fingerprint that includes this ABI version.
pub const COMPONENT_DESCRIPTOR_MACRO_ABI: u32 = 5;

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

/// Target-safe compatibility contract for one component payload.
///
/// The fingerprint must derive from the payload declaration and include the macro ABI.
/// Generated deployment code must compare it with the selected metadata descriptor fingerprint.
/// Cargo feature resolution and target configuration do not establish this agreement.
/// Rust checks exported symbols and signatures; the fingerprint checks descriptor semantics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BindingManifest {
    /// Canonical descriptor fingerprint computed by the host macro.
    descriptor_fingerprint: DescriptorFingerprint,
}

impl BindingManifest {
    /// Constructs a manifest from a declaration-derived fingerprint that includes the macro ABI.
    pub const fn new(descriptor_fingerprint: DescriptorFingerprint) -> Self {
        Self {
            descriptor_fingerprint,
        }
    }

    /// Returns the canonical descriptor fingerprint for this component payload.
    pub const fn descriptor_fingerprint(self) -> DescriptorFingerprint {
        self.descriptor_fingerprint
    }
}

/// Const-asserts that an application's expected fingerprint matches its component payload.
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
