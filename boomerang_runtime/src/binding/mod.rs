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

/// Returns the input key for the named component table sharing one reactor contract.
/// Each newline-delimited record is `full_rust_module_path=lowercase_hex_fingerprint`.
pub fn component_payload_fingerprint_compile_inputs_key(
    canonical_manifest_dir: &str,
    contract: &str,
    contract_version: u64,
    reactor_root: &str,
) -> String {
    let mut key = payload_fingerprint_compile_input_key(
        canonical_manifest_dir,
        contract,
        contract_version,
        reactor_root,
    );
    key.push_str("_COMPONENTS");
    key
}

/// Resolves a selected component's header during launcher const evaluation.
/// Unselected components may capture absent inputs without evaluating this function.
/// Raw identifier prefixes are ignored when comparing complete module-path segments.
pub const fn component_payload_binding_manifest(
    inputs: Option<&str>,
    module_path: &str,
    macro_abi: Option<&str>,
) -> BindingManifest {
    let Some(macro_abi) = macro_abi else {
        panic!("missing payload macro ABI compile input");
    };
    validate_payload_macro_abi_compile_input(macro_abi);
    let Some(inputs) = inputs else {
        panic!("missing payload descriptor fingerprint compile input for selected component");
    };
    let bytes = inputs.as_bytes();
    let mut position = 0;
    let mut fingerprint = None;
    while position < bytes.len() {
        let record_start = position;
        while position < bytes.len() && bytes[position] != b'\n' {
            position += 1;
        }
        let (_, remainder) = bytes.split_at(record_start);
        let (record, _) = remainder.split_at(position - record_start);
        let mut separator = 0;
        while separator < record.len() && record[separator] != b'=' {
            separator += 1;
        }
        assert!(
            separator > 0 && separator < record.len(),
            "malformed component payload fingerprint input record"
        );
        let (path, fingerprint_input) = record.split_at(separator);
        let (_, fingerprint_input) = fingerprint_input.split_at(1);
        let value = payload_fingerprint_from_compile_input_bytes(fingerprint_input);
        if component_module_paths_match(path, module_path.as_bytes()) {
            assert!(
                fingerprint.is_none(),
                "duplicate component payload fingerprint input record"
            );
            fingerprint = Some(value);
        }
        position += 1;
    }
    let Some(fingerprint) = fingerprint else {
        panic!("missing payload descriptor fingerprint compile input for selected component");
    };
    BindingManifest::new(fingerprint, COMPONENT_DESCRIPTOR_MACRO_ABI)
}

const fn component_module_paths_match(left: &[u8], right: &[u8]) -> bool {
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() && right_index < right.len() {
        if (left_index == 0
            || (left_index >= 2 && left[left_index - 2] == b':' && left[left_index - 1] == b':'))
            && left_index + 1 < left.len()
            && left[left_index] == b'r'
            && left[left_index + 1] == b'#'
        {
            left_index += 2;
        }
        if (right_index == 0
            || (right_index >= 2
                && right[right_index - 2] == b':'
                && right[right_index - 1] == b':'))
            && right_index + 1 < right.len()
            && right[right_index] == b'r'
            && right[right_index + 1] == b'#'
        {
            right_index += 2;
        }
        if left_index >= left.len()
            || right_index >= right.len()
            || left[left_index] != right[right_index]
        {
            return false;
        }
        left_index += 1;
        right_index += 1;
    }
    left_index == left.len() && right_index == right.len()
}

/// Parses the canonical lowercase hexadecimal fingerprint during target compilation.
pub const fn payload_fingerprint_from_compile_input(value: &str) -> DescriptorFingerprint {
    payload_fingerprint_from_compile_input_bytes(value.as_bytes())
}

const fn payload_fingerprint_from_compile_input_bytes(input: &[u8]) -> DescriptorFingerprint {
    const fn digit(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            _ => panic!("payload descriptor fingerprint must be exactly 64 lowercase hex digits"),
        }
    }
    assert!(
        input.len() == 64,
        "payload descriptor fingerprint must be exactly 64 lowercase hex digits"
    );
    let mut bytes = [0; 32];
    let mut index = 0;
    while index < bytes.len() {
        bytes[index] = digit(input[2 * index]) * 16 + digit(input[2 * index + 1]);
        index += 1;
    }
    DescriptorFingerprint::new(bytes)
}

/// Validates the descriptor macro ABI supplied to the target compiler.
pub const fn validate_payload_macro_abi_compile_input(value: &str) {
    let input = value.as_bytes();
    assert!(
        !input.is_empty(),
        "payload macro ABI compile input must be a decimal u32"
    );
    let mut result = 0u32;
    let mut index = 0;
    while index < input.len() {
        let digit = input[index];
        assert!(
            digit >= b'0' && digit <= b'9',
            "payload macro ABI compile input must be a decimal u32"
        );
        let Some(next) = result.checked_mul(10) else {
            panic!("payload macro ABI compile input must be a decimal u32");
        };
        let Some(next) = next.checked_add((digit - b'0') as u32) else {
            panic!("payload macro ABI compile input must be a decimal u32");
        };
        result = next;
        index += 1;
    }
    assert!(
        result == COMPONENT_DESCRIPTOR_MACRO_ABI,
        "payload macro ABI mismatch"
    );
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
