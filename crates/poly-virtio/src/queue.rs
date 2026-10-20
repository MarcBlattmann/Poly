//! Split virtqueues (§2.7).
//!
//! A split virtqueue of size N has three parts in memory both sides share:
//!
//! ```text
//! descriptor table  16·N bytes, align 16   { le64 addr; le32 len; le16 flags; le16 next; }[N]
//! available ring    6 + 2·N,    align 2    le16 flags; le16 idx; le16 ring[N]; le16 used_event;
//! used ring         6 + 8·N,    align 4    le16 flags; le16 idx; { le32 id; le32 len; }[N]; le16 avail_event;
//! ```
//!
//! The driver chains descriptors into a buffer (device-readable parts first,
//! then device-writable ones), puts the head's index into the available ring
//! and bumps `avail.idx`. The device hands the head back through the used
//! ring with the number of bytes it wrote. Both indices run freely and wrap
//! at 2^16; the ring slot is the index modulo N, which is why N is a power of
//! two (§2.7: at most 32768).
//!
//! # Memory ordering
//!
//! The device is another agent working on the same memory, so every access
//! to the queue memory is volatile (the compiler may neither drop nor merge
//! them) and three fences order them against the device (§2.7.13,
//! §2.7.14, §2.7.7):
//!
//! - **Release** before writing `avail.idx`: the descriptors and the ring
//!   entry must be visible before the device can see the new index
//!   (§2.7.13.3).
//! - **Acquire** after reading `used.idx`: the used elements and the
//!   data the device wrote into the buffers must not be read before it
//!   (§2.7.14).
//! - **SeqCst** between publishing a buffer and reading the device's
//!   notification suppression (`used.flags` or `avail_event`), and between
//!   re-enabling interrupts and re-checking `used.idx` (§2.7.13.4, §2.7.7):
//!   these are store-then-load orders, which only a full barrier gives.
//!
//! On x86_64 the Release and Acquire fences cost nothing at run time
//! (stores are not reordered with stores, nor loads with loads, and DMA is
//! cache coherent) but stop the compiler moving accesses across them; the
//! SeqCst fence is an `mfence`.
//!
//! Not implemented: indirect descriptors (VIRTIO_F_INDIRECT_DESC) and the
//! packed ring layout. VIRTIO_F_EVENT_IDX is supported.
//!
//! The driver never trusts what it reads back from the shared memory: the
//! free list and the shape of every chain in flight live in driver-private
//! [`DescState`]s, and the device's used elements are checked against them.

use core::fmt;
use core::ptr::{self, NonNull};
use core::sync::atomic::{Ordering, fence};

/// Descriptor flags (§2.7.5).
mod desc_flags {
    pub const NEXT: u16 = 1;
    pub const WRITE: u16 = 2;
}

/// `avail.flags`: please do not interrupt (§2.7.7).
const AVAIL_F_NO_INTERRUPT: u16 = 1;
/// `used.flags`: please do not notify (§2.7.13.4).
const USED_F_NO_NOTIFY: u16 = 1;

const DESC_SIZE: usize = 16;
const USED_ELEM_SIZE: usize = 8;
/// `flags` and `idx` in front of each ring.
const RING_HEADER: usize = 4;

/// Where the parts of a split virtqueue of a given size go in one
/// contiguous block of queue memory: descriptor table at offset 0, then the
/// available ring, then the used ring at the next multiple of 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    size: u16,
    avail: usize,
    used: usize,
    total: usize,
}

impl Layout {
    pub const MAX_SIZE: u16 = 32768;
    /// Alignment the whole block needs (the descriptor table's).
    pub const ALIGN: usize = 16;

    /// `size` must be a power of two from 1 to 32768.
    pub const fn new(size: u16) -> Option<Layout> {
        if !size.is_power_of_two() || size > Self::MAX_SIZE {
            return None;
        }
        let n = size as usize;
        let avail = DESC_SIZE * n;
        let avail_end = avail + RING_HEADER + 2 * n + 2;
        let used = avail_end.next_multiple_of(4);
        let total = used + RING_HEADER + USED_ELEM_SIZE * n + 2;
        Some(Layout { size, avail, used, total })
    }

    pub const fn size(&self) -> u16 {
        self.size
    }

    /// Offset of the available ring ("driver area").
    pub const fn avail_offset(&self) -> usize {
        self.avail
    }

    /// Offset of the used ring ("device area").
    pub const fn used_offset(&self) -> usize {
        self.used
    }

    /// Bytes of queue memory needed.
    pub const fn total(&self) -> usize {
        self.total
    }

    const fn desc(&self, i: u16) -> usize {
        i as usize * DESC_SIZE
    }

    const fn avail_idx(&self) -> usize {
        self.avail + 2
    }

    const fn avail_ring(&self, slot: u16) -> usize {
        self.avail + RING_HEADER + 2 * slot as usize
    }

    const fn used_event(&self) -> usize {
        self.avail_ring(self.size)
    }

    const fn used_idx(&self) -> usize {
        self.used + 2
    }

    const fn used_elem(&self, slot: u16) -> usize {
        self.used + RING_HEADER + USED_ELEM_SIZE * slot as usize
    }

    const fn avail_event(&self) -> usize {
        self.used_elem(self.size)
    }
}

/// Bus addresses of the three parts, to program into the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueAddresses {
    pub desc: u64,
    /// The available ring.
    pub driver: u64,
    /// The used ring.
    pub device: u64,
}

/// One piece of a buffer: bus address and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buffer {
    pub addr: u64,
    pub len: u32,
}

/// Driver-private bookkeeping for one descriptor. The caller provides one
/// per queue entry (`[DescState::default(); N]`) and otherwise leaves them
/// alone.
#[derive(Clone, Copy, Debug, Default)]
pub struct DescState {
    /// Next descriptor in the chain while in flight, next free one
    /// otherwise.
    next: u16,
    /// On the head of a chain in flight: its length. 0 otherwise.
    chain: u16,
    /// On the head of a chain in flight: total device-writable bytes.
    writable: u32,
}

/// A buffer the device has finished with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Used {
    /// The head descriptor [`SplitQueue::add`] returned for it.
    pub head: u16,
    /// Bytes the device wrote, never more than the buffer's writable part.
    pub len: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// The queue memory is not aligned to [`Layout::ALIGN`], in the CPU's
    /// or the device's view.
    Misaligned,
    /// Fewer [`DescState`]s than queue entries.
    StateTooSmall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddError {
    /// A buffer needs at least one piece.
    NoBuffers,
    /// More pieces than the queue has descriptors: can never fit.
    TooLong,
    /// Not enough free descriptors right now; retry after harvesting used
    /// buffers.
    Full,
}

/// The device broke the protocol. The queue is unusable until the device is
/// reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsedError {
    /// `used.idx` moved further than there are buffers in flight.
    IndexJump,
    /// A used element names a descriptor that is not the head of a buffer
    /// in flight.
    BadId(u32),
}

/// The driver side of a split virtqueue.
pub struct SplitQueue<'a> {
    base: NonNull<u8>,
    phys: u64,
    layout: Layout,
    state: &'a mut [DescState],
    free_head: u16,
    num_free: u16,
    /// Buffers handed to the device and not yet back.
    in_flight: u16,
    /// Our copy of `avail.idx`.
    avail_idx: u16,
    /// `used.idx` up to which we have harvested.
    last_used: u16,
    /// `avail.idx` when we last decided whether to notify.
    notified: u16,
    event_idx: bool,
    interrupts: bool,
}

// SAFETY: the queue memory belongs to the queue alone (the contract of
// `SplitQueue::new`), so moving the queue to another CPU moves the only
// driver-side user of it with it.
unsafe impl Send for SplitQueue<'_> {}

impl fmt::Debug for SplitQueue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SplitQueue")
            .field("phys", &self.phys)
            .field("size", &self.layout.size)
            .field("num_free", &self.num_free)
            .field("in_flight", &self.in_flight)
            .field("avail_idx", &self.avail_idx)
            .field("last_used", &self.last_used)
            .field("event_idx", &self.event_idx)
            .finish()
    }
}

impl<'a> SplitQueue<'a> {
    /// Set up an empty queue in `mem`: zero it and put every descriptor on
    /// the free list. `event_idx` says whether VIRTIO_F_EVENT_IDX was
    /// negotiated.
    ///
    /// # Safety
    ///
    /// `mem` must point to `layout.total()` bytes that are valid for reads
    /// and writes for `'a` and that the device reaches at bus address
    /// `phys`. Nothing but this queue and the device may access them while
    /// the queue exists, and the device must not be using them yet (its
    /// queue is disabled).
    pub unsafe fn new(
        mem: NonNull<u8>,
        phys: u64,
        layout: Layout,
        state: &'a mut [DescState],
        event_idx: bool,
    ) -> Result<SplitQueue<'a>, SetupError> {
        if !(mem.as_ptr() as usize).is_multiple_of(Layout::ALIGN) || !phys.is_multiple_of(Layout::ALIGN as u64) {
            return Err(SetupError::Misaligned);
        }
        let size = layout.size;
        let Some(state) = state.get_mut(..size as usize) else {
            return Err(SetupError::StateTooSmall);
        };
        for (i, s) in state.iter_mut().enumerate() {
            *s = DescState { next: i as u16 + 1, chain: 0, writable: 0 };
        }
        let q = SplitQueue {
            base: mem,
            phys,
            layout,
            state,
            free_head: 0,
            num_free: size,
            in_flight: 0,
            avail_idx: 0,
            last_used: 0,
            notified: 0,
            event_idx,
            interrupts: true,
        };
        // The total is even and every offset is 2-aligned.
        for off in (0..layout.total).step_by(2) {
            q.write16(off, 0);
        }
        Ok(q)
    }

    pub const fn size(&self) -> u16 {
        self.layout.size
    }

    pub const fn num_free(&self) -> u16 {
        self.num_free
    }

    /// Buffers the device has not handed back yet.
    pub const fn in_flight(&self) -> u16 {
        self.in_flight
    }

    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// Where the device finds each part.
    pub const fn addresses(&self) -> QueueAddresses {
        QueueAddresses {
            desc: self.phys,
            driver: self.phys + self.layout.avail as u64,
            device: self.phys + self.layout.used as u64,
        }
    }

    /// Hand a buffer to the device: `readable` pieces (data for the device)
    /// followed by `writable` ones (room for its answer). Returns the head
    /// descriptor, which identifies the buffer when it comes back from
    /// [`SplitQueue::pop_used`]. The memory the pieces describe must stay
    /// valid until then. Call [`SplitQueue::should_notify`] afterwards.
    pub fn add(&mut self, readable: &[Buffer], writable: &[Buffer]) -> Result<u16, AddError> {
        let count = readable.len() + writable.len();
        if count == 0 {
            return Err(AddError::NoBuffers);
        }
        if count > self.size() as usize {
            return Err(AddError::TooLong);
        }
        if count > self.num_free as usize {
            return Err(AddError::Full);
        }
        // Take descriptors off the front of the free list. The chain links
        // are simply the free list links, so they need no rewriting.
        let head = self.free_head;
        let mut desc = head;
        let mut writable_len = 0u32;
        let pieces = readable.iter().map(|b| (b, 0)).chain(writable.iter().map(|b| (b, desc_flags::WRITE)));
        for (i, (buf, write)) in pieces.enumerate() {
            let next = self.state[desc as usize].next;
            let last = i + 1 == count;
            let flags = write | if last { 0 } else { desc_flags::NEXT };
            let off = self.layout.desc(desc);
            self.write64(off, buf.addr);
            self.write32(off + 8, buf.len);
            self.write16(off + 12, flags);
            self.write16(off + 14, if last { 0 } else { next });
            if write != 0 {
                writable_len = writable_len.saturating_add(buf.len);
            }
            if !last {
                desc = next;
            } else {
                self.free_head = next;
            }
        }
        self.num_free -= count as u16;
        self.in_flight += 1;
        let h = &mut self.state[head as usize];
        h.chain = count as u16;
        h.writable = writable_len;

        // §2.7.13.2: ring entry first, then (after a barrier) the index.
        let slot = self.avail_idx & (self.size() - 1);
        self.write16(self.layout.avail_ring(slot), head);
        fence(Ordering::Release);
        self.avail_idx = self.avail_idx.wrapping_add(1);
        self.write16(self.layout.avail_idx(), self.avail_idx);
        Ok(head)
    }

    /// Does the device want to be told about the buffers added since the
    /// last call (§2.7.13.4)? If so, the caller writes the notification
    /// register (see [`crate::Notifier`]).
    pub fn should_notify(&mut self) -> bool {
        // Our `avail.idx` store must be visible before we read whether the
        // device wants a notification; otherwise both sides can decide the
        // other one will act.
        fence(Ordering::SeqCst);
        let old = self.notified;
        let new = self.avail_idx;
        self.notified = new;
        if old == new {
            return false;
        }
        if self.event_idx {
            need_event(self.read16(self.layout.avail_event()), new, old)
        } else {
            self.read16(self.layout.used) & USED_F_NO_NOTIFY == 0
        }
    }

    /// Is there a used buffer waiting to be harvested?
    pub fn has_used(&self) -> bool {
        self.read16(self.layout.used_idx()) != self.last_used
    }

    /// Take the next buffer the device has finished with, putting its
    /// descriptors back on the free list. `Ok(None)` if there is none.
    pub fn pop_used(&mut self) -> Result<Option<Used>, UsedError> {
        let used_idx = self.read16(self.layout.used_idx());
        if used_idx == self.last_used {
            return Ok(None);
        }
        if used_idx.wrapping_sub(self.last_used) > self.in_flight {
            return Err(UsedError::IndexJump);
        }
        // Read the element (and let the caller read the buffer) only after
        // the index that published it.
        fence(Ordering::Acquire);
        let elem = self.layout.used_elem(self.last_used & (self.size() - 1));
        let id = self.read32(elem);
        let len = self.read32(elem + 4);
        let head = match self.state.get(id as usize) {
            Some(s) if s.chain != 0 => id as u16,
            _ => return Err(UsedError::BadId(id)),
        };
        let used = Used { head, len: len.min(self.state[head as usize].writable) };
        self.free_chain(head);
        self.last_used = self.last_used.wrapping_add(1);
        if self.event_idx && self.interrupts {
            // Interrupt again once the device moves past this point.
            self.write16(self.layout.used_event(), self.last_used);
        }
        Ok(Some(used))
    }

    fn free_chain(&mut self, head: u16) {
        let h = &mut self.state[head as usize];
        let count = h.chain;
        h.chain = 0;
        h.writable = 0;
        let mut tail = head;
        for _ in 1..count {
            tail = self.state[tail as usize].next;
        }
        self.state[tail as usize].next = self.free_head;
        self.free_head = head;
        self.num_free += count;
        self.in_flight -= 1;
    }

    /// Ask the device not to interrupt for used buffers. Only a hint
    /// (§2.7.7): with EVENT_IDX the device may still interrupt once more.
    pub fn disable_interrupts(&mut self) {
        self.interrupts = false;
        if !self.event_idx {
            self.write16(self.layout.avail, AVAIL_F_NO_INTERRUPT);
        }
    }

    /// Ask for interrupts again. Returns true if used buffers arrived in
    /// the meantime, which may not raise an interrupt: the caller must
    /// harvest them before waiting.
    pub fn enable_interrupts(&mut self) -> bool {
        self.interrupts = true;
        if self.event_idx {
            self.write16(self.layout.used_event(), self.last_used);
        } else {
            self.write16(self.layout.avail, 0);
        }
        fence(Ordering::SeqCst);
        self.has_used()
    }

    // All accesses below stay inside the queue memory: every offset comes
    // from `Layout` with a descriptor or slot index below the queue size,
    // and is aligned for its width because the memory is 16-byte aligned
    // and the layout keeps each field naturally aligned.

    fn at<T>(&self, off: usize) -> *mut T {
        debug_assert!(off + size_of::<T>() <= self.layout.total && off.is_multiple_of(align_of::<T>()));
        // SAFETY: `off` is inside the queue memory (see above), which `new`'s
        // caller promised is one valid allocation.
        unsafe { self.base.as_ptr().add(off).cast() }
    }

    fn read16(&self, off: usize) -> u16 {
        // SAFETY: in bounds and aligned (see above); volatile because the
        // device writes this memory behind the compiler's back.
        u16::from_le(unsafe { ptr::read_volatile(self.at(off)) })
    }

    fn read32(&self, off: usize) -> u32 {
        // SAFETY: as for `read16`.
        u32::from_le(unsafe { ptr::read_volatile(self.at(off)) })
    }

    fn write16(&self, off: usize, v: u16) {
        // SAFETY: as for `read16`; the memory is ours to write.
        unsafe { ptr::write_volatile(self.at(off), v.to_le()) }
    }

    fn write32(&self, off: usize, v: u32) {
        // SAFETY: as for `write16`.
        unsafe { ptr::write_volatile(self.at(off), v.to_le()) }
    }

    fn write64(&self, off: usize, v: u64) {
        // SAFETY: as for `write16`; descriptors are 16-byte aligned.
        unsafe { ptr::write_volatile(self.at(off), v.to_le()) }
    }
}

/// Has the index moved past `event` in the step from `old` to `new`, all
/// modulo 2^16? This is the EVENT_IDX rule of §2.7.7 (interrupts) and
/// §2.7.10 (notifications): signal only when the other side's event index
/// was crossed.
const fn need_event(event: u16, new: u16, old: u16) -> bool {
    new.wrapping_sub(event).wrapping_sub(1) < new.wrapping_sub(old)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{DeviceQueue, DmaMemory};
    extern crate std;
    use std::{vec, vec::Vec};

    struct Fixture {
        mem: DmaMemory,
        state: Vec<DescState>,
        layout: Layout,
    }

    impl Fixture {
        fn new(size: u16) -> Fixture {
            let layout = Layout::new(size).unwrap();
            Fixture { mem: DmaMemory::new(layout.total()), state: vec![DescState::default(); size as usize], layout }
        }

        fn queue(&mut self, event_idx: bool) -> (SplitQueue<'_>, DeviceQueue) {
            let (ptr, phys) = (self.mem.ptr(), self.mem.phys());
            // SAFETY: the fixture's memory is big enough, lives as long as
            // the queue and is used only by the queue and the simulated
            // device.
            let q = unsafe { SplitQueue::new(ptr, phys, self.layout, &mut self.state, event_idx) }.unwrap();
            let d = DeviceQueue::new(q.addresses(), self.layout.size(), event_idx);
            (q, d)
        }
    }

    fn buf(v: &mut [u8]) -> Buffer {
        Buffer { addr: v.as_mut_ptr() as u64, len: v.len() as u32 }
    }

    #[test]
    fn layout_sizes() {
        let l = Layout::new(1).unwrap();
        assert_eq!((l.avail_offset(), l.used_offset(), l.total()), (16, 24, 38));
        let l = Layout::new(8).unwrap();
        assert_eq!((l.avail_offset(), l.used_offset(), l.total()), (128, 152, 222));
        let l = Layout::new(256).unwrap();
        assert_eq!((l.avail_offset(), l.used_offset(), l.total()), (4096, 4616, 6670));
        let l = Layout::new(32768).unwrap();
        assert_eq!(l.total(), 16 * 32768 + 65542 + 2 + 6 + 8 * 32768);
        assert_eq!(l.used_offset() % 4, 0);
        for bad in [0, 3, 100, 65535] {
            assert_eq!(Layout::new(bad), None);
        }
    }

    #[test]
    fn setup_checks() {
        let mut f = Fixture::new(4);
        let ptr = f.mem.ptr();
        let phys = f.mem.phys();
        // SAFETY: as in `Fixture::queue`; offsets stay inside the allocation
        // (which has slack) and each call fails before touching memory.
        unsafe {
            let off = NonNull::new(ptr.as_ptr().add(8)).unwrap();
            assert_eq!(SplitQueue::new(off, phys, f.layout, &mut f.state, false).err(), Some(SetupError::Misaligned));
            assert_eq!(
                SplitQueue::new(ptr, phys + 4, f.layout, &mut f.state, false).err(),
                Some(SetupError::Misaligned)
            );
            let mut short = [DescState::default(); 3];
            assert_eq!(SplitQueue::new(ptr, phys, f.layout, &mut short, false).err(), Some(SetupError::StateTooSmall));
        }
    }

    #[test]
    fn echo_round_trip() {
        let mut f = Fixture::new(8);
        let (mut q, mut d) = f.queue(false);
        let mut req = *b"hello";
        let mut resp = [0u8; 16];
        let head = q.add(&[buf(&mut req)], &[buf(&mut resp)]).unwrap();
        assert_eq!((q.num_free(), q.in_flight()), (6, 1));
        assert!(q.should_notify());
        assert!(!q.has_used());
        assert_eq!(q.pop_used(), Ok(None));
        assert_eq!(d.process(|data| data.to_ascii_uppercase()), 1);
        assert!(q.has_used());
        assert_eq!(q.pop_used(), Ok(Some(Used { head, len: 5 })));
        assert_eq!(&resp[..5], b"HELLO");
        assert_eq!((q.num_free(), q.in_flight()), (8, 0));
        assert_eq!(q.pop_used(), Ok(None));
    }

    #[test]
    fn chains_keep_order_and_flags() {
        let mut f = Fixture::new(8);
        let (mut q, mut d) = f.queue(false);
        let (mut a, mut b, mut c) = (*b"ab", *b"cde", *b"f");
        let (mut w1, mut w2) = ([0u8; 2], [0u8; 10]);
        let head = q.add(&[buf(&mut a), buf(&mut b), buf(&mut c)], &[buf(&mut w1), buf(&mut w2)]).unwrap();
        let chain = d.peek_chain().unwrap();
        assert_eq!(chain.head, head);
        let flags: Vec<u16> = chain.descs.iter().map(|d| d.flags).collect();
        assert_eq!(flags, [1, 1, 1, 3, 2]);
        assert_eq!(d.process(|data| data.iter().rev().copied().collect()), 1);
        assert_eq!(q.pop_used(), Ok(Some(Used { head, len: 6 })));
        assert_eq!((&w1, &w2[..4]), (b"fe", &b"dcba"[..]));
        assert_eq!(q.num_free(), 8);
    }

    #[test]
    fn device_only_and_driver_only_buffers() {
        let mut f = Fixture::new(4);
        let (mut q, mut d) = f.queue(false);
        let mut out = *b"log line";
        let mut inp = [0u8; 4];
        let h1 = q.add(&[buf(&mut out)], &[]).unwrap();
        let h2 = q.add(&[], &[buf(&mut inp)]).unwrap();
        let mut seen = Vec::new();
        d.process(|data| {
            seen.push(data.to_vec());
            b"rand".to_vec()
        });
        assert_eq!(seen, [b"log line".to_vec(), Vec::new()]);
        assert_eq!(q.pop_used(), Ok(Some(Used { head: h1, len: 0 })));
        assert_eq!(q.pop_used(), Ok(Some(Used { head: h2, len: 4 })));
        assert_eq!(&inp, b"rand");
    }

    #[test]
    fn full_and_too_long() {
        let mut f = Fixture::new(4);
        let (mut q, mut d) = f.queue(false);
        let mut bufs = [[0u8; 4]; 6];
        let [b0, b1, b2, b3, b4, b5] = &mut bufs;
        assert_eq!(q.add(&[], &[]), Err(AddError::NoBuffers));
        let five = [buf(b0), buf(b1), buf(b2), buf(b3), buf(b4)];
        assert_eq!(q.add(&five, &[]), Err(AddError::TooLong));
        let first = q.add(&[buf(b0)], &[buf(b1), buf(b2)]).unwrap();
        q.add(&[buf(b3)], &[]).unwrap();
        assert_eq!(q.num_free(), 0);
        assert_eq!(q.add(&[buf(b4)], &[]), Err(AddError::Full));
        // Finish only the first buffer: three descriptors come back.
        d.process_one(|_| vec![7; 8]);
        assert_eq!(q.pop_used(), Ok(Some(Used { head: first, len: 8 })));
        assert_eq!(q.num_free(), 3);
        assert_eq!(q.add(&[buf(b4)], &[buf(b5), buf(b0), buf(b1)]), Err(AddError::Full));
        q.add(&[buf(b4)], &[buf(b5), buf(b0)]).unwrap();
        assert_eq!((q.num_free(), q.in_flight()), (0, 2));
    }

    #[test]
    fn out_of_order_completion_returns_every_descriptor() {
        let mut f = Fixture::new(8);
        let (mut q, mut d) = f.queue(false);
        let mut data = [[0u8; 1]; 8];
        let [b0, b1, b2, b3, b4, b5, b6, b7] = &mut data;
        let h = [
            q.add(&[buf(b0)], &[]).unwrap(),
            q.add(&[buf(b1), buf(b2)], &[buf(b3)]).unwrap(),
            q.add(&[buf(b4)], &[buf(b5), buf(b6)]).unwrap(),
            q.add(&[buf(b7)], &[]).unwrap(),
        ];
        assert_eq!(q.num_free(), 0);
        let chains: Vec<_> = (0..4).map(|_| d.take_chain().unwrap()).collect();
        for c in chains.iter().rev() {
            d.complete(c.head, 0);
        }
        let mut back = Vec::new();
        while let Some(u) = q.pop_used().unwrap() {
            back.push(u.head);
        }
        assert_eq!(back, [h[3], h[2], h[1], h[0]]);
        assert_eq!((q.num_free(), q.in_flight()), (8, 0));
        // The free list is whole again: a chain of all 8 fits.
        let all: Vec<Buffer> = data.iter_mut().map(|b| buf(b)).collect();
        q.add(&all, &[]).unwrap();
        assert_eq!(q.num_free(), 0);
    }

    #[test]
    fn wraps_around_many_times() {
        // Well past 2^16 buffers through a queue of 8, in batches of varying
        // size and chain length (up to a completely full queue), so both
        // indices wrap and every descriptor is reused many times.
        let mut f = Fixture::new(8);
        let (mut q, mut d) = f.queue(false);
        let mut done = 0u32;
        let mut round = 0u32;
        while done < 70_000 {
            let (batch, split) = match round % 4 {
                0 => (1, true),
                1 => (4, false),
                2 => (2, true),
                _ => (3, false),
            };
            let mut reqs: Vec<[u8; 4]> = (0..batch).map(|i| (done + i).to_le_bytes()).collect();
            let mut resps = vec![[0u8; 4]; batch as usize];
            let mut heads = Vec::new();
            for (req, resp) in reqs.iter_mut().zip(resps.iter_mut()) {
                let (r0, r1) = req.split_at_mut(2);
                heads.push(if split {
                    q.add(&[buf(r0), buf(r1)], &[buf(resp)]).unwrap()
                } else {
                    q.add(&[buf(req)], &[buf(resp)]).unwrap()
                });
            }
            assert_eq!(d.process(|data| data.iter().map(|b| !b).collect()), batch as usize);
            for (i, head) in heads.iter().enumerate() {
                assert_eq!(q.pop_used(), Ok(Some(Used { head: *head, len: 4 })));
                assert_eq!(u32::from_le_bytes(resps[i].map(|b| !b)), done + i as u32);
            }
            assert_eq!(q.num_free(), 8);
            done += batch;
            round += 1;
        }
        assert_eq!(d.avail_idx(), done as u16);
        assert_eq!(q.pop_used(), Ok(None));
    }

    #[test]
    fn bad_device_is_caught() {
        let mut f = Fixture::new(4);
        let (mut q, mut d) = f.queue(false);
        let mut b = [0u8; 4];
        let head = q.add(&[], &[buf(&mut b)]).unwrap();
        // An id beyond the queue.
        d.complete(9, 0);
        assert_eq!(q.pop_used(), Err(UsedError::BadId(9)));
        // An id that is not a head in flight.
        d.rewrite_last(head + 1, 0);
        assert_eq!(q.pop_used(), Err(UsedError::BadId(head as u32 + 1)));
        // The right id, with a length beyond the buffer: clamped.
        d.rewrite_last(head, 1000);
        assert_eq!(q.pop_used(), Ok(Some(Used { head, len: 4 })));
        // Handing the same buffer back twice.
        d.complete(head, 0);
        assert_eq!(q.pop_used(), Err(UsedError::IndexJump));
        let mut f = Fixture::new(4);
        let (mut q, mut d) = f.queue(false);
        q.add(&[], &[buf(&mut b)]).unwrap();
        d.set_used_idx(3);
        assert_eq!(q.pop_used(), Err(UsedError::IndexJump));
    }

    #[test]
    fn notification_suppression_by_flag() {
        let mut f = Fixture::new(4);
        let (mut q, d) = f.queue(false);
        let mut b = [0u8; 1];
        assert!(!q.should_notify());
        q.add(&[buf(&mut b)], &[]).unwrap();
        assert!(q.should_notify());
        assert!(!q.should_notify());
        d.set_no_notify(true);
        q.add(&[buf(&mut b)], &[]).unwrap();
        assert!(!q.should_notify());
        d.set_no_notify(false);
        q.add(&[buf(&mut b)], &[]).unwrap();
        assert!(q.should_notify());
    }

    #[test]
    fn notification_suppression_by_event_idx() {
        let mut f = Fixture::new(8);
        let (mut q, d) = f.queue(true);
        let mut b = [0u8; 1];
        // The device wants to hear when the entry at index 2 is added.
        d.set_avail_event(2);
        q.add(&[buf(&mut b)], &[]).unwrap();
        q.add(&[buf(&mut b)], &[]).unwrap();
        assert!(!q.should_notify(), "indices 0 and 1 do not cross 2");
        q.add(&[buf(&mut b)], &[]).unwrap();
        q.add(&[buf(&mut b)], &[]).unwrap();
        assert!(q.should_notify(), "index 2 was added");
        q.add(&[buf(&mut b)], &[]).unwrap();
        assert!(!q.should_notify(), "already past the event");
        // The flag means nothing once EVENT_IDX is negotiated.
        d.set_no_notify(true);
        d.set_avail_event(5);
        q.add(&[buf(&mut b)], &[]).unwrap();
        assert!(q.should_notify());
    }

    #[test]
    fn need_event_wraps() {
        assert!(need_event(0xffff, 0x0001, 0xfffe));
        assert!(need_event(0x0000, 0x0001, 0xfffe));
        assert!(!need_event(0x0001, 0x0001, 0xfffe));
        assert!(!need_event(0xfffd, 0x0001, 0xfffe));
        assert!(need_event(5, 6, 5));
        assert!(!need_event(6, 6, 5));
    }

    #[test]
    fn interrupt_suppression_by_flag() {
        let mut f = Fixture::new(4);
        let (mut q, mut d) = f.queue(false);
        let mut b = [0u8; 1];
        assert!(!d.no_interrupt());
        q.disable_interrupts();
        assert!(d.no_interrupt());
        q.add(&[buf(&mut b)], &[]).unwrap();
        d.process(|_| vec![]);
        // Re-enabling reports the buffer that arrived meanwhile.
        assert!(q.enable_interrupts());
        assert!(!d.no_interrupt());
        q.pop_used().unwrap().unwrap();
        assert!(!q.enable_interrupts());
    }

    #[test]
    fn interrupt_suppression_by_event_idx() {
        let mut f = Fixture::new(4);
        let (mut q, mut d) = f.queue(true);
        let mut b = [0u8; 1];
        for _ in 0..3 {
            q.add(&[buf(&mut b)], &[]).unwrap();
        }
        d.process_one(|_| vec![]);
        q.pop_used().unwrap().unwrap();
        assert_eq!(d.used_event(), 1, "interrupt at the next completion");
        q.disable_interrupts();
        d.process_one(|_| vec![]);
        q.pop_used().unwrap().unwrap();
        assert_eq!(d.used_event(), 1, "left alone while disabled");
        d.process_one(|_| vec![]);
        assert!(q.enable_interrupts());
        assert_eq!(d.used_event(), 2);
        q.pop_used().unwrap().unwrap();
        assert_eq!(d.used_event(), 3);
        assert!(!q.enable_interrupts());
    }
}
