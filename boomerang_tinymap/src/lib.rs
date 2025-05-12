#![doc=include_str!( "../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(clippy::all)]

pub mod key_set;
pub mod map;
pub mod secondary_map;

pub use key_set::KeySet;
pub use map::TinyMap;
pub use secondary_map::TinySecondaryMap;

pub trait Key: From<usize> + Copy + Ord {
    fn index(&self) -> usize;
}

#[macro_export]
macro_rules! key_type {
    ($(#[$outer:meta])* $vis:vis $name:ident) => {
        $(#[$outer])*
        #[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        $vis struct $name(u64);

        impl $crate::Key for $name {
            fn index(&self) -> usize {
                self.0 as usize
            }
        }

        impl From<usize> for $name {
            fn from(value: usize) -> Self {
                Self(value as _)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                const PREFIX: &str = concat!(stringify!($name), "(");
                if s.starts_with(PREFIX) && s.ends_with(')') {
                    let inner = &s[PREFIX.len()..s.len() - 1];
                    inner
                        .parse::<u64>()
                        .map(Self)
                        .map_err(|_| format!("Failed to parse inner value: {}", inner))
                } else {
                    Err(format!("Invalid format for {}: {}", stringify!($name), s))
                }
            }
        }
    };
}

key_type!(pub DefaultKey);

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_key_type() {
        let key = DefaultKey(42);
        assert_eq!(key.index(), 42);
        assert_eq!(DefaultKey::from(42), key);
        assert_eq!(key.to_string(), "DefaultKey(42)");
        assert_eq!(DefaultKey::from_str("DefaultKey(42)").unwrap(), key);
    }
}
