//! The `_Mtx_*` mutexes (`_Mtx_lock.yaml`): the object layout the caller's inline code shares
//! with the DLL, and what each operation does to its owner and count.
//!
//! `_Mtx_internal_imp_t` (x64, 80 bytes, align 8): `int _Type` at 0; the critical section
//! storage from 8 (a slot kept at 8, the `SRWLOCK` at 16); `long _Thread_id` at 72 (-1 when
//! nobody owns it); `int _Count` at 76. A recursive mutex counts its owner's locks; the owner
//! is cleared only when the count reaches 0. `_Mtx_trylock` takes a free mutex; on one another
//! thread owns it returns `_Busy`, except that a mutex of type `_Mtx_plain` without `_Mtx_try`
//! or `_Mtx_timed` makes it BLOCK like `_Mtx_lock`. The owner's own trylock on a non-recursive
//! mutex is `_Busy` and changes nothing.

/// `_Mtx_plain`.
pub const MTX_PLAIN: i32 = 0x01;
/// `_Mtx_try`.
pub const MTX_TRY: i32 = 0x02;
/// `_Mtx_timed`.
pub const MTX_TIMED: i32 = 0x04;
/// `_Mtx_recursive`.
pub const MTX_RECURSIVE: i32 = 0x100;

/// `_Thrd_result`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ThrdResult {
    Success = 0,
    Nomem = 1,
    Timedout = 2,
    Busy = 3,
    Error = 4,
}

/// `_Thread_id` of a mutex nobody owns.
pub const NO_OWNER: i32 = -1;

/// Offsets in `_Mtx_internal_imp_t`.
pub const OFFSET_TYPE: usize = 0;
pub const OFFSET_SRWLOCK: usize = 16;
pub const OFFSET_THREAD_ID: usize = 72;
pub const OFFSET_COUNT: usize = 76;
/// `sizeof(_Mtx_internal_imp_t)`.
pub const SIZE: usize = 80;
/// `sizeof(_Cnd_internal_imp_t)`: a slot at 0, the `CONDITION_VARIABLE` at 8 (`_Cnd_wait.yaml`).
pub const CND_SIZE: usize = 72;
pub const CND_OFFSET_CV: usize = 8;

/// Whether a mutex of this type counts its owner's locks.
pub fn is_recursive(ty: i32) -> bool {
    ty & MTX_RECURSIVE != 0
}

/// Whether `_Mtx_trylock` waits for another owner (the sheet's `_Mtx_plain` rule).
pub fn trylock_blocks(ty: i32) -> bool {
    ty & MTX_PLAIN != 0 && ty & (MTX_TRY | MTX_TIMED) == 0
}

/// What a lock operation does, given the mutex's type and owner and the calling thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The caller owns a recursive mutex: count one more.
    Recurse,
    /// Return `_Busy` at once, changing nothing.
    Busy,
    /// Wait for the lock and take it (owner = caller, count 1).
    Wait,
    /// Take the lock if it is free, else `_Busy`.
    Try,
}

/// `_Mtx_lock`. The owner locking a non-recursive mutex again waits for itself (the C++
/// standard leaves that undefined; the sheet does not observe it).
pub fn lock_step(ty: i32, owner: i32, me: i32) -> Step {
    if owner == me && is_recursive(ty) { Step::Recurse } else { Step::Wait }
}

/// `_Mtx_trylock`.
pub fn trylock_step(ty: i32, owner: i32, me: i32) -> Step {
    if owner == me {
        return if is_recursive(ty) { Step::Recurse } else { Step::Busy };
    }
    if trylock_blocks(ty) { Step::Wait } else { Step::Try }
}

/// `_Mtx_unlock` with the count before it: the count after it and whether the lock is
/// released (the owner then is [`NO_OWNER`]). `None`: the mutex is not locked; nothing
/// changes and the result is `_Error`.
pub fn unlock_step(count: i32) -> Option<(i32, bool)> {
    if count <= 0 { None } else { Some((count - 1, count == 1)) }
}

// The layout's fields fit (checked when the crate compiles).
const _: () = assert!(OFFSET_TYPE == 0 && OFFSET_SRWLOCK + 8 <= OFFSET_THREAD_ID && OFFSET_COUNT + 4 == SIZE);
const _: () = assert!(CND_OFFSET_CV + 8 <= CND_SIZE);

#[cfg(test)]
mod tests {
    use super::*;

    /// A mutex as the sheet's tests see it: owner, count, and whether the lock is held.
    struct Model {
        ty: i32,
        owner: i32,
        count: i32,
    }

    impl Model {
        fn new(ty: i32) -> Model {
            Model { ty, owner: NO_OWNER, count: 0 }
        }
        /// `Some(result)`, or `None` where the real call would wait.
        fn apply(&mut self, step: Step, me: i32) -> Option<ThrdResult> {
            match step {
                Step::Recurse => {
                    self.count += 1;
                    Some(ThrdResult::Success)
                }
                Step::Busy => Some(ThrdResult::Busy),
                Step::Wait | Step::Try if self.owner == NO_OWNER => {
                    self.owner = me;
                    self.count = 1;
                    Some(ThrdResult::Success)
                }
                Step::Try => Some(ThrdResult::Busy),
                Step::Wait => None,
            }
        }
        fn lock(&mut self, me: i32) -> Option<ThrdResult> {
            self.apply(lock_step(self.ty, self.owner, me), me)
        }
        fn trylock(&mut self, me: i32) -> Option<ThrdResult> {
            self.apply(trylock_step(self.ty, self.owner, me), me)
        }
        fn unlock(&mut self) -> ThrdResult {
            match unlock_step(self.count) {
                Some((count, released)) => {
                    self.count = count;
                    if released {
                        self.owner = NO_OWNER;
                    }
                    ThrdResult::Success
                }
                None => ThrdResult::Error,
            }
        }
    }

    #[test]
    fn plain_lock_unlock() {
        let mut m = Model::new(MTX_PLAIN);
        assert_eq!(m.lock(7), Some(ThrdResult::Success));
        assert_eq!((m.owner, m.count), (7, 1));
        assert_eq!(m.unlock(), ThrdResult::Success);
        assert_eq!((m.owner, m.count), (NO_OWNER, 0));
        assert_eq!(m.trylock(7), Some(ThrdResult::Success), "trylock takes a free mutex");
        assert_eq!(m.count, 1);
        assert_eq!(m.unlock(), ThrdResult::Success);
        assert_eq!(m.unlock(), ThrdResult::Error, "not locked");
    }

    #[test]
    fn recursive_counts() {
        let mut m = Model::new(MTX_RECURSIVE | MTX_PLAIN);
        assert_eq!(m.lock(7), Some(ThrdResult::Success));
        assert_eq!(m.lock(7), Some(ThrdResult::Success));
        assert_eq!(m.trylock(7), Some(ThrdResult::Success));
        assert_eq!((m.owner, m.count), (7, 3));
        m.unlock();
        m.unlock();
        assert_eq!((m.owner, m.count), (7, 1), "the owner stays until 0");
        m.unlock();
        assert_eq!((m.owner, m.count), (NO_OWNER, 0));
    }

    #[test]
    fn trylock_kinds() {
        // Another thread (8) owns the mutex: busy at once, or a wait for the sheet's two types.
        let busy = [MTX_TRY, MTX_TIMED, MTX_PLAIN | MTX_TRY, MTX_PLAIN | MTX_TIMED, MTX_RECURSIVE | MTX_TRY, 0];
        for ty in busy {
            assert_eq!(trylock_step(ty, 8, 7), Step::Try, "type {ty:#x}");
            let mut m = Model { ty, owner: 8, count: 1 };
            assert_eq!(m.trylock(7), Some(ThrdResult::Busy));
        }
        for ty in [MTX_PLAIN, MTX_RECURSIVE | MTX_PLAIN] {
            assert_eq!(trylock_step(ty, 8, 7), Step::Wait, "type {ty:#x} blocks");
        }
    }

    #[test]
    fn trylock_by_the_owner() {
        let mut m = Model::new(MTX_PLAIN | MTX_TRY);
        m.lock(7);
        assert_eq!(m.trylock(7), Some(ThrdResult::Busy));
        assert_eq!((m.owner, m.count), (7, 1), "unchanged");
    }
}
