//! The lock around a mounted file system's state.
//!
//! A test-and-test-and-set spinlock, like the VFS's own. Unlike the VFS's
//! critical sections, ext2's are held across disk I/O (one lock per mounted
//! file system keeps every operation atomic, which is what the `FileSystem`
//! contract asks for). That is correct with a preemptive scheduler: a thread
//! spinning here is preempted by the tick, and the holder, sleeping in the
//! block layer, runs again when its I/O is done. It wastes the spinner's
//! time slice under contention; see ADR 0032 for when that changes.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct SpinMutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is only reached through a guard, and only one guard can
// exist at a time (see `lock`), so sharing the mutex between threads hands
// the value from one thread to another, which `T: Send` allows.
unsafe impl<T: Send> Sync for SpinMutex<T> {}

impl<T> SpinMutex<T> {
    pub(crate) const fn new(value: T) -> SpinMutex<T> {
        SpinMutex { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    pub(crate) fn lock(&self) -> SpinGuard<'_, T> {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        SpinGuard { mutex: self }
    }

    pub(crate) fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

pub(crate) struct SpinGuard<'a, T> {
    mutex: &'a SpinMutex<T>,
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: this guard is the only one (the compare-exchange in `lock`
        // admits one holder), so nothing else reaches the value while it lives.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as in `deref`, and `&mut self` makes this the only borrow
        // through the guard.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}
