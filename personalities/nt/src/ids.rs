//! Process and thread ids as Windows hands them out: non-zero multiples of 4 (the sheet of
//! `GetThreadId`, rule `created_thread`), from one counter the kernel shares with the other
//! personalities' ids, so no two live processes or threads ever have the same id.

use core::sync::atomic::{AtomicU32, Ordering};

/// The lowest id an NT process or thread gets: real ids are never small (a program may use a small
/// number such as 4 as "no such thread", as the visible tests of `GetThreadDesktop` do).
pub const FIRST_ID: u32 = 0x400;

/// The next id at or above `counter`'s value (and [`FIRST_ID`]) that is a multiple of 4; `counter`
/// moves past it (the ids in between are never handed out by this call, and the others' ids never
/// collide with the one returned).
pub fn alloc_multiple_of_4(counter: &AtomicU32) -> u32 {
    let mut cur = counter.load(Ordering::Relaxed);
    loop {
        let id = cur.max(FIRST_ID).wrapping_add(3) & !3;
        match counter.compare_exchange_weak(cur, id + 1, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return id,
            Err(now) => cur = now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_multiples_of_four_not_small_and_never_repeat() {
        let counter = AtomicU32::new(1);
        let a = alloc_multiple_of_4(&counter);
        let b = alloc_multiple_of_4(&counter);
        assert_eq!((a, b), (FIRST_ID, FIRST_ID + 4));
        // The ids another allocator takes from the same counter lie between ours.
        assert_eq!(counter.fetch_add(1, Ordering::Relaxed), FIRST_ID + 5);
        assert_eq!(counter.fetch_add(1, Ordering::Relaxed), FIRST_ID + 6);
        let c = alloc_multiple_of_4(&counter);
        assert_eq!(c, FIRST_ID + 8);
        assert!(a.is_multiple_of(4) && b.is_multiple_of(4) && c.is_multiple_of(4) && a > 4);
    }

    #[test]
    fn a_counter_past_the_floor_gives_the_next_multiple() {
        let counter = AtomicU32::new(0x1001);
        assert_eq!(alloc_multiple_of_4(&counter), 0x1004);
        assert_eq!(counter.load(Ordering::Relaxed), 0x1005);
        let counter = AtomicU32::new(0x1004);
        assert_eq!(alloc_multiple_of_4(&counter), 0x1004);
        assert_eq!(alloc_multiple_of_4(&AtomicU32::new(0)), FIRST_ID);
    }
}
