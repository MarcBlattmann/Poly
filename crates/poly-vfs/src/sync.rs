//! The one lock the VFS uses inside itself.
//!
//! Every critical section behind a [`SpinMutex`] is short and stays in
//! memory: a table lookup, an offset update, a copy into or out of a tmpfs
//! page. None of them calls into a file system, a device or a callback, so
//! the lock is never held across anything that could sleep or fault. That is
//! what makes a plain spinlock correct here even with preemption: a holder
//! that is preempted is only ever waited for by spinners until it runs again.
//!
//! The one exception is the offset lock of an open regular file, directory
//! or seekable device (see `file.rs`), which is held across the call that
//! reads, writes or lists (including the `readdir` callback, which copies
//! entries out), because POSIX makes those offset updates atomic per open
//! file description. A file system that sleeps there (ext2 waiting for the
//! disk) makes other threads using the *same* open file description spin
//! meanwhile; nothing else waits on it. Lock order: offset lock, then any
//! lock inside a file system.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A test-and-test-and-set spinlock around a value.
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
}

impl<T: Default> Default for SpinMutex<T> {
    fn default() -> SpinMutex<T> {
        SpinMutex::new(T::default())
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for SpinMutex<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SpinMutex").field("locked", &self.locked.load(Ordering::Relaxed)).finish_non_exhaustive()
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
