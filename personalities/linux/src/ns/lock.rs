//! The one lock of the namespace objects: a spinlock around the few
//! things in them that change after creation (a pid namespace's table,
//! a UTS namespace's names, an id map being written, the registry).
//!
//! Every critical section is a table lookup or update in memory: nothing
//! behind it blocks, faults, or calls back into the kernel, and no code
//! holds two of these locks at once except a pid namespace walk, which
//! always locks from the innermost namespace outwards and only one table
//! at a time. A plain spinlock is therefore enough, as for the futex
//! table (ADR 0031).

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A test-and-test-and-set spinlock around a value.
pub struct Mutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is only reached through a guard, and only one guard
// exists at a time (`lock` takes the flag with acquire ordering and the
// guard's drop releases it), so sharing the mutex hands the value from one
// thread to the next, which `T: Send` allows.
unsafe impl<T: Send> Sync for Mutex<T> {}
// SAFETY: moving the mutex moves the value; `T: Send` allows that.
unsafe impl<T: Send> Send for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Mutex<T> {
        Mutex { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    pub fn lock(&self) -> Guard<'_, T> {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        Guard { mutex: self }
    }
}

impl<T> core::fmt::Debug for Mutex<T> {
    /// Without the value: printing must not wait for the lock.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mutex").field("locked", &self.locked.load(Ordering::Relaxed)).finish_non_exhaustive()
    }
}

/// Access to the value while the lock is held.
pub struct Guard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> core::fmt::Debug for Guard<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Guard").finish_non_exhaustive()
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard exists only while this thread holds the lock,
        // so no other reference to the value exists.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as in `deref`; `&mut self` makes this the only reference
        // through the guard.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}
