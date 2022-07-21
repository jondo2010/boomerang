#![feature(map_first_last)]
#![feature(type_name_of_val)]
#![feature(new_uninit)]
#![feature(split_array)]
#![feature(type_alias_impl_trait)]

mod action;
mod context;
mod disjoint;
mod env;
mod key_set;
mod port;
mod reaction;
mod reactor;
mod sched;
mod time;
pub mod util;

pub use action::*;
pub use context::*;
pub use env::*;
pub use port::*;
pub use reaction::*;
pub use reactor::*;
pub use sched::*;
pub use time::*;

pub use std::time::{Duration, Instant};

use slotmap::SlotMap;

#[macro_use]
extern crate derivative;

pub trait PortData: std::fmt::Debug + Send + Sync + 'static {}
impl<T> PortData for T where T: std::fmt::Debug + Send + Sync + 'static {}

/// Used to get access to the inner type from Port, Action, etc.
pub trait InnerType {
    type Inner: PortData;
}

#[derive(thiserror::Error, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    #[error("Port Key not found: {}", 0)]
    PortKeyNotFound(PortKey),

    #[error("Mismatched Dynamic Types found {} but wanted {}", found, wanted)]
    TypeMismatch {
        found: &'static str,
        wanted: &'static str,
    },
}

/// Returns a tuple of disjoint sets of (immutable, mutable) borrows from the SlotMap.
/// This is only safe if:
/// 1. All keys in `mut_keys` are disjoint from each other and the keys in `keys`.
/// 2. All keys in `keys` are disjoint from those in `mut_keys`.
pub unsafe fn disjoint_unchecked<'sm, K, V, I1, I2>(
    sm: &'sm mut SlotMap<K, V>,
    keys: I1,
    mut_keys: I2,
) -> (Box<[&'sm V]>, Box<[&'sm mut V]>)
where
    K: slotmap::Key,
    I1: ExactSizeIterator + Iterator<Item = K>,
    I2: ExactSizeIterator + Iterator<Item = K>,
{
    let mut iptrs = Box::<[*const V]>::new_uninit_slice(keys.len());
    for (ptr, key) in iptrs.iter_mut().zip(keys) {
        ptr.as_mut_ptr().write(sm.get_unchecked(key))
    }
    let mut optrs = Box::<[*const V]>::new_uninit_slice(mut_keys.len());
    for (ptr, key) in optrs.iter_mut().zip(mut_keys) {
        ptr.as_mut_ptr().write(sm.get_unchecked_mut(key))
    }
    let iraw = core::mem::transmute_copy(&Box::into_raw(iptrs.assume_init()));
    let oraw = core::mem::transmute_copy(&Box::into_raw(optrs.assume_init()));
    (Box::from_raw(iraw), Box::from_raw(oraw))
}
