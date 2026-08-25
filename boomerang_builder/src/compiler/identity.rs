use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    str::FromStr,
};

/// Reports a stable identity that is not a non-empty canonical path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidStableId {
    /// The rejected identity text.
    value: Box<str>,
}

impl fmt::Display for InvalidStableId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid stable identity '{}'", self.value)
    }
}

impl std::error::Error for InvalidStableId {}

fn validate(value: Box<str>) -> Result<Box<str>, InvalidStableId> {
    if value.is_empty()
        || value.starts_with('/')
        || value.ends_with('/')
        || value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        Err(InvalidStableId { value })
    } else {
        Ok(value)
    }
}

macro_rules! stable_id {
    ($(#[$attribute:meta])* $name:ident) => {
        $(#[$attribute])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(
            /// Validated stable identity text.
            Box<str>,
        );

        impl $name {
            /// Validates and constructs a stable identity.
            pub fn new(value: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
                validate(value.into()).map(Self)
            }

            /// Returns the stable identity text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = InvalidStableId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }
    };
}

stable_id!(
    /// Stable identity of an application topology.
    ApplicationId
);
stable_id!(
    /// Stable identity of a logical component instance.
    ComponentInstanceId
);
stable_id!(
    /// Stable identity and version of a component contract.
    ContractId
);
stable_id!(
    /// Stable identity of a source-declared placement group.
    PlacementGroupId
);
stable_id!(
    /// Stable identity of a scheduler and logical-time domain.
    StableEnclaveId
);
stable_id!(
    /// Stable identity of a logical recording or routing boundary.
    BoundaryId
);

/// Stable identity of a generated implementation binding slot.
pub struct BindingSlotId<T> {
    /// Validated stable identity text.
    value: Box<str>,
    /// Compile-time binding value category without a runtime representation.
    marker: PhantomData<fn() -> T>,
}

impl<T> Clone for BindingSlotId<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            marker: PhantomData,
        }
    }
}

impl<T> fmt::Debug for BindingSlotId<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BindingSlotId")
            .field(&self.value)
            .finish()
    }
}

impl<T> PartialEq for BindingSlotId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
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
        self.value.cmp(&other.value)
    }
}

impl<T> Hash for BindingSlotId<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<T> BindingSlotId<T> {
    /// Validates and constructs a binding slot identity.
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, InvalidStableId> {
        Ok(Self {
            value: validate(value.into())?,
            marker: PhantomData,
        })
    }

    /// Returns the stable binding slot identity text.
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl<T> fmt::Display for BindingSlotId<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.value)
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
    use std::collections::{BTreeSet, HashSet};

    use super::*;

    /// Marker deliberately providing no identity-related traits.
    struct TraitlessMarker;

    #[test]
    fn binding_slot_traits_depend_only_on_stable_identity() {
        let read = BindingSlotId::<TraitlessMarker>::new("sensor/read").unwrap();
        let read_clone = read.clone();
        let write = BindingSlotId::<TraitlessMarker>::new("sensor/write").unwrap();

        assert_eq!(read, read_clone);
        assert!(read < write);

        let mut hashed = HashSet::new();
        hashed.insert(read.clone());
        assert!(hashed.contains(&read));

        let mut ordered = BTreeSet::new();
        ordered.insert(write);
        ordered.insert(read.clone());
        assert_eq!(
            ordered
                .iter()
                .map(BindingSlotId::as_str)
                .collect::<Vec<_>>(),
            ["sensor/read", "sensor/write"]
        );
        assert_eq!(format!("{read:?}"), "BindingSlotId(\"sensor/read\")");
    }
}
