#![doc=include_str!( "../README.md")]
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
    };
}

key_type!(pub DefaultKey);
