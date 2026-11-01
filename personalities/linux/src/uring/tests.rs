//! Host tests of the io_uring bookkeeping: layout, parameters, the SQ
//! and CQ rings, overflow, the probe, link chains.

use super::abi::{self, Cqe, Layout, Params, Sqe, op, setup, sq_flags, sqe_flags};
use super::exec::breaks;
use super::{Instance, Pending, RingMemory, Wait};
use crate::abi::Timespec;
use crate::kernel::{Clock, Deadline};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;
use core::sync::atomic::{AtomicU8, Ordering};

/// Ring memory in a plain vector, as the host tests' kernel.
#[derive(Debug)]
struct VecMemory(Vec<AtomicU8>);

impl VecMemory {
    fn shared(len: usize) -> Arc<dyn RingMemory> {
        Arc::new(VecMemory((0..len).map(|_| AtomicU8::new(0)).collect()))
    }
}

impl RingMemory for VecMemory {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn read(&self, offset: usize, buf: &mut [u8]) {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.0[offset + i].load(Ordering::Relaxed);
        }
    }
    fn write(&self, offset: usize, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.0[offset + i].store(*b, Ordering::Relaxed);
        }
    }
    fn load_acquire(&self, offset: usize) -> u32 {
        let mut b = [0; 4];
        self.read(offset, &mut b);
        u32::from_le_bytes(b)
    }
    fn store_release(&self, offset: usize, value: u32) {
        self.write(offset, &value.to_le_bytes());
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn params(flags: u32, cq: u32) -> Params {
    Params { flags, cq_entries: cq, ..Params::default() }
}

fn ring(entries: u32) -> (Instance, Arc<dyn RingMemory>, Arc<dyn RingMemory>) {
    let l = Layout::for_setup(entries, &params(0, 0)).unwrap();
    let rings = VecMemory::shared(l.rings_len());
    let sqes = VecMemory::shared(l.sqes_len());
    (Instance::new(l, 0, rings.clone(), sqes.clone()), rings, sqes)
}

/// Queue `sqe` at the user's SQ tail the way liburing does.
fn queue(rings: &Arc<dyn RingMemory>, sqes: &Arc<dyn RingMemory>, l: &Layout, slot_value: Option<u32>, sqe: &Sqe) {
    let tail = rings.load_acquire(Layout::SQ_TAIL as usize);
    let slot = tail & (l.sq_entries - 1);
    let mut raw = [0u8; 64];
    raw[0] = sqe.opcode;
    raw[1] = sqe.flags;
    raw[4..8].copy_from_slice(&sqe.fd.to_le_bytes());
    raw[32..40].copy_from_slice(&sqe.user_data.to_le_bytes());
    sqes.write(slot as usize * 64, &raw);
    rings.store_release(l.array.unwrap() as usize + 4 * slot as usize, slot_value.unwrap_or(slot));
    rings.store_release(Layout::SQ_TAIL as usize, tail + 1);
}

fn pop(rings: &Arc<dyn RingMemory>, l: &Layout) -> Option<Cqe> {
    let head = rings.load_acquire(Layout::CQ_HEAD as usize);
    if head == rings.load_acquire(Layout::CQ_TAIL as usize) {
        return None;
    }
    let mut b = [0u8; 16];
    rings.read(Layout::CQES as usize + 16 * (head & (l.cq_entries - 1)) as usize, &mut b);
    rings.store_release(Layout::CQ_HEAD as usize, head + 1);
    Some(Cqe {
        user_data: u64::from_le_bytes(b[..8].try_into().unwrap()),
        res: i32::from_le_bytes(b[8..12].try_into().unwrap()),
        flags: u32::from_le_bytes(b[12..].try_into().unwrap()),
    })
}

#[test]
fn sizes_round_up_and_clamp() {
    let l = |e, p: Params| Layout::for_setup(e, &p).map(|l| (l.sq_entries, l.cq_entries));
    assert_eq!(l(1, params(0, 0)), Ok((1, 2)));
    assert_eq!(l(4, params(0, 99)), Ok((4, 8)), "cq_entries ignored without CQSIZE");
    assert_eq!(l(5, params(0, 0)), Ok((8, 16)));
    assert_eq!(l(32768, params(0, 0)), Ok((32768, 65536)));
    assert_eq!(l(0, params(0, 0)), Err(crate::errno::Errno::EINVAL));
    assert_eq!(l(32769, params(0, 0)), Err(crate::errno::Errno::EINVAL));
    assert_eq!(l(40000, params(setup::CLAMP, 0)), Ok((32768, 65536)));
    assert_eq!(l(4, params(setup::CQSIZE, 2)), Err(crate::errno::Errno::EINVAL), "below sq_entries");
    assert_eq!(l(4, params(setup::CQSIZE, 3)), Ok((4, 4)), "rounded up first");
    assert_eq!(l(2, params(setup::CQSIZE, 3)), Ok((2, 4)));
    assert_eq!(l(4, params(setup::CQSIZE, 100)), Ok((4, 128)));
    assert_eq!(l(4, params(setup::CQSIZE, 0)), Err(crate::errno::Errno::EINVAL));
    assert_eq!(l(4, params(setup::CQSIZE, 65537)), Err(crate::errno::Errno::EINVAL));
    assert_eq!(l(4, params(setup::CQSIZE | setup::CLAMP, 70000)), Ok((4, 65536)));
}

#[test]
fn offsets_match_the_observed_layout() {
    let l = Layout::for_setup(4, &params(0, 0)).unwrap();
    let out = Params::result(&[0; Params::SIZE], &l);
    let w = |i: usize| u32::from_le_bytes(out[i..i + 4].try_into().unwrap());
    // sq_off: head 0, tail 4, ring_mask 16, ring_entries 24, flags 36,
    // dropped 32, array 192 (io_uring_setup's spec sheet, 4 entries).
    assert_eq!([w(40), w(44), w(48), w(52), w(56), w(60), w(64)], [0, 4, 16, 24, 36, 32, 192]);
    // cq_off: head 8, tail 12, ring_mask 20, ring_entries 28, overflow
    // 44, cqes 64, flags 40.
    assert_eq!([w(80), w(84), w(88), w(92), w(96), w(100), w(104)], [8, 12, 20, 28, 44, 64, 40]);
    assert_eq!((w(0), w(4), w(20)), (4, 8, abi::feat::ALL));
    assert_eq!(l.rings_len(), 192 + 16);
    assert_eq!(l.sqes_len(), 256);
    let n = Layout::for_setup(4, &params(setup::NO_SQARRAY, 0)).unwrap();
    assert_eq!(n.array, None);
    assert_eq!(u32::from_le_bytes(Params::result(&[0; 120], &n)[64..68].try_into().unwrap()), 0);
}

#[test]
fn new_ring_shows_sizes_and_starts_empty() {
    let (inst, rings, _) = ring(4);
    let w = |o: u32| rings.load_acquire(o as usize);
    assert_eq!((w(Layout::SQ_ENTRIES), w(Layout::SQ_MASK)), (4, 3));
    assert_eq!((w(Layout::CQ_ENTRIES), w(Layout::CQ_MASK)), (8, 7));
    assert_eq!((w(Layout::SQ_HEAD), w(Layout::CQ_TAIL), w(Layout::SQ_FLAGS)), (0, 0, 0));
    assert_eq!(inst.cq_ready(), 0);
    assert!(inst.region(abi::OFF_SQ_RING).is_some());
    assert!(Arc::ptr_eq(inst.region(abi::OFF_SQ_RING).unwrap(), inst.region(abi::OFF_CQ_RING).unwrap()));
    assert!(inst.region(abi::OFF_SQES).is_some());
    assert!(inst.region(0x2000_0000).is_none());
    assert!(inst.region(abi::OFF_SQES | 0x1234).is_some(), "low bits are masked off");
}

#[test]
fn take_sqes_respects_limit_and_drops_bad_indices() {
    let (inst, rings, sqes) = ring(8);
    let l = inst.layout;
    for i in 0..3 {
        queue(&rings, &sqes, &l, None, &Sqe { opcode: op::NOP, user_data: 10 + i, ..Sqe::default() });
    }
    let got = inst.take_sqes(2);
    assert_eq!(got.iter().map(|s| s.user_data).collect::<Vec<_>>(), vec![10, 11]);
    assert_eq!(rings.load_acquire(Layout::SQ_HEAD as usize), 2);
    // An SQ array value past the SQEs is dropped: consumed, not returned.
    queue(&rings, &sqes, &l, Some(99), &Sqe::default());
    let got = inst.take_sqes(10);
    assert_eq!(got.iter().map(|s| s.user_data).collect::<Vec<_>>(), vec![12]);
    assert_eq!(rings.load_acquire(Layout::SQ_DROPPED as usize), 1);
    assert_eq!(rings.load_acquire(Layout::SQ_HEAD as usize), 4);
    assert!(inst.take_sqes(10).is_empty());
}

#[test]
fn overflow_keeps_cqes_in_order_and_flags_it() {
    // 2 SQ entries, 4 CQ entries, 6 CQEs.
    let (inst, rings, _) = ring(2);
    let l = inst.layout;
    for i in 0..6 {
        inst.post(Cqe { user_data: 100 + i, res: 0, flags: 0 });
    }
    assert_eq!(inst.cq_ready(), 4);
    assert!(inst.overflowing());
    assert_ne!(rings.load_acquire(Layout::SQ_FLAGS as usize) & sq_flags::CQ_OVERFLOW, 0);
    let mut seen = Vec::new();
    while let Some(c) = pop(&rings, &l) {
        seen.push(c.user_data);
    }
    inst.flush_overflow();
    while let Some(c) = pop(&rings, &l) {
        seen.push(c.user_data);
    }
    assert_eq!(seen, (100..106).collect::<Vec<_>>());
    assert_eq!(rings.load_acquire(Layout::SQ_FLAGS as usize) & sq_flags::CQ_OVERFLOW, 0);
    assert_eq!(inst.posted(), 6);
}

#[test]
fn probe_fills_only_ops_len_entries() {
    let mut buf = vec![0u8; 16 + 8 * 256];
    abi::fill_probe(&mut buf, 256);
    assert_eq!(buf[0], op::LAST);
    assert_eq!(buf[1], op::LAST + 1);
    for o in [op::NOP, op::READV, op::TIMEOUT, op::ACCEPT, op::RECV] {
        assert_eq!(buf[16 + 8 * usize::from(o)], o);
        assert_eq!(buf[16 + 8 * usize::from(o) + 2], 1);
    }
    assert_eq!(buf[16 + 8 * 8 + 2], 0, "IORING_OP_FILES_UPDATE is not supported");
    assert!(buf[16 + 8 * (usize::from(op::LAST) + 1)..].iter().all(|&b| b == 0));
    let mut small = vec![0u8; 16 + 8 * 4];
    abi::fill_probe(&mut small, 4);
    assert_eq!((small[0], small[1]), (op::LAST, 4));
}

#[test]
fn chains_break_on_failure_and_short_io_unless_hard_linked() {
    let read = Sqe { opcode: op::READ, len: 4, flags: sqe_flags::IO_LINK, ..Sqe::default() };
    assert!(breaks(&read, -9));
    assert!(breaks(&read, 3), "a short read");
    assert!(!breaks(&read, 4));
    let nop = Sqe { opcode: op::NOP, flags: sqe_flags::IO_LINK, ..Sqe::default() };
    assert!(!breaks(&nop, 0));
    let hard = Sqe { flags: sqe_flags::IO_HARDLINK, ..read };
    assert!(!breaks(&hard, -9));
}

#[test]
fn pending_requests_are_found_by_user_data() {
    let (inst, _, _) = ring(2);
    let at = Timespec { sec: 5, nsec: 0 };
    let timeout = Wait::Timeout { deadline: Deadline { clock: Clock::Monotonic, at }, target: None };
    inst.add_pending(Pending { sqe: Sqe { user_data: 4, ..Sqe::default() }, wait: timeout, chain: Vec::new() });
    inst.add_pending(Pending {
        sqe: Sqe { user_data: 8, fd: 3, ..Sqe::default() },
        wait: Wait::Ready { events: 1 },
        chain: Vec::new(),
    });
    let t = inst.wait_targets();
    assert_eq!(t.fds, vec![(3, 1)]);
    assert_eq!(t.timeout, Some(at));
    assert!(inst.remove_pending(8, |p| matches!(p.wait, Wait::Timeout { .. })).is_none());
    assert!(inst.remove_pending(4, |p| matches!(p.wait, Wait::Timeout { .. })).is_some());
    assert!(inst.has_pending());
}

#[test]
fn registrations_are_exclusive() {
    let (inst, _, _) = ring(2);
    assert!(!inst.clear_buffers());
    assert!(inst.set_buffers(vec![(0x1000, 4096)]));
    assert!(!inst.set_buffers(vec![(0x1000, 4096)]));
    assert!(inst.fixed_buffer_ok(0, 0x1000, 10));
    assert!(!inst.fixed_buffer_ok(0, 0x1ffa, 10), "past the end of the buffer");
    assert!(!inst.fixed_buffer_ok(5, 0x1000, 10), "no such buffer");
    assert!(inst.clear_buffers());
    assert!(!inst.fixed_buffer_ok(0, 0x1000, 10));
    assert_eq!(inst.files_len(), None);
    assert!(inst.set_files(vec![None, None]));
    assert_eq!(inst.files_len(), Some(2));
    assert!(inst.file(0).is_none());
    assert!(inst.clear_files());
}
