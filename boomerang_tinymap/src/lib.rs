#![no_std]
#![doc=include_str!( "../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(clippy::all)]

#[cfg(feature = "alloc")]
#[macro_use]
pub extern crate alloc;
#[cfg(test)]
extern crate std;

mod error;
#[cfg(feature = "alloc")]
pub mod key_set;
#[cfg(feature = "alloc")]
pub mod map;
mod range;
#[cfg(feature = "alloc")]
pub mod secondary_map;
pub mod tiny_vec;

pub use error::TinyMapError;
#[cfg(feature = "alloc")]
pub use key_set::KeySet;
#[cfg(feature = "alloc")]
pub use map::{CapacityError, TinyMap, TinyMapView};
pub use range::{IndexSpan, SliceRange};
#[cfg(feature = "alloc")]
pub use secondary_map::TinySecondaryMap;
pub use tiny_vec::{BorrowedStorage, InlineStorage, TinyVecBuilder};

/// A key that identifies a value by its dense table index.
pub trait Key: From<usize> + Copy + Ord {
    /// The greatest length of a table this key can index.
    const MAX_LEN: usize = usize::MAX;

    /// Returns this key's table index.
    fn index(&self) -> usize;
}

#[macro_export]
macro_rules! key_type {
    ($(#[$outer:meta])* $vis:vis $name:ident) => {
        $(#[$outer])*
        #[doc = concat!("A u32-backed dense key named [`", stringify!($name), "`].")]
        #[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        $vis struct $name(u32);

        impl $name {
            /// Creates a key from its u32 representation.
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            /// Returns this key's u32 representation.
            pub const fn as_u32(self) -> u32 {
                self.0
            }
        }

        impl $crate::Key for $name {
            const MAX_LEN: usize = if usize::BITS > u32::BITS {
                u32::MAX as usize + 1
            } else {
                usize::MAX
            };

            fn index(&self) -> usize {
                self.0 as usize
            }
        }

        impl ::core::convert::From<usize> for $name {
            fn from(value: usize) -> Self {
                match <u32 as ::core::convert::TryFrom<usize>>::try_from(value) {
                    Ok(value) => Self(value),
                    Err(_) => panic!("dense key index exceeds u32 range"),
                }
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        $crate::__key_type_from_str!($name);
    };
}

#[cfg(feature = "alloc")]
#[doc(hidden)]
#[macro_export]
macro_rules! __key_type_from_str {
    ($name:ident) => {
        impl ::core::str::FromStr for $name {
            type Err = $crate::alloc::string::String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                const PREFIX: &str = concat!(stringify!($name), "(");
                if s.starts_with(PREFIX) && s.ends_with(')') {
                    let inner = &s[PREFIX.len()..s.len() - 1];
                    inner
                        .parse::<u32>()
                        .map(Self)
                        .map_err(|_| format!("Failed to parse inner value: {}", inner))
                } else {
                    Err(format!("Invalid format for {}: {}", stringify!($name), s))
                }
            }
        }
    };
}

#[cfg(not(feature = "alloc"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __key_type_from_str {
    ($name:ident) => {};
}

key_type!(pub DefaultKey);

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use std::{str::FromStr, string::ToString};

    #[test]
    fn test_key_type() {
        let key = DefaultKey(42);
        assert_eq!(key.index(), 42);
        assert_eq!(DefaultKey::from(42), key);
        assert_eq!(key.to_string(), "DefaultKey(42)");
        assert_eq!(DefaultKey::from_str("DefaultKey(42)").unwrap(), key);
    }
}
