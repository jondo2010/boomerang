use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    ops::Deref,
    str::FromStr,
};

/// Reports text that is not a canonical stable identity.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid stable identity '{value}': {reason}")]
pub struct InvalidStableId {
    /// Rejected identity text.
    value: Box<str>,
    /// Human-readable validation reason.
    reason: &'static str,
}

impl InvalidStableId {
    fn new(value: impl Into<Box<str>>, reason: &'static str) -> Self {
        Self {
            value: value.into(),
            reason,
        }
    }
}

/// One typed segment of a hierarchical stable identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StablePathSegment {
    /// User-provided printable Unicode name.
    Name(/** Name text before canonical escaping. */ Box<str>),
    /// Durable reactor-bank index.
    BankIndex(/** Non-negative bank index. */ u32),
    /// Durable compiler-generated ordinal.
    GeneratedOrdinal(/** Non-negative generated ordinal. */ u32),
}

/// Canonical, segment-aware hierarchical stable identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StablePath(
    /// Non-empty typed segments.
    Box<[StablePathSegment]>,
);

impl StablePath {
    /// Constructs a one-segment path from a user name.
    pub fn from_name(name: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
        Self::from_segments([StablePathSegment::Name(name.into())])
    }

    /// Constructs a non-empty path from typed segments.
    pub fn from_segments(
        segments: impl IntoIterator<Item = StablePathSegment>,
    ) -> Result<Self, InvalidStableId> {
        let segments = segments.into_iter().collect::<Vec<_>>();
        if segments.is_empty() {
            return Err(InvalidStableId::new("", "path is empty"));
        }
        for segment in &segments {
            if let StablePathSegment::Name(name) = segment {
                validate_name(name)?;
            }
        }
        Ok(Self(segments.into_boxed_slice()))
    }

    /// Appends a user-name segment.
    pub fn append_name(&self, name: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
        self.append(StablePathSegment::Name(name.into()))
    }

    /// Appends a typed bank-index segment.
    #[must_use]
    pub fn append_bank_index(&self, index: u32) -> Self {
        self.append(StablePathSegment::BankIndex(index))
            .expect("numeric segment is valid")
    }

    /// Appends a typed generated-ordinal segment.
    #[must_use]
    pub fn append_generated_ordinal(&self, ordinal: u32) -> Self {
        self.append(StablePathSegment::GeneratedOrdinal(ordinal))
            .expect("numeric segment is valid")
    }

    /// Returns the parent path, or `None` for a root segment.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        (self.0.len() > 1).then(|| Self(self.0[..self.0.len() - 1].into()))
    }

    /// Returns the typed path segments.
    pub fn segments(&self) -> &[StablePathSegment] {
        &self.0
    }

    fn append(&self, segment: StablePathSegment) -> Result<Self, InvalidStableId> {
        let mut segments = self.0.to_vec();
        segments.push(segment);
        Self::from_segments(segments)
    }
}

fn validate_name(name: &str) -> Result<(), InvalidStableId> {
    if name.is_empty() {
        return Err(InvalidStableId::new(name, "name segment is empty"));
    }
    if name.chars().any(char::is_control) {
        return Err(InvalidStableId::new(name, "control character in name"));
    }
    Ok(())
}

fn encode_name(name: &str, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for character in name.chars() {
        match character {
            '/' | '%' | '#' | '[' | ']' => {
                let mut bytes = [0; 4];
                for byte in character.encode_utf8(&mut bytes).bytes() {
                    write!(formatter, "%{byte:02X}")?;
                }
            }
            _ => formatter.write_str(character.encode_utf8(&mut [0; 4]))?,
        }
    }
    Ok(())
}

impl fmt::Display for StablePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, segment) in self.0.iter().enumerate() {
            if index != 0 {
                formatter.write_str("/")?;
            }
            match segment {
                StablePathSegment::Name(name) => encode_name(name, formatter)?,
                StablePathSegment::BankIndex(value) => write!(formatter, "#b{value}")?,
                StablePathSegment::GeneratedOrdinal(value) => write!(formatter, "#g{value}")?,
            }
        }
        Ok(())
    }
}

fn parse_number(value: &str, whole: &str) -> Result<u32, InvalidStableId> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return Err(InvalidStableId::new(whole, "noncanonical numeric segment"));
    }
    value
        .parse()
        .map_err(|_| InvalidStableId::new(whole, "numeric segment exceeds u32"))
}

fn decode_name(segment: &str, whole: &str) -> Result<Box<str>, InvalidStableId> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(InvalidStableId::new(whole, "malformed percent escape"));
            }
            let hex = |byte| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) else {
                return Err(InvalidStableId::new(
                    whole,
                    "percent escape is not uppercase hex",
                ));
            };
            let byte = high << 4 | low;
            if !matches!(byte, b'/' | b'%' | b'#' | b'[' | b']') {
                return Err(InvalidStableId::new(whole, "unnecessary percent escape"));
            }
            decoded.push(byte);
            index += 3;
        } else {
            let tail = &segment[index..];
            let character = tail
                .chars()
                .next()
                .ok_or_else(|| InvalidStableId::new(whole, "invalid UTF-8"))?;
            if character.is_control() || matches!(character, '#' | '[' | ']') {
                return Err(InvalidStableId::new(
                    whole,
                    "unescaped reserved or control character",
                ));
            }
            let length = character.len_utf8();
            decoded.extend_from_slice(&bytes[index..index + length]);
            index += length;
        }
    }
    let decoded = String::from_utf8(decoded)
        .map_err(|_| InvalidStableId::new(whole, "invalid UTF-8"))?
        .into_boxed_str();
    validate_name(&decoded)?;
    Ok(decoded)
}

impl FromStr for StablePath {
    type Err = InvalidStableId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || value.starts_with('/') || value.ends_with('/') {
            return Err(InvalidStableId::new(
                value,
                "path is empty or has an empty segment",
            ));
        }
        let mut segments = Vec::new();
        for segment in value.split('/') {
            let parsed = if let Some(number) = segment.strip_prefix("#b") {
                StablePathSegment::BankIndex(parse_number(number, value)?)
            } else if let Some(number) = segment.strip_prefix("#g") {
                StablePathSegment::GeneratedOrdinal(parse_number(number, value)?)
            } else {
                StablePathSegment::Name(decode_name(segment, value)?)
            };
            segments.push(parsed);
        }
        Self::from_segments(segments)
    }
}

/// Validated stable non-hierarchical text.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StableText(
    /// Validated non-hierarchical text.
    Box<str>,
);

impl StableText {
    /// Validates non-empty printable text without surrounding whitespace.
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
        let value = value.into();
        if value.is_empty() || value.trim() != &*value || value.chars().any(char::is_control) {
            Err(InvalidStableId::new(
                value,
                "text is empty, contains controls, or has surrounding whitespace",
            ))
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the validated text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StableText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

macro_rules! text_id {
    ($(#[$attribute:meta])* $name:ident) => {
        $(#[$attribute])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(
            /// Validated stable text.
            StableText,
        );

        impl $name {
            /// Validates and constructs this identity.
            pub fn new(value: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
                StableText::new(value).map(Self)
            }
            /// Returns the canonical identity text.
            pub fn as_str(&self) -> &str { self.0.as_str() }
        }
        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
        }
        impl FromStr for $name {
            type Err = InvalidStableId;
            fn from_str(value: &str) -> Result<Self, Self::Err> { Self::new(value) }
        }
    };
}

macro_rules! path_id {
    ($(#[$attribute:meta])* $name:ident) => {
        $(#[$attribute])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(
            /// Validated typed stable path.
            StablePath,
        );

        impl $name {
            /// Parses and constructs this canonical hierarchical identity.
            pub fn new(value: impl AsRef<str>) -> Result<Self, InvalidStableId> {
                value.as_ref().parse().map(Self)
            }
            /// Constructs this identity from an already validated typed path.
            pub fn from_path(path: StablePath) -> Self { Self(path) }
            /// Returns the typed stable path.
            pub fn path(&self) -> &StablePath { &self.0 }
            /// Returns the canonical identity text.
            pub fn to_canonical_string(&self) -> String { self.0.to_string() }
            /// Returns the parent identity, if it has one.
            pub fn parent(&self) -> Option<Self> { self.0.parent().map(Self) }
            /// Appends a user-name segment while retaining this identity type.
            pub fn append_name(&self, name: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
                self.0.append_name(name).map(Self)
            }
            /// Appends a bank-index segment while retaining this identity type.
            pub fn append_bank_index(&self, index: u32) -> Self { Self(self.0.append_bank_index(index)) }
            /// Appends a generated-ordinal segment while retaining this identity type.
            pub fn append_generated_ordinal(&self, ordinal: u32) -> Self { Self(self.0.append_generated_ordinal(ordinal)) }
        }
        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
        }
        impl FromStr for $name {
            type Err = InvalidStableId;
            fn from_str(value: &str) -> Result<Self, Self::Err> { Self::new(value) }
        }
        impl Deref for $name {
            type Target = StablePath;
            fn deref(&self) -> &Self::Target { &self.0 }
        }
    };
}

text_id!(
    /// Stable identity of an application topology.
    ApplicationId
);
text_id!(
    /// Stable identity of a component contract.
    ContractId
);
text_id!(
    /// Stable identity of a federated compilation member.
    FederateId
);
path_id!(
    /// Stable identity of a logical component instance.
    ComponentInstanceId
);
path_id!(
    /// Stable identity of a reactor instance.
    ReactorId
);
path_id!(
    /// Stable identity of an action.
    ActionId
);
path_id!(
    /// Stable identity of a port.
    PortId
);
path_id!(
    /// Stable identity of a reaction.
    ReactionId
);
path_id!(
    /// Stable identity of a mode.
    ModeId
);
path_id!(
    /// Stable identity of a source-declared placement group.
    PlacementGroupId
);
path_id!(
    /// Stable identity of a scheduler and logical-time domain.
    StableEnclaveId
);
path_id!(
    /// Stable identity of a logical recording or routing boundary.
    BoundaryId
);

/// Stable identity of a generated implementation binding slot.
pub struct BindingSlotId<T> {
    /// Typed hierarchical stable path.
    path: StablePath,
    /// Compile-time binding category without runtime representation.
    marker: PhantomData<fn() -> T>,
}

impl<T> BindingSlotId<T> {
    /// Parses and constructs a canonical binding-slot identity.
    pub fn new(value: impl AsRef<str>) -> Result<Self, InvalidStableId> {
        Ok(Self {
            path: value.as_ref().parse()?,
            marker: PhantomData,
        })
    }

    /// Constructs a binding identity from a typed path.
    pub fn from_path(path: StablePath) -> Self {
        Self {
            path,
            marker: PhantomData,
        }
    }

    /// Returns the typed stable path.
    pub fn path(&self) -> &StablePath {
        &self.path
    }

    /// Returns the parent slot identity, if it has one.
    pub fn parent(&self) -> Option<Self> {
        self.path.parent().map(Self::from_path)
    }

    /// Appends a user-name segment while retaining the binding category.
    pub fn append_name(&self, name: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
        self.path.append_name(name).map(Self::from_path)
    }

    /// Appends a bank-index segment while retaining the binding category.
    pub fn append_bank_index(&self, index: u32) -> Self {
        Self::from_path(self.path.append_bank_index(index))
    }

    /// Appends a generated ordinal while retaining the binding category.
    pub fn append_generated_ordinal(&self, ordinal: u32) -> Self {
        Self::from_path(self.path.append_generated_ordinal(ordinal))
    }
}

impl<T> Clone for BindingSlotId<T> {
    fn clone(&self) -> Self {
        Self::from_path(self.path.clone())
    }
}
impl<T> fmt::Debug for BindingSlotId<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BindingSlotId")
            .field(&self.path.to_string())
            .finish()
    }
}
impl<T> PartialEq for BindingSlotId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
impl<T> Eq for BindingSlotId<T> {}
impl<T> PartialOrd for BindingSlotId<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for BindingSlotId<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.path.cmp(&other.path)
    }
}
impl<T> Hash for BindingSlotId<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}
impl<T> fmt::Display for BindingSlotId<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.path.fmt(formatter)
    }
}
impl<T> FromStr for BindingSlotId<T> {
    type Err = InvalidStableId;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};

    struct TraitlessMarker;

    #[test]
    fn stable_path_round_trips_canonical_text() {
        let path = StablePath::from_name("vehicle")
            .unwrap()
            .append_name("sensor/#bank[0]")
            .unwrap()
            .append_bank_index(12)
            .append_generated_ordinal(3);
        assert_eq!(path.to_string(), "vehicle/sensor%2F%23bank%5B0%5D/#b12/#g3");
        assert_eq!(path.to_string().parse::<StablePath>().unwrap(), path);
        assert_eq!(
            path.parent().unwrap().to_string(),
            "vehicle/sensor%2F%23bank%5B0%5D/#b12"
        );
    }

    #[test]
    fn stable_path_rejects_noncanonical_aliases() {
        for invalid in [
            "",
            "/vehicle",
            "vehicle/",
            "vehicle//sensor",
            "vehicle/%2f",
            "vehicle/%41",
            "vehicle/%",
            "vehicle/#b01",
            "vehicle/#g00",
            "vehicle/#b4294967296",
            "vehicle/control\n",
        ] {
            assert!(
                invalid.parse::<StablePath>().is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn stable_path_rejects_unicode_adjacent_malformed_percent_escapes_without_panicking() {
        for invalid in ["vehicle/%Aé", "vehicle/%é", "vehicle/%Fé", "vehicle/é%"] {
            assert!(
                invalid.parse::<StablePath>().is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn typed_segments_cannot_collide_with_names() {
        let bank = StablePath::from_name("root").unwrap().append_bank_index(7);
        let named = StablePath::from_name("root")
            .unwrap()
            .append_name("#b7")
            .unwrap();
        assert_ne!(bank, named);
        assert_eq!(bank.to_string(), "root/#b7");
        assert_eq!(named.to_string(), "root/%23b7");
    }

    #[test]
    fn binding_slot_traits_depend_only_on_stable_identity() {
        let read = BindingSlotId::<TraitlessMarker>::new("sensor/read").unwrap();
        let write = BindingSlotId::<TraitlessMarker>::new("sensor/write").unwrap();
        assert_eq!(read, read.clone());
        assert!(read < write);
        assert!(HashSet::from([read.clone()]).contains(&read));
        assert_eq!(
            BTreeSet::from([write, read.clone()])
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
            ["sensor/read", "sensor/write"]
        );
        assert_eq!(format!("{read:?}"), "BindingSlotId(\"sensor/read\")");
    }

    #[test]
    fn binding_slot_navigation_retains_its_type_category() {
        let slot = BindingSlotId::<TraitlessMarker>::new("sensor")
            .unwrap()
            .append_name("#read")
            .unwrap()
            .append_bank_index(2)
            .append_generated_ordinal(4);
        let parent: BindingSlotId<TraitlessMarker> = slot.parent().unwrap();
        assert_eq!(slot.to_string(), "sensor/%23read/#b2/#g4");
        assert_eq!(parent.to_string(), "sensor/%23read/#b2");
    }
}
