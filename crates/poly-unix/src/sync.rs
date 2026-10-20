//! The lock of this crate: a spinlock around each socket's state and the
//! namespace's tables. Critical sections are short and never call out (no
//! waker runs and no open file is dropped with a lock held: both happen
//! after the guard is gone), as in `poly-vfs`.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct SpinMutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is only reached through a guard and only one guard
// exists at a time (`lock` spins until it owns the flag), so sharing the
// mutex hands the value between threads, which `T: Send` allows.
unsafe impl<T: Send> Sync for SpinMutex<T> {}

impl<T> SpinMutex<T> {
    pub(crate) const fn new(value: T) -> SpinMutex<T> {
        SpinMutex { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    /// The value, through exclusive access to the mutex (no locking).
    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
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

pub(crate) struct SpinGuard<'a, T> {
    mutex: &'a SpinMutex<T>,
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard owns the lock, so no other reference exists.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as in `deref`, and `&mut self` makes this the only one.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}
