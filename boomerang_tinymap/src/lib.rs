pub mod chunks;
pub mod map;
pub mod secondary;

pub use map::TinyMap;
pub use secondary::TinySecondaryMap;

pub trait Key: From<usize> + Copy {
    fn index(&self) -> usize;
}

#[macro_export(local_inner_macros)]
macro_rules! key_type {
    ($(#[$outer:meta])* $vis:vis $name:ident) => {
        $(#[$outer])*
        #[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    };
}

key_type!(pub DefaultKey);
