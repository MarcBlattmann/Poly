//! Native Linux asynchronous I/O ("Linux AIO", io_setup(2) and friends):
//! the contexts, their completion rings and the operations still pending.
//! The system calls are in [`crate::sys::aio`]; this module is the state
//! they share, kept free of the kernel so it can be tested on the host.
//!
//! **The ring.** A context's id is the user address of its completion
//! ring, a shared read-write mapping in the caller's address space that
//! libaio reads directly (`linux/aio_abi.h` defines `struct io_event`; the
//! 32-byte header is described in io_setup's spec sheet): eight `u32`s
//! (`id`, `nr`, `head`, `tail`, magic `0xa10a10a1`, compat features 1,
//! incompat features 0, header length 32) followed by `nr` 32-byte event
//! slots. Completions are written at `tail`; reaping reads from `head`.
//! The kernel keeps its own `head` and `tail` here and mirrors them into
//! the header; before reaping it takes `head` back from the header, so a
//! library that reaps in user space (advancing `head` itself) stays in
//! step.
//!
//! **Ownership.** A context belongs to the address space that made it
//! (every thread of the process may use it; a forked child, with an
//! address space of its own, may not: `EINVAL`) and to the process that
//! made it, which matters for teardown: the context goes when its ring is
//! unmapped, when io_destroy(2) removes it, and when its process exits or
//! execs (its address space ends).
//!
//! **What completes when.** Reads, writes and syncs are done inside
//! io_submit, so their events are in the ring when it returns. Only
//! `IOCB_CMD_POLL` stays pending; Poly completes a pending poll when the
//! process next looks at the context (io_submit, io_getevents, io_cancel),
//! and io_getevents waits on the polled files. That is Poly's
//! simplification: a pending poll does not complete (nor signal its
//! eventfd) while the process is busy elsewhere.

use crate::errno::Errno;
use alloc::vec::Vec;

/// `AIO_RING_MAGIC` (the value io_setup's spec sheet gives for the ring
/// header's magic field).
pub const AIO_RING_MAGIC: u32 = 0xa10a_10a1;
/// The ring header's `compat_features` and `header_length`.
pub const AIO_RING_COMPAT_FEATURES: u32 = 1;
pub const HEADER_LEN: u64 = 32;
/// `struct io_event` (`linux/aio_abi.h`): data, obj, res, res2.
pub const EVENT_SIZE: u64 = 32;
/// `struct iocb` (`linux/aio_abi.h`).
pub const IOCB_SIZE: usize = 64;
/// `/proc/sys/fs/aio-max-nr`'s default (io_setup's spec sheet: 65536).
pub const AIO_MAX_NR: u64 = 65536;
/// The largest ring Poly makes; a request that needs more is absurd
/// (`EINVAL`; io_setup's spec sheet: "more than about 4 million" events).
pub const MAX_RING_BYTES: u64 = 128 << 20;
const PAGE: u64 = 4096;

/// `IOCB_CMD_*` (`linux/aio_abi.h`).
pub mod cmd {
    pub const PREAD: u16 = 0;
    pub const PWRITE: u16 = 1;
    pub const FSYNC: u16 = 2;
    pub const FDSYNC: u16 = 3;
    pub const POLL: u16 = 5;
    pub const NOOP: u16 = 6;
    pub const PREADV: u16 = 7;
    pub const PWRITEV: u16 = 8;
}

/// `IOCB_FLAG_RESFD` (`linux/aio_abi.h`): signal the eventfd `aio_resfd`.
pub const IOCB_FLAG_RESFD: u32 = 1;

/// The fewest events a ring is sized for: io_setup's spec sheet observed
/// 383 slots (three pages) "for any request up to 128".
pub const MIN_RING_EVENTS: u64 = 128;

/// The size of a ring for `nr_events`: `(slots, bytes)`. As the spec sheet
/// observed, the ring is sized for twice the larger of `nr_events` and
/// [`MIN_RING_EVENTS`], in whole pages (so 1 to 128 events give 383 slots),
/// which leaves room for well over `nr_events` outstanding events.
/// `None` if `nr_events` is absurdly large (its own ring would exceed
/// [`MAX_RING_BYTES`]).
pub fn ring_size(nr_events: u32) -> Option<(u32, u64)> {
    if HEADER_LEN + (u64::from(nr_events) + 1) * EVENT_SIZE > MAX_RING_BYTES {
        return None;
    }
    let want = HEADER_LEN + 2 * u64::from(nr_events).max(MIN_RING_EVENTS) * EVENT_SIZE;
    let bytes = want.div_ceil(PAGE) * PAGE;
    Some((((bytes - HEADER_LEN) / EVENT_SIZE) as u32, bytes))
}

/// The ring header for a ring at index `id` with `nr` slots.
pub fn header(id: u32, nr: u32, head: u32, tail: u32) -> [u8; HEADER_LEN as usize] {
    let fields = [id, nr, head, tail, AIO_RING_MAGIC, AIO_RING_COMPAT_FEATURES, 0, HEADER_LEN as u32];
    let mut out = [0; HEADER_LEN as usize];
    for (chunk, f) in out.chunks_exact_mut(4).zip(fields) {
        chunk.copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// One completion, as `struct io_event`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub data: u64,
    pub obj: u64,
    pub res: i64,
    pub res2: i64,
}

impl Event {
    pub fn to_bytes(self) -> [u8; EVENT_SIZE as usize] {
        let mut out = [0; EVENT_SIZE as usize];
        out[..8].copy_from_slice(&self.data.to_le_bytes());
        out[8..16].copy_from_slice(&self.obj.to_le_bytes());
        out[16..24].copy_from_slice(&self.res.to_le_bytes());
        out[24..].copy_from_slice(&self.res2.to_le_bytes());
        out
    }
}

/// `struct iocb` (`linux/aio_abi.h`, little-endian field order).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Iocb {
    pub data: u64,
    pub key: u32,
    pub rw_flags: u32,
    pub opcode: u16,
    pub reqprio: i16,
    pub fildes: u32,
    pub buf: u64,
    pub nbytes: u64,
    pub offset: i64,
    pub reserved2: u64,
    pub flags: u32,
    pub resfd: u32,
}

impl Iocb {
    pub fn from_bytes(b: &[u8; IOCB_SIZE]) -> Iocb {
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or_default());
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default());
        let u16_at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap_or_default());
        Iocb {
            data: u64_at(0),
            key: u32_at(8),
            rw_flags: u32_at(12),
            opcode: u16_at(16),
            reqprio: u16_at(18) as i16,
            fildes: u32_at(20),
            buf: u64_at(24),
            nbytes: u64_at(32),
            offset: u64_at(40) as i64,
            reserved2: u64_at(48),
            flags: u32_at(56),
            resfd: u32_at(60),
        }
    }
}

/// A poll that waits for its event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The iocb's user address (the event's `obj`, io_cancel's key).
    pub iocb: u64,
    pub data: u64,
    pub fd: i32,
    pub mask: u16,
    /// The eventfd to signal on completion.
    pub resfd: Option<i32>,
}

/// One AIO context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    /// The address space (`LinuxKernel::address_space_id`) it lives in.
    pub space: u64,
    /// The process that made it.
    pub owner: i32,
    /// The ring's address: the context id.
    pub base: u64,
    /// The ring mapping's length in bytes.
    pub len: u64,
    /// The ring header's `id`: an index among the address space's contexts.
    pub index: u32,
    /// Event slots.
    pub nr: u32,
    pub head: u32,
    pub tail: u32,
    /// The `nr_events` asked for, counted against [`AIO_MAX_NR`].
    pub requested: u32,
    pub pending: Vec<Pending>,
}

impl Context {
    /// Completed events not reaped yet.
    pub fn available(&self) -> u32 {
        (self.tail + self.nr - self.head) % self.nr
    }

    /// Whether another operation fits: unreaped events and pending
    /// operations together stay below the ring's slots (one stays empty).
    pub fn has_room(&self) -> bool {
        (self.available() as usize + self.pending.len()) < self.nr as usize - 1
    }

    /// The user address of slot `i`.
    pub fn slot_addr(&self, i: u32) -> u64 {
        self.base + HEADER_LEN + u64::from(i % self.nr) * EVENT_SIZE
    }

    /// Take the slot for the next event, advancing the tail.
    pub fn claim_slot(&mut self) -> u32 {
        let slot = self.tail;
        self.tail = (self.tail + 1) % self.nr;
        slot
    }

    /// Mark `n` events reaped, advancing the head.
    pub fn consume(&mut self, n: u32) {
        self.head = (self.head + n.min(self.available())) % self.nr;
    }

    /// Adopt a head that user space wrote into the ring header (it may
    /// reap there itself); nonsense is ignored.
    pub fn adopt_head(&mut self, head: u32) {
        let reaped = (head + self.nr - self.head) % self.nr;
        if head < self.nr && reaped <= self.available() {
            self.head = head;
        }
    }

    fn overlaps(&self, addr: u64, len: u64) -> bool {
        self.base < addr.saturating_add(len) && addr < self.base + self.len
    }
}

/// Every live context of the system.
#[derive(Debug, Default)]
pub struct AioTable {
    contexts: Vec<Context>,
    /// `/proc/sys/fs/aio-nr`: events asked for by live contexts.
    pub aio_nr: u64,
}

impl AioTable {
    pub const fn new() -> AioTable {
        AioTable { contexts: Vec::new(), aio_nr: 0 }
    }

    /// May a context for `nr_events` be made? `EINVAL` for 0 or an absurd
    /// size, `EAGAIN` above [`AIO_MAX_NR`]; else its ring size.
    pub fn check_new(&self, nr_events: u32) -> Result<(u32, u64), Errno> {
        if nr_events == 0 {
            return Err(Errno::EINVAL);
        }
        let size = ring_size(nr_events).ok_or(Errno::EINVAL)?;
        if self.aio_nr + u64::from(nr_events) > AIO_MAX_NR {
            return Err(Errno::EAGAIN);
        }
        Ok(size)
    }

    /// The lowest ring index not used in `space`.
    pub fn free_index(&self, space: u64) -> u32 {
        (0..).find(|i| !self.contexts.iter().any(|c| c.space == space && c.index == *i)).unwrap_or(0)
    }

    /// Record a new context whose ring is mapped at `base`.
    pub fn insert(&mut self, space: u64, owner: i32, base: u64, len: u64, nr: u32, requested: u32) -> &Context {
        let index = self.free_index(space);
        self.aio_nr += u64::from(requested);
        self.contexts.push(Context {
            space,
            owner,
            base,
            len,
            index,
            nr,
            head: 0,
            tail: 0,
            requested,
            pending: Vec::new(),
        });
        let last = self.contexts.len() - 1;
        &self.contexts[last]
    }

    /// The context `id` of address space `space`.
    pub fn get(&self, space: u64, id: u64) -> Option<&Context> {
        self.contexts.iter().find(|c| c.space == space && c.base == id)
    }

    pub fn get_mut(&mut self, space: u64, id: u64) -> Option<&mut Context> {
        self.contexts.iter_mut().find(|c| c.space == space && c.base == id)
    }

    /// Remove context `id` of `space`; its pending operations go with it.
    pub fn remove(&mut self, space: u64, id: u64) -> Option<Context> {
        let i = self.contexts.iter().position(|c| c.space == space && c.base == id)?;
        let c = self.contexts.remove(i);
        self.aio_nr -= u64::from(c.requested);
        Some(c)
    }

    /// Remove every context of `space` matching `which`.
    fn remove_where(&mut self, which: impl Fn(&Context) -> bool) -> usize {
        let before = self.contexts.len();
        let mut freed = 0;
        self.contexts.retain(|c| {
            let gone = which(c);
            if gone {
                freed += u64::from(c.requested);
            }
            !gone
        });
        self.aio_nr -= freed;
        before - self.contexts.len()
    }

    /// `[addr, addr + len)` of `space` was unmapped: contexts whose ring
    /// was (partly) there are gone.
    pub fn unmapped(&mut self, space: u64, addr: u64, len: u64) -> usize {
        self.remove_where(|c| c.space == space && c.overlaps(addr, len))
    }

    /// Process `owner` left address space `space` (exit, execve; a
    /// `space` of 0 when the kernel no longer knows it matches any).
    pub fn left(&mut self, space: u64, owner: i32) -> usize {
        self.remove_where(|c| c.owner == owner && (space == 0 || c.space == space))
    }

    /// Take pending operation `iocb` out of context `id` (io_cancel).
    pub fn take_pending(&mut self, space: u64, id: u64, iocb: u64) -> Option<Pending> {
        let c = self.get_mut(space, id)?;
        let i = c.pending.iter().position(|p| p.iocb == iocb)?;
        Some(c.pending.remove(i))
    }

    /// How many contexts there are.
    pub fn len(&self) -> usize {
        self.contexts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_sizes() {
        assert_eq!(ring_size(1), Some((383, 12288)), "the spec sheet's 383 slots");
        assert_eq!(ring_size(128), Some((383, 12288)));
        assert_eq!(ring_size(191), Some((383, 12288)));
        assert_eq!(ring_size(192), Some((511, 16384)));
        assert!(ring_size(65536).unwrap().0 >= 2 * 65536);
        assert_eq!(ring_size(0x7fff_ffff), None);
        assert_eq!(ring_size(u32::MAX), None);
    }

    #[test]
    fn header_layout() {
        let h = header(2, 127, 3, 4);
        let field = |i: usize| u32::from_le_bytes(h[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!((field(0), field(1), field(2), field(3)), (2, 127, 3, 4));
        assert_eq!((field(4), field(5), field(6), field(7)), (AIO_RING_MAGIC, 1, 0, 32));
    }

    #[test]
    fn iocb_and_event_layouts() {
        let mut b = [0u8; IOCB_SIZE];
        b[..8].copy_from_slice(&0x1234u64.to_le_bytes());
        b[8..12].copy_from_slice(&5u32.to_le_bytes());
        b[16..18].copy_from_slice(&cmd::POLL.to_le_bytes());
        b[20..24].copy_from_slice(&7u32.to_le_bytes());
        b[40..48].copy_from_slice(&(-1i64).to_le_bytes());
        b[56..60].copy_from_slice(&IOCB_FLAG_RESFD.to_le_bytes());
        b[60..64].copy_from_slice(&9u32.to_le_bytes());
        let cb = Iocb::from_bytes(&b);
        assert_eq!((cb.data, cb.key, cb.opcode, cb.fildes, cb.offset), (0x1234, 5, cmd::POLL, 7, -1));
        assert_eq!((cb.flags, cb.resfd), (IOCB_FLAG_RESFD, 9));
        let e = Event { data: 1, obj: 2, res: -14, res2: 0 }.to_bytes();
        assert_eq!(i64::from_le_bytes(e[16..24].try_into().unwrap()), -14);
        assert_eq!(u64::from_le_bytes(e[8..16].try_into().unwrap()), 2);
    }

    #[test]
    fn limits() {
        let mut t = AioTable::new();
        assert_eq!(t.check_new(0), Err(Errno::EINVAL));
        assert_eq!(t.check_new(0x7fff_ffff), Err(Errno::EINVAL));
        assert_eq!(t.check_new(70000), Err(Errno::EAGAIN));
        t.insert(1, 10, 0x1000, 4096, 127, 65000);
        assert_eq!(t.check_new(600), Err(Errno::EAGAIN));
        assert!(t.check_new(536).is_ok());
        t.remove(1, 0x1000);
        assert_eq!(t.aio_nr, 0);
    }

    #[test]
    fn contexts_belong_to_their_address_space() {
        let mut t = AioTable::new();
        assert_eq!(t.insert(1, 10, 0x1000, 4096, 127, 1).index, 0);
        assert_eq!(t.insert(1, 10, 0x3000, 4096, 127, 1).index, 1);
        assert_eq!(t.insert(2, 11, 0x1000, 4096, 127, 1).index, 0, "a forked child's own index");
        assert!(t.get(1, 0x1000).is_some() && t.get(2, 0x3000).is_none());
        t.remove(1, 0x1000);
        assert_eq!(t.insert(1, 10, 0x5000, 4096, 127, 1).index, 0, "the lowest free index");
        assert_eq!(t.unmapped(1, 0x3000, 4096), 1);
        assert_eq!(t.unmapped(1, 0x6000, 4096), 0, "a neighbour");
        assert_eq!(t.unmapped(1, 0x4000, 0x2000), 1, "partly");
        assert_eq!(t.left(2, 99), 0);
        assert_eq!(t.left(0, 11), 1);
        assert!(t.is_empty() && t.aio_nr == 0);
    }

    #[test]
    fn ring_accounting() {
        let mut t = AioTable::new();
        t.insert(1, 10, 0x1000, 4096, 4, 2);
        let c = t.get_mut(1, 0x1000).unwrap();
        assert!(c.has_room());
        assert_eq!(c.claim_slot(), 0);
        assert_eq!(c.claim_slot(), 1);
        c.pending.push(Pending { iocb: 9, data: 0, fd: 3, mask: 1, resfd: None });
        assert_eq!(c.available(), 2);
        assert!(!c.has_room(), "two events and a poll fill a ring of four");
        assert_eq!(c.slot_addr(5), 0x1000 + 32 + 32);
        c.consume(1);
        assert_eq!((c.head, c.available()), (1, 1));
        c.adopt_head(3);
        assert_eq!(c.head, 1, "a head past the tail is ignored");
        c.adopt_head(2);
        assert_eq!((c.head, c.available()), (2, 0));
        assert_eq!(c.claim_slot(), 2);
        assert_eq!(c.claim_slot(), 3);
        assert_eq!(c.claim_slot(), 0, "wraps");
        assert_eq!(c.available(), 3);
        assert_eq!(t.take_pending(1, 0x1000, 9).map(|p| p.fd), Some(3));
        assert_eq!(t.take_pending(1, 0x1000, 9), None);
    }
}
