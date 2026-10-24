//! Per-thread Rust allocation requests for separate diagnostic builds only.
//! Requested bytes are not retained heap size, managed allocations or copied
//! bytes. Transfers/deallocations on another thread do not affect this counter.
#[derive(Clone, Copy, Default)]
pub struct Snapshot {
    pub allocations: u64,
    pub requested_bytes: u64,
    pub reallocations: u64,
}

impl Snapshot {
    pub fn since(self, before: Self) -> Self {
        Self {
            allocations: self.allocations.saturating_sub(before.allocations),
            requested_bytes: self.requested_bytes.saturating_sub(before.requested_bytes),
            reallocations: self.reallocations.saturating_sub(before.reallocations),
        }
    }
}

#[cfg(feature = "allocation-diagnostics")]
mod enabled {
    use super::Snapshot;
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };
    thread_local! {
        // No allocation, locks, formatting or lazy heap initialization inside
        // the allocator. try_with also tolerates thread-local destruction.
        static COUNTS: Cell<Snapshot> = const { Cell::new(Snapshot {
            allocations: 0, requested_bytes: 0, reallocations: 0,
        }) };
    }
    fn record(bytes: usize, reallocation: bool) {
        let _ = COUNTS.try_with(|cell| {
            let mut counts = cell.get();
            counts.allocations = counts.allocations.saturating_add(1);
            counts.requested_bytes = counts.requested_bytes.saturating_add(bytes as u64);
            counts.reallocations = counts.reallocations.saturating_add(u64::from(reallocation));
            cell.set(counts);
        });
    }
    struct Counting;
    // SAFETY: all allocation, alignment and deallocation operations delegate
    // unchanged to System. Successful requests update numeric TLS only.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { System.alloc(layout) };
            if !pointer.is_null() {
                record(layout.size(), false);
            }
            pointer
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { System.alloc_zeroed(layout) };
            if !pointer.is_null() {
                record(layout.size(), false);
            }
            pointer
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) };
        }
        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let result = unsafe { System.realloc(pointer, layout, size) };
            if !result.is_null() {
                record(size, true);
            }
            result
        }
    }
    #[global_allocator]
    static ALLOCATOR: Counting = Counting;
    pub fn snapshot() -> Snapshot {
        COUNTS.try_with(Cell::get).unwrap_or_default()
    }
}

#[inline]
pub fn snapshot() -> Snapshot {
    #[cfg(feature = "allocation-diagnostics")]
    {
        enabled::snapshot()
    }
    #[cfg(not(feature = "allocation-diagnostics"))]
    {
        Snapshot::default()
    }
}

#[cfg(all(test, feature = "allocation-diagnostics"))]
mod tests {
    #[test]
    fn successful_requests_are_attributed_to_the_allocating_thread() {
        std::thread::spawn(|| {
            let before = super::snapshot();
            let layout = std::alloc::Layout::from_size_align(64, 16).unwrap();
            unsafe {
                let pointer = std::alloc::alloc_zeroed(layout);
                assert!(!pointer.is_null());
                let moved = std::alloc::realloc(pointer, layout, 128);
                assert!(!moved.is_null());
                std::alloc::dealloc(moved, std::alloc::Layout::from_size_align(128, 16).unwrap());
            }
            let delta = super::snapshot().since(before);
            assert_eq!(delta.allocations, 2);
            assert_eq!(delta.requested_bytes, 192);
            assert_eq!(delta.reallocations, 1);
        })
        .join()
        .unwrap();
    }
}
