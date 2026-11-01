//! A serial port's receive ring with flow control (ADR 0123).
//!
//! The interrupt handler moves what the UART received into the ring; a
//! reader (the shell, or the terminal on the port) takes it out. When the
//! ring is full the handler stops reading the UART instead of dropping
//! bytes: what is still in the UART's FIFO stays there, the UART stops
//! taking more, and the other end (QEMU's character device, or a sender
//! honouring hardware flow control) holds its data back. The handler
//! switches the receive interrupt off and marks the ring throttled; once
//! the reader has taken it down to half full it is told to resume
//! ([`RxRing::take_resume`]): it drains the UART again and switches the
//! interrupt back on.
//!
//! One producer (the handler, or code holding the receiver's lock with
//! interrupts off) and one consumer at a time (the reader, which takes its
//! own lock): lock-free between the two. `head` counts bytes ever written,
//! `tail` bytes ever read; both only grow (wrapping), and `head - tail` is
//! the fill level.

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

/// The ring: `N` bytes.
#[derive(Debug)]
pub struct RxRing<const N: usize> {
    buf: [AtomicU8; N],
    head: AtomicUsize,
    tail: AtomicUsize,
    /// The producer stopped reading the source because the ring was full.
    throttled: AtomicBool,
}

/// What [`RxRing::fill`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fill {
    /// Bytes moved into the ring.
    pub moved: usize,
    /// It stopped because the ring was full: the source may hold more, and
    /// the ring is throttled now (stop the source's interrupt).
    pub full: bool,
}

impl<const N: usize> Default for RxRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RxRing<N> {
    /// Resume once the reader took the ring down to this fill level.
    pub const RESUME_AT: usize = N / 2;

    pub const fn new() -> Self {
        RxRing {
            buf: [const { AtomicU8::new(0) }; N],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            throttled: AtomicBool::new(false),
        }
    }

    /// Bytes in the ring.
    pub fn len(&self) -> usize {
        self.head.load(Ordering::Acquire).wrapping_sub(self.tail.load(Ordering::Acquire))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Producer: store `byte` if there is room; `false` (the byte not
    /// taken) if the ring is full.
    pub fn push(&self, byte: u8) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        if head.wrapping_sub(self.tail.load(Ordering::Acquire)) >= N {
            return false;
        }
        self.buf[head % N].store(byte, Ordering::Relaxed);
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Producer: move bytes from `next` (the UART: `None` when it holds no
    /// more) into the ring, at most `limit`. `next` is only called while
    /// there is room, so a full ring leaves the rest where it is; the ring
    /// is then marked throttled.
    pub fn fill(&self, mut next: impl FnMut() -> Option<u8>, limit: usize) -> Fill {
        let mut moved = 0;
        while moved < limit {
            if !self.has_room() {
                // Marked before looking again: a reader that empties the
                // ring meanwhile sees the mark (`take_resume`), or this
                // second look sees the room.
                self.throttled.store(true, Ordering::SeqCst);
                if !self.has_room() {
                    return Fill { moved, full: true };
                }
                self.throttled.store(false, Ordering::SeqCst);
            }
            let Some(b) = next() else { break };
            let stored = self.push(b);
            debug_assert!(stored, "room was checked");
            moved += 1;
        }
        Fill { moved, full: false }
    }

    fn has_room(&self) -> bool {
        self.head.load(Ordering::Relaxed).wrapping_sub(self.tail.load(Ordering::SeqCst)) < N
    }

    /// Consumer: the oldest byte.
    pub fn pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        let byte = self.buf[tail % N].load(Ordering::Relaxed);
        self.tail.store(tail.wrapping_add(1), Ordering::SeqCst);
        Some(byte)
    }

    /// Whether the producer stopped because the ring was full.
    pub fn is_throttled(&self) -> bool {
        self.throttled.load(Ordering::SeqCst)
    }

    /// Consumer, after taking bytes and before waiting for more: `true`
    /// once when the ring was throttled and is down to [`Self::RESUME_AT`]
    /// (the mark is cleared): the caller drains the source again and
    /// switches its interrupt back on.
    pub fn take_resume(&self) -> bool {
        self.len() <= Self::RESUME_AT
            && self.throttled.compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A UART's FIFO: bytes waiting to be read.
    struct Source(Vec<u8>);

    impl Source {
        fn next(&mut self) -> Option<u8> {
            (!self.0.is_empty()).then(|| self.0.remove(0))
        }
    }

    #[test]
    fn keeps_order_and_refuses_bytes_when_full() {
        let r = RxRing::<8>::new();
        for i in 0..8 {
            assert!(r.push(i));
        }
        assert!(!r.push(8), "a full ring takes no more");
        assert_eq!(r.len(), 8);
        assert_eq!((0..8).map(|_| r.pop().unwrap()).collect::<Vec<_>>(), (0..8).collect::<Vec<_>>());
        assert_eq!(r.pop(), None);
        assert!(r.is_empty());
    }

    #[test]
    fn a_full_ring_leaves_the_rest_in_the_source_and_throttles() {
        let r = RxRing::<8>::new();
        let mut src = Source((0..20).collect());
        let f = r.fill(|| src.next(), 64);
        assert_eq!(f, Fill { moved: 8, full: true });
        assert!(r.is_throttled());
        assert_eq!(src.0, (8..20).collect::<Vec<_>>(), "nothing read past the room, nothing dropped");
        // Not until half of it was read.
        for i in 0..3 {
            assert_eq!(r.pop(), Some(i));
            assert!(!r.take_resume());
        }
        assert_eq!(r.pop(), Some(3));
        assert!(r.take_resume(), "down to half: resume");
        assert!(!r.take_resume(), "told once");
        assert!(!r.is_throttled());
        let f = r.fill(|| src.next(), 64);
        assert_eq!(f, Fill { moved: 4, full: true });
    }

    #[test]
    fn an_empty_source_is_not_full() {
        let r = RxRing::<8>::new();
        let mut src = Source(alloc::vec![1, 2, 3]);
        assert_eq!(r.fill(|| src.next(), 64), Fill { moved: 3, full: false });
        assert!(!r.is_throttled());
        // The limit stops it too, without throttling.
        let mut src = Source((0..5).collect());
        assert_eq!(r.fill(|| src.next(), 2), Fill { moved: 2, full: false });
        assert!(!r.is_throttled());
    }

    /// A long line through a small ring, with the reader slower than the
    /// sender: every byte arrives, in order (the scenario of ADR 0123).
    #[test]
    fn every_byte_of_a_long_line_arrives() {
        let r = RxRing::<16>::new();
        let line: Vec<u8> = (0..1000u32).map(|i| b'a' + (i % 26) as u8).collect();
        let mut src = Source(line.clone());
        let mut got = Vec::new();
        let mut interrupt_on = true;
        let mut steps = 0;
        while got.len() < line.len() {
            steps += 1;
            assert!(steps < 100_000, "stuck");
            // The sender (QEMU) fills the UART's FIFO, which raises the
            // interrupt while it is on.
            if interrupt_on && !src.0.is_empty() {
                let f = r.fill(|| src.next(), 7);
                if f.full {
                    interrupt_on = false;
                }
            }
            // The reader takes a few bytes, and resumes the source.
            for _ in 0..3 {
                if let Some(b) = r.pop() {
                    got.push(b);
                }
            }
            if r.take_resume() {
                let f = r.fill(|| src.next(), 7);
                interrupt_on = !f.full;
            }
        }
        assert_eq!(got, line);
    }

    /// Producer and consumer on two threads.
    #[test]
    fn concurrent_producer_and_consumer() {
        use std::sync::Arc;
        let r = Arc::new(RxRing::<32>::new());
        let total = 20_000usize;
        let producer = {
            let r = Arc::clone(&r);
            std::thread::spawn(move || {
                let mut next = 0usize;
                while next < total {
                    let limit = (total - next).min(13);
                    let f = r.fill(
                        || {
                            let b = (next % 251) as u8;
                            next += 1;
                            Some(b)
                        },
                        limit,
                    );
                    if f.full {
                        // Interrupt off: wait to be resumed.
                        while r.is_throttled() {
                            std::thread::yield_now();
                        }
                    }
                }
            })
        };
        let mut got = 0usize;
        while got < total {
            if let Some(b) = r.pop() {
                assert_eq!(b, (got % 251) as u8);
                got += 1;
            } else {
                std::thread::yield_now();
            }
            r.take_resume();
        }
        producer.join().unwrap();
    }
}
