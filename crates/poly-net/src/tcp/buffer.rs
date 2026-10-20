//! Socket buffers: a fixed-capacity byte ring, and the assembler that
//! tracks out-of-order data written past the ring's contiguous end.

use alloc::vec;
use alloc::vec::Vec;

/// A ring of bytes with a fixed capacity, allocated once.
///
/// `len` bytes from `head` are the contiguous data (unread received data, or
/// unacknowledged and unsent data to send). Received data that arrives early
/// is written past that end with [`write_at`](RingBuf::write_at) and joined
/// with [`commit`](RingBuf::commit) once the gap fills.
#[derive(Debug)]
pub struct RingBuf {
    buf: Vec<u8>,
    head: usize,
    len: usize,
}

impl RingBuf {
    pub fn new(capacity: usize) -> RingBuf {
        RingBuf { buf: vec![0; capacity], head: 0, len: 0 }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Space after the contiguous data.
    pub fn free(&self) -> usize {
        self.buf.len() - self.len
    }

    /// Copy `data` into the ring starting `off` bytes after the contiguous
    /// end, without making it contiguous. Bytes that would not fit are
    /// ignored; returns how many were written.
    pub fn write_at(&mut self, off: usize, data: &[u8]) -> usize {
        let cap = self.buf.len();
        let room = self.free().saturating_sub(off);
        let n = data.len().min(room);
        if n == 0 {
            return 0;
        }
        let start = (self.head + self.len + off) % cap;
        let first = n.min(cap - start);
        self.buf[start..start + first].copy_from_slice(&data[..first]);
        self.buf[..n - first].copy_from_slice(&data[first..n]);
        n
    }

    /// Make `n` more bytes after the end contiguous (they were written with
    /// `write_at`).
    pub fn commit(&mut self, n: usize) {
        self.len = (self.len + n).min(self.buf.len());
    }

    /// Append as much of `data` as fits; returns how much did.
    pub fn push(&mut self, data: &[u8]) -> usize {
        let n = self.write_at(0, data);
        self.commit(n);
        n
    }

    /// Copy bytes starting `off` into the contiguous data into `out`,
    /// without consuming them; returns how many were copied.
    pub fn peek(&self, off: usize, out: &mut [u8]) -> usize {
        if off >= self.len {
            return 0;
        }
        let cap = self.buf.len();
        let n = out.len().min(self.len - off);
        let start = (self.head + off) % cap;
        let first = n.min(cap - start);
        out[..first].copy_from_slice(&self.buf[start..start + first]);
        out[first..n].copy_from_slice(&self.buf[..n - first]);
        n
    }

    /// Drop `n` bytes from the front.
    pub fn consume(&mut self, n: usize) {
        let n = n.min(self.len);
        if self.buf.is_empty() {
            return;
        }
        // The head never jumps back when the ring empties: data written past
        // the end (out of order) must keep its position.
        self.head = (self.head + n) % self.buf.len();
        self.len -= n;
    }

    /// Copy out and drop up to `out.len()` bytes from the front.
    pub fn pop(&mut self, out: &mut [u8]) -> usize {
        let n = self.peek(0, out);
        self.consume(n);
        n
    }
}

/// Byte ranges received beyond the contiguous end of a receive ring,
/// as offsets from that end. Sorted, disjoint, never touching.
#[derive(Debug)]
pub struct Assembler {
    ranges: Vec<(usize, usize)>,
    max_ranges: usize,
}

impl Assembler {
    pub fn new(max_ranges: usize) -> Assembler {
        Assembler { ranges: Vec::new(), max_ranges }
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Record `[off, off + len)` as received. Returns false (recording
    /// nothing) if it would need more ranges than allowed.
    pub fn add(&mut self, off: usize, len: usize) -> bool {
        if len == 0 {
            return true;
        }
        let (mut s, mut e) = (off, off + len);
        let mut out = Vec::with_capacity(self.ranges.len() + 1);
        let mut placed = false;
        for &(a, b) in &self.ranges {
            if b < s {
                out.push((a, b));
            } else if e < a {
                if !placed {
                    out.push((s, e));
                    placed = true;
                }
                out.push((a, b));
            } else {
                s = s.min(a);
                e = e.max(b);
            }
        }
        if !placed {
            out.push((s, e));
        }
        if out.len() > self.max_ranges {
            return false;
        }
        self.ranges = out;
        true
    }

    /// If a range starts at offset 0, remove it and return its length (the
    /// bytes that just became contiguous), shifting the rest down.
    pub fn take_front(&mut self) -> usize {
        match self.ranges.first() {
            Some(&(0, e)) => {
                self.ranges.remove(0);
                for r in &mut self.ranges {
                    r.0 -= e;
                    r.1 -= e;
                }
                e
            }
            _ => 0,
        }
    }

    /// Shift every range down by `n` after `n` bytes were appended in
    /// order; ranges this overlaps are trimmed.
    pub fn advance(&mut self, n: usize) {
        self.ranges.retain_mut(|r| {
            if r.1 <= n {
                return false;
            }
            r.0 = r.0.saturating_sub(n);
            r.1 -= n;
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_wraps() {
        let mut r = RingBuf::new(8);
        assert_eq!(r.push(b"abcdef"), 6);
        let mut out = [0u8; 4];
        assert_eq!(r.pop(&mut out), 4);
        assert_eq!(&out, b"abcd");
        assert_eq!(r.push(b"ghijklmn"), 6);
        assert_eq!(r.free(), 0);
        let mut all = [0u8; 8];
        assert_eq!(r.peek(0, &mut all), 8);
        assert_eq!(&all, b"efghijkl");
        assert_eq!(r.peek(3, &mut out), 4);
        assert_eq!(&out, b"hijk");
        assert_eq!(r.peek(8, &mut out), 0);
        r.consume(100);
        assert!(r.is_empty());
    }

    #[test]
    fn ring_out_of_order_writes() {
        let mut r = RingBuf::new(8);
        r.push(b"ab");
        r.consume(1);
        assert_eq!(r.write_at(3, b"xyz12345"), 4);
        assert_eq!(r.write_at(0, b"cde"), 3);
        r.commit(6);
        let mut out = [0u8; 8];
        assert_eq!(r.pop(&mut out), 7);
        assert_eq!(&out[..7], b"bcdexyz");
        assert_eq!(r.write_at(9, b"a"), 0);
    }

    #[test]
    fn emptying_keeps_data_written_past_the_end() {
        let mut r = RingBuf::new(8);
        r.push(b"abc");
        assert_eq!(r.write_at(2, b"xy"), 2);
        let mut out = [0u8; 3];
        r.pop(&mut out);
        assert!(r.is_empty());
        r.write_at(0, b"12");
        r.commit(4);
        let mut all = [0u8; 4];
        r.pop(&mut all);
        assert_eq!(&all, b"12xy");
    }

    #[test]
    fn assembler_merges() {
        let mut a = Assembler::new(3);
        assert!(a.add(10, 5));
        assert!(a.add(20, 5));
        assert!(a.add(30, 5));
        assert!(!a.add(40, 5), "a fourth range is refused");
        assert!(a.add(15, 5), "joins two ranges into one");
        assert_eq!(a.ranges, [(10, 25), (30, 35)]);
        assert!(a.add(5, 40));
        assert_eq!(a.ranges, [(5, 45)]);
        assert_eq!(a.take_front(), 0);
        a.advance(5);
        assert_eq!(a.ranges, [(0, 40)]);
        assert_eq!(a.take_front(), 40);
        assert!(a.is_empty());
    }

    #[test]
    fn assembler_advance_trims() {
        let mut a = Assembler::new(4);
        a.add(4, 4);
        a.add(10, 4);
        a.advance(6);
        assert_eq!(a.ranges, [(0, 2), (4, 8)]);
        assert_eq!(a.take_front(), 2);
        assert_eq!(a.ranges, [(2, 6)]);
    }
}
