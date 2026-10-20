//! A lock around [`Heap`] so it can be the `#[global_allocator]`.
//!
//! The lock is a type parameter because the kernel will want one that also
//! masks interrupts once an interrupt handler can allocate; until then
//! [`RawSpinLock`] does.

use crate::{Heap, PageSource};
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A lock with no data attached.
///
/// # Safety
///
/// Between a `lock` that returned and the matching `unlock`, no other call
/// to `lock` may return.
pub unsafe trait RawLock {
    fn lock(&self);

    /// # Safety
    ///
    /// The caller holds the lock.
    unsafe fn unlock(&self);
}

/// Test-and-test-and-set spinlock.
#[derive(Debug, Default)]
pub struct RawSpinLock {
    locked: AtomicBool,
}

impl RawSpinLock {
    pub const fn new() -> RawSpinLock {
        RawSpinLock { locked: AtomicBool::new(false) }
    }
}

// SAFETY: only the caller whose compare-exchange flips `locked` from false
// to true returns from `lock`, and `unlock` is the only thing that clears it.
unsafe impl RawLock for RawSpinLock {
    fn lock(&self) {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
    }

    unsafe fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

/// A [`Heap`] behind a lock, usable from a `static`:
///
/// ```ignore
/// #[global_allocator]
/// static HEAP: LockedHeap<KernelPages> = LockedHeap::new(KernelPages);
/// ```
pub struct LockedHeap<S, L = RawSpinLock> {
    lock: L,
    heap: UnsafeCell<Heap<S>>,
}

// SAFETY: the heap is only reached through `lock()`, which serialises access
// with `L`, and the heap itself may move between threads.
unsafe impl<S: Send, L: RawLock + Sync> Sync for LockedHeap<S, L> {}

impl<S, L> core::fmt::Debug for LockedHeap<S, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LockedHeap").finish_non_exhaustive()
    }
}

impl<S> LockedHeap<S> {
    pub const fn new(source: S) -> LockedHeap<S> {
        LockedHeap::with_lock(RawSpinLock::new(), source)
    }
}

impl<S, L> LockedHeap<S, L> {
    pub const fn with_lock(lock: L, source: S) -> LockedHeap<S, L> {
        LockedHeap { lock, heap: UnsafeCell::new(Heap::new(source)) }
    }
}

impl<S, L: RawLock> LockedHeap<S, L> {
    /// Lock the heap, e.g. to read its statistics or trim it. Allocating
    /// through the global allocator while holding the guard deadlocks.
    pub fn lock(&self) -> HeapGuard<'_, S, L> {
        self.lock.lock();
        HeapGuard { owner: self }
    }
}

/// Exclusive access to a [`LockedHeap`]'s heap; unlocks on drop.
pub struct HeapGuard<'a, S, L: RawLock> {
    owner: &'a LockedHeap<S, L>,
}

impl<S, L: RawLock> core::fmt::Debug for HeapGuard<'_, S, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("HeapGuard").field(&**self).finish()
    }
}

impl<S, L: RawLock> Deref for HeapGuard<'_, S, L> {
    type Target = Heap<S>;
    fn deref(&self) -> &Heap<S> {
        // SAFETY: the guard proves we hold the lock.
        unsafe { &*self.owner.heap.get() }
    }
}

impl<S, L: RawLock> DerefMut for HeapGuard<'_, S, L> {
    fn deref_mut(&mut self) -> &mut Heap<S> {
        // SAFETY: the guard proves we hold the lock, so nobody else has a
        // reference to the heap.
        unsafe { &mut *self.owner.heap.get() }
    }
}

impl<S, L: RawLock> Drop for HeapGuard<'_, S, L> {
    fn drop(&mut self) {
        // SAFETY: the guard exists only while the lock is held.
        unsafe { self.owner.lock.unlock() };
    }
}

// SAFETY: `Heap` returns null or memory valid for the layout that no live
// allocation overlaps, and the lock serialises every call.
unsafe impl<S: PageSource, L: RawLock> GlobalAlloc for LockedHeap<S, L> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: GlobalAlloc's caller contract is the heap's.
        unsafe { self.lock().dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: GlobalAlloc's caller contract is the heap's.
        unsafe { self.lock().realloc(ptr, layout, new_size) }
    }
}
