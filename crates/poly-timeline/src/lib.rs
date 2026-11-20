//! The timeline (`poly.debug.timeline`, ADR 0247): what every CPU switched to, who woke whom,
//! which system calls ran, when keys came in and frames went out, as fixed-size records in one
//! ring of guest memory. The kernel writes it lock-free from any context; the host copies it out
//! of the guest's memory (QEMU's `pmemsave`) and `tools/timeline/timeline.py` turns it into
//! per-thread lanes. Nothing is written to the serial port while it runs.
//!
//! # Memory layout
//!
//! A [`Header`] of [`HEADER_BYTES`], then `capacity` records of [`RECORD_BYTES`], all
//! little-endian `u64` words:
//!
//! | word | header | record |
//! |---|---|---|
//! | 0 | [`MAGIC`] | time, ns since boot |
//! | 1 | [`VERSION`] | thread ([`ident`]) |
//! | 2 | capacity (records) | `a` |
//! | 3 | next index (records ever written) | `b` |
//! | 4 | frozen (1 while frozen) | kind, CPU, flags, sequence ([`Meta`]) |
//! | 5–7 | 0 | |
//!
//! A record's slot is its index modulo the capacity. Its last word carries the low 32 bits of
//! its index plus one, written last, so a reader tells a whole record of index `i` from a slot
//! another CPU was still writing or that a later record took.

#![no_std]

use core::sync::atomic::{AtomicU64, Ordering};

/// "POLYTL01".
pub const MAGIC: u64 = u64::from_le_bytes(*b"POLYTL01");
/// The layout version.
pub const VERSION: u64 = 1;
/// Bytes before the first record.
pub const HEADER_BYTES: usize = 64;
/// Bytes of a record.
pub const RECORD_BYTES: usize = 40;
const WORDS: usize = RECORD_BYTES / 8;

/// What a record says. `a` and `b` per kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// The CPU switched to the record's thread. `a`: the thread switched away from; `b`: what
    /// that one became ([`Left`]).
    Switch = 1,
    /// The record's thread became runnable. `a`: the thread that was running on this CPU (the
    /// waker; [`Meta::IN_INTERRUPT`] when an interrupt handler did it); `b`: 0.
    Wake = 2,
    /// The record's thread entered a system call. `a`: [`call`]; `b`: its first argument.
    SysEnter = 3,
    /// The record's thread returned from a system call. `a`: [`call`]; `b`: the result register.
    SysExit = 4,
    /// A key went down or up. `a`: its evdev code; `b`: the value (1 down, 0 up, 2 repeat).
    Key = 5,
    /// The display sent a frame to the host. `a`: bytes transferred; `b`: rectangles.
    Frame = 6,
    /// The ring was frozen (the Pause key): the records before this one are the window.
    Freeze = 7,
    /// The ring runs again.
    Thaw = 8,
    /// The record's thread is about to block on a wait queue. `a`: the queue's address (the
    /// kernel names some: `timeline: queue`); `b`: 0.
    Block = 9,
    /// A timer tick found the record's thread running its program: `a`: the instruction pointer.
    Sample = 10,
}

impl Kind {
    pub fn from_u8(v: u8) -> Option<Kind> {
        Some(match v {
            1 => Kind::Switch,
            2 => Kind::Wake,
            3 => Kind::SysEnter,
            4 => Kind::SysExit,
            5 => Kind::Key,
            6 => Kind::Frame,
            7 => Kind::Freeze,
            8 => Kind::Thaw,
            9 => Kind::Block,
            10 => Kind::Sample,
            _ => return None,
        })
    }
}

/// What the thread a CPU switched away from became.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Left {
    /// Still runnable: preempted or yielded.
    Runnable = 0,
    /// Blocked on a wait queue.
    Blocked = 1,
    /// Exited.
    Exited = 2,
    /// Anything else.
    Other = 3,
}

/// The process id of kernel threads in an [`ident`].
pub const KERNEL_PID: u32 = 0xFFFF_FFFF;

/// A thread as records name it: a user thread by its process and its id in it, a kernel thread
/// by [`KERNEL_PID`] and its slot in the thread table.
pub const fn ident(pid: u32, tid: u32) -> u64 {
    (pid as u64) << 32 | tid as u64
}

/// The personalities in [`call`].
pub const LINUX: u8 = 0;
pub const NT: u8 = 1;
pub const XNU: u8 = 2;
pub const NATIVE: u8 = 3;

/// A system call as records name it: the personality in the top byte, the number below.
pub const fn call(personality: u8, number: u64) -> u64 {
    (personality as u64) << 56 | (number & 0x00FF_FFFF_FFFF_FFFF)
}

/// A record's last word: kind, CPU, flags, sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
    pub kind: u8,
    pub cpu: u8,
    pub flags: u16,
    /// The low 32 bits of the record's index plus one (0: never written).
    pub seq: u32,
}

impl Meta {
    /// The record was written by an interrupt handler.
    pub const IN_INTERRUPT: u16 = 1;

    pub const fn to_word(self) -> u64 {
        self.kind as u64 | (self.cpu as u64) << 8 | (self.flags as u64) << 16 | (self.seq as u64) << 32
    }

    pub const fn from_word(w: u64) -> Meta {
        Meta { kind: w as u8, cpu: (w >> 8) as u8, flags: (w >> 16) as u16, seq: (w >> 32) as u32 }
    }
}

/// One record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub t_ns: u64,
    pub ident: u64,
    pub a: u64,
    pub b: u64,
    pub kind: Kind,
    pub cpu: u8,
    pub flags: u16,
}

/// The header's words, as the ring sees them.
#[derive(Debug)]
#[repr(C)]
pub struct Header {
    magic: AtomicU64,
    version: AtomicU64,
    capacity: AtomicU64,
    next: AtomicU64,
    frozen: AtomicU64,
    _spare: [AtomicU64; 3],
}

const _: () = assert!(core::mem::size_of::<Header>() == HEADER_BYTES);

/// The ring over memory the kernel gave it.
#[derive(Debug)]
pub struct Ring<'a> {
    header: &'a Header,
    slots: &'a [[AtomicU64; WORDS]],
}

impl<'a> Ring<'a> {
    /// How many records fit in `bytes` of memory (0: not even the header).
    pub const fn capacity_for(bytes: usize) -> usize {
        if bytes < HEADER_BYTES { 0 } else { (bytes - HEADER_BYTES) / RECORD_BYTES }
    }

    /// Lay a ring out over `words` (zeroed or not): the header written, every slot empty.
    pub fn new(words: &'a mut [u64]) -> Ring<'a> {
        let capacity = Self::capacity_for(words.len() * 8);
        words.fill(0);
        let (head, rest) = words.split_at_mut(HEADER_BYTES / 8);
        let rest = &mut rest[..capacity * WORDS];
        // SAFETY: `AtomicU64` has the size and alignment of `u64`, and the `&mut` borrow of
        // `words` is held by the returned ring for 'a, so nothing else reaches them.
        let (header, slots) = unsafe {
            (
                &*(head.as_ptr().cast::<Header>()),
                core::slice::from_raw_parts(rest.as_ptr().cast::<[AtomicU64; WORDS]>(), capacity),
            )
        };
        header.magic.store(MAGIC, Ordering::Relaxed);
        header.version.store(VERSION, Ordering::Relaxed);
        header.capacity.store(capacity as u64, Ordering::Relaxed);
        Ring { header, slots }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Records ever written.
    pub fn written(&self) -> u64 {
        self.header.next.load(Ordering::Relaxed)
    }

    pub fn frozen(&self) -> bool {
        self.header.frozen.load(Ordering::Relaxed) != 0
    }

    /// Stop or restart recording. A [`Kind::Freeze`] or [`Kind::Thaw`] record is written
    /// (before the ring stops, after it runs again).
    pub fn set_frozen(&self, frozen: bool, t_ns: u64, cpu: u8) {
        if frozen {
            self.push(&Record { t_ns, ident: 0, a: 0, b: 0, kind: Kind::Freeze, cpu, flags: 0 });
            self.header.frozen.store(1, Ordering::SeqCst);
        } else {
            self.header.frozen.store(0, Ordering::SeqCst);
            self.push(&Record { t_ns, ident: 0, a: 0, b: 0, kind: Kind::Thaw, cpu, flags: 0 });
        }
    }

    /// Write `r`, unless the ring is frozen or has no room at all. Any CPU, any context.
    pub fn push(&self, r: &Record) -> bool {
        if self.slots.is_empty() || self.frozen() {
            return false;
        }
        let index = self.header.next.fetch_add(1, Ordering::Relaxed);
        let slot = &self.slots[(index % self.slots.len() as u64) as usize];
        // Mark the slot as being written first, so a reader never takes it for the old record.
        slot[4].store(0, Ordering::Relaxed);
        slot[0].store(r.t_ns, Ordering::Relaxed);
        slot[1].store(r.ident, Ordering::Relaxed);
        slot[2].store(r.a, Ordering::Relaxed);
        slot[3].store(r.b, Ordering::Relaxed);
        let meta = Meta { kind: r.kind as u8, cpu: r.cpu, flags: r.flags, seq: (index as u32).wrapping_add(1) };
        slot[4].store(meta.to_word(), Ordering::Release);
        true
    }
}

/// Read the records out of a copy of the ring's memory, oldest first. Slots being written or
/// overwritten while the copy was made are left out. `None` when `bytes` is not a ring.
pub fn read(bytes: &[u8], mut each: impl FnMut(u64, Record)) -> Option<()> {
    let word = |i: usize| -> Option<u64> { Some(u64::from_le_bytes(bytes.get(i * 8..i * 8 + 8)?.try_into().ok()?)) };
    if word(0)? != MAGIC || word(1)? != VERSION {
        return None;
    }
    let capacity = word(2)?;
    let next = word(3)?;
    if capacity == 0 || bytes.len() < HEADER_BYTES + capacity as usize * RECORD_BYTES {
        return None;
    }
    let first = next.saturating_sub(capacity);
    for index in first..next {
        let base = HEADER_BYTES / 8 + (index % capacity) as usize * WORDS;
        let meta = Meta::from_word(word(base + 4)?);
        if meta.seq != (index as u32).wrapping_add(1) {
            continue;
        }
        let Some(kind) = Kind::from_u8(meta.kind) else { continue };
        each(
            index,
            Record {
                t_ns: word(base)?,
                ident: word(base + 1)?,
                a: word(base + 2)?,
                b: word(base + 3)?,
                kind,
                cpu: meta.cpu,
                flags: meta.flags,
            },
        );
    }
    Some(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;

    fn bytes_of(words: &[u64]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn rec(t: u64, kind: Kind) -> Record {
        Record { t_ns: t, ident: ident(7, t as u32), a: t * 10, b: t * 100, kind, cpu: (t % 4) as u8, flags: 0 }
    }

    #[test]
    fn records_come_back_in_order_with_their_fields() {
        let mut mem = vec![0u64; (HEADER_BYTES + 10 * RECORD_BYTES) / 8];
        let ring = Ring::new(&mut mem);
        assert_eq!(ring.capacity(), 10);
        for t in 1..=4 {
            assert!(ring.push(&rec(t, Kind::SysEnter)));
        }
        let mut got = Vec::new();
        read(&bytes_of(&mem), |i, r| got.push((i, r))).unwrap();
        assert_eq!(got.len(), 4);
        for (n, (i, r)) in got.iter().enumerate() {
            assert_eq!(*i, n as u64);
            assert_eq!(*r, rec(n as u64 + 1, Kind::SysEnter));
        }
    }

    #[test]
    fn a_full_ring_keeps_the_newest_records() {
        let mut mem = vec![0u64; (HEADER_BYTES + 5 * RECORD_BYTES) / 8];
        let ring = Ring::new(&mut mem);
        for t in 1..=12 {
            ring.push(&rec(t, Kind::Switch));
        }
        assert_eq!(ring.written(), 12);
        let mut ts = Vec::new();
        read(&bytes_of(&mem), |_, r| ts.push(r.t_ns)).unwrap();
        assert_eq!(ts, [8, 9, 10, 11, 12]);
    }

    #[test]
    fn a_frozen_ring_writes_nothing_until_thawed() {
        let mut mem = vec![0u64; (HEADER_BYTES + 8 * RECORD_BYTES) / 8];
        let ring = Ring::new(&mut mem);
        ring.push(&rec(1, Kind::Key));
        ring.set_frozen(true, 2, 0);
        assert!(ring.frozen());
        assert!(!ring.push(&rec(3, Kind::Key)));
        ring.set_frozen(false, 4, 0);
        ring.push(&rec(5, Kind::Key));
        let mut kinds = Vec::new();
        read(&bytes_of(&mem), |_, r| kinds.push((r.t_ns, r.kind))).unwrap();
        assert_eq!(kinds, [(1, Kind::Key), (2, Kind::Freeze), (4, Kind::Thaw), (5, Kind::Key)]);
    }

    #[test]
    fn a_slot_in_writing_or_taken_by_a_later_record_is_left_out() {
        let mut mem = vec![0u64; (HEADER_BYTES + 4 * RECORD_BYTES) / 8];
        let ring = Ring::new(&mut mem);
        for t in 1..=3 {
            ring.push(&rec(t, Kind::Wake));
        }
        let mut bytes = bytes_of(&mem);
        // Record 1's slot half written: its meta word is 0.
        let meta = HEADER_BYTES + RECORD_BYTES + 32;
        bytes[meta..meta + 8].fill(0);
        let mut ts = Vec::new();
        read(&bytes, |_, r| ts.push(r.t_ns)).unwrap();
        assert_eq!(ts, [1, 3]);
    }

    #[test]
    fn meta_words_and_idents_round_trip() {
        let m = Meta { kind: 6, cpu: 3, flags: Meta::IN_INTERRUPT, seq: 0xdead_beef };
        assert_eq!(Meta::from_word(m.to_word()), m);
        assert_eq!(ident(KERNEL_PID, 5) >> 32, u64::from(KERNEL_PID));
        assert_eq!(call(NT, 0x1003) >> 56, u64::from(NT));
        assert_eq!(call(NT, 0x1003) & 0xFFFF, 0x1003);
    }

    #[test]
    fn memory_that_is_not_a_ring_is_refused() {
        assert!(read(&[0u8; 200], |_, _| {}).is_none());
        assert_eq!(Ring::capacity_for(10), 0);
    }
}
