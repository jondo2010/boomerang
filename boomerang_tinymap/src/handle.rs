//! Stable, non-owning handles for pinned generated-image roots.
//!
//! A generated root owns its sealed tables and concrete heterogeneous values. It also contains a
//! pre-sized [`HandleTable`] for each runtime handle domain. The root may move after construction
//! and sealing, but its generated `link` method must accept `Pin<&mut Root>` and project both the
//! fixed handle table and its targets from that pinned owner. Linking fills the existing slots and
//! returns a [`LinkedHandleTable`] guard; runtime code can obtain [`Handle`] values only from that
//! guard. The guard and its handles borrow the pinned root, so they are gone before the root drops.
//!
//! `HandleTable` is infrastructure for generated roots, not a heterogeneous owner. Concrete values
//! remain ordinary root fields, arrays, or sealed tables. The table stores only non-owning pointers,
//! and [`Handle`] never destroys its target. If shutdown order matters, perform it explicitly while
//! the linked guard is live; normal `Drop` is memory cleanup and promises no value-drop order.
//!
//! The generated API makes linking unavailable until the root is pinned:
//!
//! ```compile_fail,E0599
//! # use core::pin::Pin;
//! # use boomerang_tinymap::{key_type, HandleTable, InlineStorage, LinkedHandleTable, SealedTinyMap, TinyMapBuilder};
//! # key_type!(ItemKey);
//! # struct Image { keys: SealedTinyMap<ItemKey, (), InlineStorage<(), 1>>, value: u8, handles: HandleTable<ItemKey, u8, 1> }
//! # impl Image {
//! #   fn link(self: Pin<&mut Self>) -> LinkedHandleTable<'_, ItemKey, u8, 1> {
//! #     unsafe {
//! #       let image = self.get_unchecked_mut();
//! #       Pin::new_unchecked(&mut image.handles).link([&image.value])
//! #     }
//! #   }
//! # }
//! # let mut keys = TinyMapBuilder::<ItemKey, (), InlineStorage<(), 1>>::inline();
//! # let _key = keys.try_insert(()).unwrap();
//! let mut image = Image { keys: keys.seal(), value: 7, handles: HandleTable::new() };
//! let linked = image.link();
//! ```
//!
//! A handle also cannot escape the pinned owning root:
//!
//! ```compile_fail,E0597
//! # use core::pin::{pin, Pin};
//! # use boomerang_tinymap::{key_type, HandleTable, InlineStorage, LinkedHandleTable, SealedTinyMap, TinyMapBuilder};
//! # key_type!(ItemKey);
//! # struct Image { keys: SealedTinyMap<ItemKey, (), InlineStorage<(), 1>>, value: u8, handles: HandleTable<ItemKey, u8, 1> }
//! # impl Image {
//! #   fn link(self: Pin<&mut Self>) -> LinkedHandleTable<'_, ItemKey, u8, 1> {
//! #     unsafe {
//! #       let image = self.get_unchecked_mut();
//! #       Pin::new_unchecked(&mut image.handles).link([&image.value])
//! #     }
//! #   }
//! # }
//! let escaped = {
//!     let mut keys = TinyMapBuilder::<ItemKey, (), InlineStorage<(), 1>>::inline();
//!     let key = keys.try_insert(()).unwrap();
//!     let image = Image { keys: keys.seal(), value: 7, handles: HandleTable::new() };
//!     let mut image = pin!(image);
//!     let linked = image.as_mut().link();
//!     linked.handle(key).unwrap()
//! };
//! let _value = escaped.get();
//! ```
//!
//! Handles have the same thread-transfer requirement as shared references: the target must be
//! [`Sync`].
//!
//! ```compile_fail,E0277
//! # use core::cell::Cell;
//! # use boomerang_tinymap::{key_type, Handle};
//! # key_type!(ItemKey);
//! # fn require_send<T: Send>() {}
//! require_send::<Handle<'static, ItemKey, Cell<u8>>>();
//! ```

use core::{
    marker::{PhantomData, PhantomPinned},
    pin::Pin,
    ptr::NonNull,
};

use crate::Key;

/// Fixed-capacity private storage for stable handles created after an owning root is pinned.
pub struct HandleTable<K: Key, T: ?Sized, const N: usize> {
    slots: [Option<NonNull<T>>; N],
    key: PhantomData<fn(K)>,
    _pinned: PhantomPinned,
}

// SAFETY: the table owns no targets and exposes no pointer dereference. Linking needs exclusive
// pinned access and publishes dereference only through guards requiring `T: Sync`. A table that is
// moved before pinning contains either empty or stale, inaccessible pointers; linking overwrites
// every slot before publishing the next guard.
unsafe impl<K: Key, T: ?Sized + Sync, const N: usize> Send for HandleTable<K, T, N> {}

// SAFETY: shared access to an unlinked table exposes no slots, while linked access is represented
// by `LinkedHandleTable`, whose reference-like thread traits also require `T: Sync`.
unsafe impl<K: Key, T: ?Sized + Sync, const N: usize> Sync for HandleTable<K, T, N> {}

impl<K: Key, T: ?Sized, const N: usize> HandleTable<K, T, N> {
    /// Creates an empty handle table with exactly `N` slots.
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            key: PhantomData,
            _pinned: PhantomPinned,
        }
    }

    /// Fills every handle slot and publishes a shared-only linked guard.
    ///
    /// # Safety
    ///
    /// Every target must have a stable address for `'a`. The handle table and all targets must be
    /// disjoint fields owned by the same pinned root, and `targets[key.index()]` must identify the
    /// value owned by that key. The root must remain pinned and structurally unchanged until the
    /// returned guard and every [`Handle`] obtained from it have been dropped. No mutable reference
    /// to a target may exist during `'a`.
    pub unsafe fn link<'a>(
        self: Pin<&'a mut Self>,
        targets: [&'a T; N],
    ) -> LinkedHandleTable<'a, K, T, N> {
        // SAFETY: the method does not move the table; it only overwrites fixed-size pointer slots.
        let table = unsafe { self.get_unchecked_mut() };
        for (slot, target) in table.slots.iter_mut().zip(targets) {
            *slot = Some(NonNull::from(target));
        }
        LinkedHandleTable {
            table,
            targets: PhantomData,
        }
    }
}

impl<K: Key, T: ?Sized, const N: usize> Default for HandleTable<K, T, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared execution-phase access to a fully linked handle table.
pub struct LinkedHandleTable<'a, K: Key, T: ?Sized, const N: usize> {
    table: &'a HandleTable<K, T, N>,
    targets: PhantomData<&'a T>,
}

impl<'a, K: Key, T: ?Sized, const N: usize> LinkedHandleTable<'a, K, T, N> {
    /// Returns the stable handle associated with an owner-issued key.
    pub fn handle(&self, key: K) -> Option<Handle<'a, K, T>> {
        let pointer = *self.table.slots.get(key.index())?.as_ref()?;
        Some(Handle {
            pointer,
            key: PhantomData,
            target: PhantomData,
        })
    }
}

/// A copyable, non-owning reference into a pinned owning root.
pub struct Handle<'a, K: Key, T: ?Sized> {
    pointer: NonNull<T>,
    key: PhantomData<fn(K)>,
    target: PhantomData<&'a T>,
}

// SAFETY: `Handle<'a, K, T>` has the same access semantics as `&'a T`; it exposes shared
// dereference only, and `T: Sync` permits that reference to cross threads.
unsafe impl<K: Key, T: ?Sized + Sync> Send for Handle<'_, K, T> {}

// SAFETY: as above, sharing a handle is equivalent to sharing `&T` and requires `T: Sync`.
unsafe impl<K: Key, T: ?Sized + Sync> Sync for Handle<'_, K, T> {}

impl<K: Key, T: ?Sized> Clone for Handle<'_, K, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Key, T: ?Sized> Copy for Handle<'_, K, T> {}

impl<'a, K: Key, T: ?Sized> Handle<'a, K, T> {
    /// Dereferences this stable handle for the lifetime of its pinned owning root.
    pub fn get(self) -> &'a T {
        // SAFETY: handles can only be created by a linked guard. `HandleTable::link` requires the
        // target to remain at a stable, valid address for `'a`, and this handle carries that borrow.
        unsafe { self.pointer.as_ref() }
    }
}
