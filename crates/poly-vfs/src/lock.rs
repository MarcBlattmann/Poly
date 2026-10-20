//! A lock the user of a file system crate chooses.
//!
//! File systems below the VFS hold their locks across disk I/O (ext2's
//! volume lock, the page cache's per-inode locks). On the host a spinlock
//! is fine for that; in the kernel a thread that finds such a lock held
//! should sleep instead of burning its time slice while the holder waits
//! for the disk. So these crates take the lock as a type parameter: a
//! [`RawLock`] (lock and unlock, no data), wrapped around the data by
//! [`Lock`]. [`RawSpinLock`] is the default; the kernel passes its sleeping
//! mutex.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A lock without data.
///
/// # Safety
///
/// Between a `lock` that returned and the matching `unlock`, no other call
/// to `lock` may return (mutual exclusion), and `unlock` must make the
/// holder's writes visible to the next holder (release / acquire ordering).
pub unsafe trait RawLock: Send + Sync {
    /// Blocks (spinning or sleeping) until the lock is held.
    fn lock(&self);

    /// # Safety
    ///
    /// The calling thread holds the lock (it called `lock`, and has not
    /// called `unlock` since).
    unsafe fn unlock(&self);
}

/// A test-and-test-and-set spinlock: the default [`RawLock`].
#[derive(Debug, Default)]
pub struct RawSpinLock {
    locked: AtomicBool,
}

impl RawSpinLock {
    pub const fn new() -> RawSpinLock {
        RawSpinLock { locked: AtomicBool::new(false) }
    }
}

// SAFETY: only the caller whose compare-exchange moves `locked` from false
// to true returns from `lock` (Acquire), and only `unlock` clears it
// (Release), so holders are exclusive and see each other's writes.
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

/// A value behind a [`RawLock`] of type `L`.
pub struct Lock<L, T: ?Sized> {
    raw: L,
    value: UnsafeCell<T>,
}

// SAFETY: the value is only reached through a guard, and `L` admits one
// holder at a time, so sharing the lock hands the value from thread to
// thread, which `T: Send` allows.
unsafe impl<L: RawLock, T: ?Sized + Send> Sync for Lock<L, T> {}
// SAFETY: as for any container owning a `T` and an `L`.
unsafe impl<L: Send, T: ?Sized + Send> Send for Lock<L, T> {}

impl<L: RawLock + Default, T> Lock<L, T> {
    pub fn new(value: T) -> Lock<L, T> {
        Lock { raw: L::default(), value: UnsafeCell::new(value) }
    }
}

impl<L, T> Lock<L, T> {
    /// With a lock made by the caller (one that has no `Default`).
    pub const fn with_raw(raw: L, value: T) -> Lock<L, T> {
        Lock { raw, value: UnsafeCell::new(value) }
    }

    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<L: RawLock, T: ?Sized> Lock<L, T> {
    /// Takes the lock; it is released when the guard is dropped.
    pub fn lock(&self) -> LockGuard<'_, L, T> {
        self.raw.lock();
        LockGuard { lock: self }
    }

    /// The value, without locking: `&mut self` proves nobody else holds it.
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
}

impl<L: RawLock + Default, T: Default> Default for Lock<L, T> {
    fn default() -> Lock<L, T> {
        Lock::new(T::default())
    }
}

impl<L, T: ?Sized> core::fmt::Debug for Lock<L, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lock").finish_non_exhaustive()
    }
}

/// Exclusive access to a [`Lock`]'s value; unlocks when dropped.
pub struct LockGuard<'a, L: RawLock, T: ?Sized> {
    lock: &'a Lock<L, T>,
}

impl<L: RawLock, T: ?Sized> Deref for LockGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard exists only while its thread holds the raw lock,
        // which admits one holder.
        unsafe { &*self.lock.value.get() }
    }
}

impl<L: RawLock, T: ?Sized> DerefMut for LockGuard<'_, L, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as in `deref`; `&mut self` makes this the only borrow
        // through the guard.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<L: RawLock, T: ?Sized> Drop for LockGuard<'_, L, T> {
    fn drop(&mut self) {
        // SAFETY: this guard was made by `lock`, so this thread holds it.
        unsafe { self.lock.raw.unlock() }
    }
}

impl<L: RawLock, T: ?Sized> core::fmt::Debug for LockGuard<'_, L, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LockGuard").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use alloc::vec::Vec;

    extern crate std;

    #[test]
    fn spin_lock_serialises_increments() {
        let lock: Arc<Lock<RawSpinLock, u64>> = Arc::new(Lock::new(0));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let lock = Arc::clone(&lock);
                std::thread::spawn(move || {
                    for _ in 0..10_000 {
                        *lock.lock() += 1;
                    }
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert_eq!(*lock.lock(), 40_000);
    }

    #[test]
    fn guard_releases_on_drop() {
        let mut lock: Lock<RawSpinLock, Vec<u8>> = Lock::new(Vec::new());
        lock.lock().push(1);
        lock.lock().push(2);
        assert_eq!(lock.get_mut(), &[1, 2]);
        assert_eq!(lock.into_inner(), [1, 2]);
    }
}
