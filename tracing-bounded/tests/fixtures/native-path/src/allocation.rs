//! Thread-local allocation observer, used only by this audit executable.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<Counts> = const { Cell::new(Counts { allocations: 0, deallocations: 0 }) };
}

#[global_allocator]
static ALLOCATOR: Observer = Observer;

struct Observer;

fn record(allocations: u64, deallocations: u64) {
    if TRACKING.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNTS.try_with(|counts| {
            let previous = counts.get();
            // Never unwind from GlobalAlloc, even if reused beyond our finite
            // scenarios. Saturation is not a production counter implementation.
            counts.set(Counts {
                allocations: previous.allocations.saturating_add(allocations),
                deallocations: previous.deallocations.saturating_add(deallocations),
            });
        });
    }
}

// SAFETY: Every operation delegates unchanged layouts/pointers to System.
// Observer bookkeeping uses nonallocating, nonpanicking thread-local Cells.
unsafe impl GlobalAlloc for Observer {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies the valid layout, forwarded unchanged.
        let result = unsafe { System.alloc(layout) };
        record(1, 0);
        result
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies the valid layout, forwarded unchanged.
        let result = unsafe { System.alloc_zeroed(layout) };
        record(1, 0);
        result
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: Caller owns this live System allocation with this layout.
        unsafe { System.dealloc(pointer, layout) };
        record(0, 1);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: Caller supplies the live allocation and valid nonzero size;
        // the original layout, pointer and new size are forwarded unchanged.
        let result = unsafe { System.realloc(pointer, layout, new_size) };
        record(1, u64::from(!result.is_null()));
        result
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub allocations: u64,
    pub deallocations: u64,
}

pub fn prepare() {
    assert!(
        !TRACKING.with(Cell::get),
        "cannot prepare during a measurement"
    );
    COUNTS.with(|counts| counts.set(Counts::default()));
}

struct StopTracking;

impl Drop for StopTracking {
    fn drop(&mut self) {
        TRACKING.with(|tracking| tracking.set(false));
    }
}

pub fn measure(f: impl FnOnce()) -> Counts {
    assert!(!TRACKING.with(Cell::get), "nested measurement");
    COUNTS.with(|counts| counts.set(Counts::default()));
    TRACKING.with(|tracking| tracking.set(true));
    let guard = StopTracking;
    f();
    drop(guard);
    COUNTS.with(Cell::get)
}
