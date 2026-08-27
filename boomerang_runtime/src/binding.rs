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

/// Rejects a descriptor/payload mismatch during const evaluation.
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
