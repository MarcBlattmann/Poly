//! Interlocked singly linked lists (ADR 0087; Learn: "Interlocked Singly
//! Linked Lists", `InitializeSListHead`, `InterlockedPushEntrySList`,
//! `InterlockedPopEntrySList`, `InterlockedFlushSList`, `QueryDepthSList`).
//!
//! The x64 `SLIST_HEADER` of `winnt.h` is 16 bytes, 16-byte aligned: the
//! low quadword holds `Depth` (bits 0-15) and `Sequence` (bits 16-63), the
//! high one `NextEntry` (bits 4-63, the first entry's address with its four
//! low bits, always zero for a 16-byte aligned `SLIST_ENTRY`, cut off) and
//! four reserved bits. Every change replaces both quadwords at once with
//! `CMPXCHG16B` (in `kernelbase`); the sequence number grows with each
//! change, so a pop that raced with a pop and a push of the same entry
//! fails its exchange instead of linking a stale `Next` (the "ABA"
//! problem the sequence exists for). These are the pure parts: the
//! header's fields and the header after each operation.

/// A header's fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub depth: u16,
    /// 48 bits.
    pub sequence: u64,
    /// The first entry (16-byte aligned), or 0.
    pub next: u64,
}

const SEQUENCE_MASK: u64 = (1 << 48) - 1;

impl Header {
    /// The two quadwords (low, high) as `winnt.h` lays them out.
    pub fn pack(&self) -> (u64, u64) {
        (u64::from(self.depth) | (self.sequence & SEQUENCE_MASK) << 16, self.next & !0xF)
    }

    pub fn unpack(lo: u64, hi: u64) -> Header {
        Header { depth: lo as u16, sequence: lo >> 16, next: hi & !0xF }
    }

    /// After pushing `entry` (whose `Next` must be set to `self.next`
    /// before the exchange).
    pub fn pushed(&self, entry: u64) -> Header {
        Header {
            depth: self.depth.wrapping_add(1),
            sequence: self.sequence.wrapping_add(1) & SEQUENCE_MASK,
            next: entry,
        }
    }

    /// After popping the first entry, whose `Next` is `following`; `None`
    /// for an empty list.
    pub fn popped(&self, following: u64) -> Option<Header> {
        if self.next == 0 {
            return None;
        }
        Some(Header {
            depth: self.depth.wrapping_sub(1),
            sequence: self.sequence.wrapping_add(1) & SEQUENCE_MASK,
            next: following,
        })
    }

    /// After taking every entry (`InterlockedFlushSList`).
    pub fn flushed(&self) -> Header {
        Header { depth: 0, sequence: self.sequence.wrapping_add(1) & SEQUENCE_MASK, next: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_operations() {
        let empty = Header::default();
        assert_eq!(empty.pack(), (0, 0));
        let h = empty.pushed(0x7FF0_1230);
        assert_eq!(h.pack(), (1 | 1 << 16, 0x7FF0_1230));
        assert_eq!(Header::unpack(1 | 1 << 16, 0x7FF0_123F), h, "reserved bits ignored");
        let h2 = h.pushed(0x7FF0_4560);
        assert_eq!((h2.depth, h2.sequence, h2.next), (2, 2, 0x7FF0_4560));
        let p = h2.popped(0x7FF0_1230).unwrap();
        assert_eq!((p.depth, p.sequence, p.next), (1, 3, 0x7FF0_1230));
        assert_eq!(empty.popped(0), None);
        assert_eq!(h2.flushed(), Header { depth: 0, sequence: 3, next: 0 });
        // The sequence wraps in its 48 bits.
        let top = Header { depth: 0, sequence: SEQUENCE_MASK, next: 0 };
        assert_eq!(top.pushed(0x10).sequence, 0);
        assert_eq!(top.pushed(0x10).pack().0, 1);
    }
}
