//! Process and thread ids as Windows hands them out: non-zero multiples of 4 (the sheet of
//! `GetThreadId`, rule `created_thread`), from one counter the kernel shares with the other
//! personalities' ids, so no two live processes or threads ever have the same id.

use core::sync::atomic::{AtomicU32, Ordering};

/// The next id at or above `counter`'s value that is a multiple of 4; `counter` moves past it
/// (the ids in between are never handed out by this call, and the others' ids never collide
/// with the one returned).
pub fn alloc_multiple_of_4(counter: &AtomicU32) -> u32 {
    let mut cur = counter.load(Ordering::Relaxed);
    loop {
        let id = cur.max(1).wrapping_add(3) & !3;
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
    fn ids_are_non_zero_multiples_of_four_and_never_repeat() {
        let counter = AtomicU32::new(1);
        let a = alloc_multiple_of_4(&counter);
        let b = alloc_multiple_of_4(&counter);
        assert_eq!((a, b), (4, 8));
        // The ids another allocator takes from the same counter lie between ours.
        assert_eq!(counter.fetch_add(1, Ordering::Relaxed), 9);
        assert_eq!(counter.fetch_add(1, Ordering::Relaxed), 10);
        let c = alloc_multiple_of_4(&counter);
        assert_eq!(c, 12);
        assert!(a % 4 == 0 && b % 4 == 0 && c % 4 == 0 && a != 0);
    }

    #[test]
    fn a_counter_that_stands_on_a_multiple_gives_that_one() {
        let counter = AtomicU32::new(16);
        assert_eq!(alloc_multiple_of_4(&counter), 16);
        assert_eq!(counter.load(Ordering::Relaxed), 17);
        assert_eq!(alloc_multiple_of_4(&AtomicU32::new(0)), 4);
    }
}
