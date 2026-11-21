//! A lock for `ws2_32`'s tables: a Windows SRW lock (Learn: "Slim Reader/Writer (SRW) Locks"), so a
//! thread that finds it taken sleeps instead of spinning. The tables are locked across system
//! calls (the event helper sets and resets event objects while it holds one, so a record and its
//! event change together); a spinning waiter would burn its CPU for as long as such a call takes,
//! and on a busy system that is as long as the holder waits for the CPU or for the kernel.

use core::cell::UnsafeCell;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn ReleaseSRWLockExclusive(lock: *mut usize);
}

/// A value behind an SRW lock. Not re-entrant: a closure given to [`Locked::with`] must not lock the
/// same value again.
pub struct Locked<T> {
    /// The `SRWLOCK`: `SRWLOCK_INIT` is zero (`synchapi.h`).
    srw: UnsafeCell<usize>,
    value: UnsafeCell<T>,
}

// SAFETY: the value is only reached with the lock held.
unsafe impl<T: Send> Sync for Locked<T> {}

impl<T> Locked<T> {
    pub const fn new(value: T) -> Locked<T> {
        Locked { srw: UnsafeCell::new(0), value: UnsafeCell::new(value) }
    }

    /// Run `f` with the value, the lock held.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let _held = self.hold();
        // SAFETY: the lock is held until `_held` is dropped, after `f` returned.
        f(unsafe { &mut *self.value.get() })
    }

    /// Take the lock until the guard is dropped (for a lock that guards an order, not a value).
    pub fn hold(&self) -> Held<'_, T> {
        // SAFETY: the lock word lives as long as `self`.
        unsafe { AcquireSRWLockExclusive(self.srw.get()) };
        Held(self)
    }
}

/// The lock of a [`Locked`], held.
pub struct Held<'a, T>(&'a Locked<T>);

impl<T> Drop for Held<'_, T> {
    fn drop(&mut self) {
        // SAFETY: taken by `hold`, released once.
        unsafe { ReleaseSRWLockExclusive(self.0.srw.get()) };
    }
}
