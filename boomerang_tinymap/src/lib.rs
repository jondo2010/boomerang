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
    ($vis:vis $name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        #[repr(transparent)]
        $vis struct $name(usize);

        impl $crate::Key for $name {
            fn index(&self) -> usize {
                self.0
            }
        }

        impl From<usize> for $name {
            fn from(value: usize) -> Self {
                Self(value)
            }
        }
    };
}

key_type!(pub DefaultKey);
