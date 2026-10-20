//! A fixed-size byte queue for one producer and one consumer, without locks:
//! an interrupt handler pushes the bytes a device delivers, a thread pops
//! them.

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

/// Bytes in arrival order, at most `N`. A push to a full ring drops the
/// byte and counts it.
///
/// `push` must only be called by one producer at a time and `pop` by one
/// consumer at a time (an interrupt handler on one CPU and a thread holding
/// a lock, say). Breaking that cannot cause undefined behaviour, only lost
/// or repeated bytes: everything is an atomic.
#[derive(Debug)]
pub struct ByteRing<const N: usize> {
    buf: [AtomicU8; N],
    /// Bytes ever pushed; the next one goes to `head % N`.
    head: AtomicUsize,
    /// Bytes ever popped.
    tail: AtomicUsize,
    dropped: AtomicUsize,
}

impl<const N: usize> Default for ByteRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> ByteRing<N> {
    pub const fn new() -> Self {
        assert!(N > 0, "a ring needs room for a byte");
        ByteRing {
            buf: [const { AtomicU8::new(0) }; N],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
        }
    }

    /// Append a byte; false (and the byte counted as dropped) if the ring
    /// is full.
    pub fn push(&self, byte: u8) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        // Acquire: the consumer has finished reading the slot it released.
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        self.buf[head % N].store(byte, Ordering::Relaxed);
        // Release: publishes the byte before the new head.
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// The oldest byte, if any.
    pub fn pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        let byte = self.buf[tail % N].load(Ordering::Relaxed);
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(byte)
    }

    /// Bytes waiting.
    pub fn len(&self) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        self.head.load(Ordering::Acquire).wrapping_sub(tail)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes ever pushed (dropped ones not included).
    pub fn pushed(&self) -> usize {
        self.head.load(Ordering::Relaxed)
    }

    /// Bytes dropped because the ring was full.
    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_drops_when_full() {
        let r = ByteRing::<4>::new();
        assert_eq!(r.pop(), None);
        for b in 1..=4 {
            assert!(r.push(b));
        }
        assert!(!r.push(5), "full");
        assert_eq!((r.len(), r.dropped(), r.pushed()), (4, 1, 4));
        assert_eq!(r.pop(), Some(1));
        assert!(r.push(6));
        let out: std::vec::Vec<u8> = core::iter::from_fn(|| r.pop()).collect();
        assert_eq!(out, [2, 3, 4, 6]);
        assert!(r.is_empty());
    }

    #[test]
    fn wraps_around_many_times() {
        let r = ByteRing::<3>::new();
        let mut expected = 0u8;
        for i in 0..1000u32 {
            assert!(r.push(i as u8));
            if i % 2 == 1 {
                for _ in 0..2 {
                    assert_eq!(r.pop(), Some(expected));
                    expected = expected.wrapping_add(1);
                }
            }
        }
        assert!(r.is_empty());
        assert_eq!(r.dropped(), 0);
    }

    #[test]
    fn one_producer_thread_and_one_consumer_thread() {
        let r = std::sync::Arc::new(ByteRing::<16>::new());
        let producer = {
            let r = std::sync::Arc::clone(&r);
            std::thread::spawn(move || {
                for i in 0..100_000u32 {
                    while !r.push(i as u8) {
                        std::thread::yield_now();
                    }
                }
            })
        };
        let mut next = 0u8;
        let mut received = 0;
        while received < 100_000 {
            if let Some(b) = r.pop() {
                assert_eq!(b, next);
                next = next.wrapping_add(1);
                received += 1;
            } else {
                std::thread::yield_now();
            }
        }
        producer.join().unwrap();
        assert!(r.is_empty());
    }
}
