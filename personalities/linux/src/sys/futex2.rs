//! The futex2 calls: futex_wait(2), futex_wake(2), futex_waitv(2) and
//! futex_requeue(2) (ADR 0121).
//!
//! They share the wait queues of futex(2) ([`crate::futex`]): a futex2
//! wake finds classic waiters of the same kind and word and the other way
//! round. What is new is the flags word (a size, which must be 32 bits,
//! `FUTEX2_PRIVATE`, `FUTEX2_NUMA`, `FUTEX2_MPOL`), 64-bit values and masks
//! that must fit in 32 bits, absolute deadlines on a chosen clock, and
//! waiting on several words at once.
//!
//! Poly has one NUMA node (0): with `FUTEX2_NUMA` the 32-bit word after
//! the futex holds a node, `-1` ("any") is replaced by 0 and any other
//! node but 0 is `EINVAL`. `FUTEX2_MPOL` changes nothing.

use crate::abi::Timespec;
use crate::abi::clock::{CLOCK_MONOTONIC, CLOCK_REALTIME};
use crate::abi::futex::*;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::futex::{self, FutexKey, WaitError};
use crate::kernel::{Clock, Deadline, LinuxKernel, UserPtr};
use crate::table::{Handler, nr};
use alloc::vec::Vec;

pub const CALLS: &[(u64, Handler)] = &[
    (nr::FUTEX_WAITV, futex_waitv),
    (nr::FUTEX_WAKE, futex_wake),
    (nr::FUTEX_WAIT, futex_wait),
    (nr::FUTEX_REQUEUE, futex_requeue),
];

/// Check a futex2 flags word: `FUTEX2_SIZE_U32` and only known bits.
pub fn check_flags(flags: u32) -> Result<(), Errno> {
    if flags & !FUTEX2_VALID_MASK != 0 || flags & FUTEX2_SIZE_MASK != FUTEX2_SIZE_U32 {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

/// A 64-bit value or mask argument that must fit in 32 bits.
fn narrow(v: u64) -> Result<u32, Errno> {
    u32::try_from(v).map_err(|_| Errno::EINVAL)
}

/// The futex's alignment: 4 bytes, 8 with `FUTEX2_NUMA` (the word and its
/// node word form a pair).
fn check_aligned(addr: UserPtr, flags: u32) -> Result<(), Errno> {
    let align = if flags & FUTEX2_NUMA != 0 { 8 } else { 4 };
    if addr.0.is_multiple_of(align) { Ok(()) } else { Err(Errno::EINVAL) }
}

/// With `FUTEX2_NUMA`, the node word after the futex: `-1` becomes this
/// machine's only node (0), any other node but 0 does not exist.
fn numa_node(k: &mut dyn LinuxKernel, addr: UserPtr, flags: u32) -> Result<(), Errno> {
    if flags & FUTEX2_NUMA == 0 {
        return Ok(());
    }
    let node_addr = UserPtr(addr.0 + 4);
    let mut raw = [0; 4];
    k.copy_from_user(node_addr, &mut raw)?;
    match u32::from_ne_bytes(raw) {
        u32::MAX => k.copy_to_user(node_addr, &0u32.to_ne_bytes()),
        0 => Ok(()),
        _ => Err(Errno::EINVAL),
    }
}

/// The clock of an absolute futex2 deadline: `CLOCK_MONOTONIC` or
/// `CLOCK_REALTIME`, else `EINVAL`.
fn deadline_clock(clockid: i32) -> Result<Clock, Errno> {
    match clockid {
        CLOCK_MONOTONIC => Ok(Clock::Monotonic),
        CLOCK_REALTIME => Ok(Clock::Realtime),
        _ => Err(Errno::EINVAL),
    }
}

/// Read and check an absolute timeout; `None` for a null pointer.
fn read_deadline(k: &mut dyn LinuxKernel, ptr: UserPtr, clock: Clock) -> Result<Option<Deadline>, Errno> {
    if ptr.is_null() {
        return Ok(None);
    }
    let mut raw = [0; Timespec::SIZE];
    k.copy_from_user(ptr, &mut raw)?;
    let at = Timespec::from_bytes(&raw);
    if !at.is_valid() {
        return Err(Errno::EINVAL);
    }
    Ok(Some(Deadline { clock, at }))
}

/// The key of a futex2 futex after its alignment and node checks;
/// `EFAULT` if the word cannot be read.
fn key(k: &mut dyn LinuxKernel, addr: UserPtr, flags: u32) -> Result<FutexKey, Errno> {
    check_aligned(addr, flags)?;
    numa_node(k, addr, flags)?;
    futex::key(k, addr, flags & FUTEX2_PRIVATE != 0)
}

/// After a signal: restart if the handler has `SA_RESTART` (the deadline
/// is absolute, so running the call again keeps it), else `EINTR`.
fn interrupted(e: WaitError) -> Errno {
    match e {
        WaitError::Errno(e) => e,
        WaitError::Interrupted => Errno::RESTART_IF_SA_RESTART,
    }
}

/// futex_wait(uaddr, val, mask, flags, timeout, clockid): sleep while the
/// word at `uaddr` holds `val` until a wake whose mask overlaps `mask`,
/// the absolute `timeout` on `clockid`, or a signal. Checks in the order
/// of futex_wait's spec sheet: flags, clockid (with a timeout), the
/// width of `val`, the timeout, `mask` and alignment, the address, the
/// value, the deadline.
pub fn futex_wait(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let uaddr = a.ptr(0);
    let flags = a.uint(3);
    check_flags(flags)?;
    let timeout = a.ptr(4);
    let clock = if timeout.is_null() { Clock::Monotonic } else { deadline_clock(a.int(5))? };
    let val = narrow(a.ulong(1))?;
    let deadline = read_deadline(k, timeout, clock)?;
    let mask = narrow(a.ulong(2))?;
    if mask == 0 {
        return Err(Errno::EINVAL);
    }
    let key = key(k, uaddr, flags)?;
    futex::wait(k, uaddr, key, val, mask, deadline).map(|()| 0).map_err(interrupted)
}

/// futex_wake(uaddr, mask, nr, flags): wake up to `nr` waiters (one for a
/// negative `nr`, none for 0) whose mask overlaps `mask`; returns how
/// many. A private futex is not looked up in memory.
pub fn futex_wake(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let uaddr = a.ptr(0);
    let flags = a.uint(3);
    check_flags(flags)?;
    let mask = narrow(a.ulong(1))?;
    if mask == 0 {
        return Err(Errno::EINVAL);
    }
    check_aligned(uaddr, flags)?;
    numa_node(k, uaddr, flags)?;
    let key = if flags & FUTEX2_PRIVATE != 0 { k.futex_key(uaddr, true)? } else { futex::key(k, uaddr, false)? };
    let nr = match a.int(2) {
        n if n < 0 => 1,
        n => n as usize,
    };
    Ok(futex::wake(k, key, nr, mask) as u64)
}

/// One `struct futex_waitv` as read from user memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitvEntry {
    pub val: u64,
    pub uaddr: u64,
    pub flags: u32,
    pub reserved: u32,
}

impl WaitvEntry {
    pub fn from_bytes(b: &[u8]) -> WaitvEntry {
        let u64_at = |i: usize| u64::from_ne_bytes(b[i..i + 8].try_into().expect("8 bytes"));
        let u32_at = |i: usize| u32::from_ne_bytes(b[i..i + 4].try_into().expect("4 bytes"));
        WaitvEntry { val: u64_at(0), uaddr: u64_at(8), flags: u32_at(16), reserved: u32_at(20) }
    }

    /// The fields' checks: flags, `__reserved` 0, a 32-bit value, an
    /// aligned address.
    pub fn check(&self) -> Result<u32, Errno> {
        check_flags(self.flags)?;
        if self.reserved != 0 {
            return Err(Errno::EINVAL);
        }
        let val = narrow(self.val)?;
        check_aligned(UserPtr(self.uaddr), self.flags)?;
        Ok(val)
    }
}

/// Read `n` `struct futex_waitv` entries at `ptr`.
fn read_entries(k: &mut dyn LinuxKernel, ptr: UserPtr, n: usize) -> Result<Vec<WaitvEntry>, Errno> {
    let mut raw = alloc::vec![0; n * FUTEX_WAITV_SIZE];
    k.copy_from_user(ptr, &mut raw)?;
    Ok(raw.chunks_exact(FUTEX_WAITV_SIZE).map(WaitvEntry::from_bytes).collect())
}

/// futex_waitv(waiters, nr_futexes, flags, timeout, clockid): sleep on
/// every futex of the array until one is woken; returns its index. Checks
/// in the order of futex_waitv's spec sheet: the call's flags, count and
/// array pointer, clockid, the timeout, the array read, every entry's
/// fields, every entry's address, the values, the deadline.
pub fn futex_waitv(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let array = a.ptr(0);
    let n = a.uint(1);
    if a.uint(2) != 0 || n == 0 || n > FUTEX_WAITV_MAX || array.is_null() {
        return Err(Errno::EINVAL);
    }
    let timeout = a.ptr(3);
    let clock = if timeout.is_null() { Clock::Monotonic } else { deadline_clock(a.int(4))? };
    let deadline = read_deadline(k, timeout, clock)?;
    let entries = read_entries(k, array, n as usize)?;
    let vals = entries.iter().map(WaitvEntry::check).collect::<Result<Vec<u32>, Errno>>()?;
    let mut words = Vec::with_capacity(entries.len());
    for (e, val) in entries.iter().zip(vals) {
        let addr = UserPtr(e.uaddr);
        numa_node(k, addr, e.flags)?;
        words.push((addr, futex::key(k, addr, e.flags & FUTEX2_PRIVATE != 0)?, val));
    }
    futex::wait_many(k, &words, deadline).map(u64::from).map_err(interrupted)
}

/// futex_requeue(waiters, flags, nr_wake, nr_requeue): if the word of
/// `waiters[0]` holds its value, wake up to `nr_wake` of its waiters and
/// move up to `nr_requeue` more to the futex of `waiters[1]`; returns
/// woken plus moved. Both entries must be of the same kind (private,
/// NUMA, MPOL).
pub fn futex_requeue(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let array = a.ptr(0);
    let nr_wake = a.int(2);
    let nr_requeue = a.int(3);
    if a.uint(1) != 0 || nr_wake < 0 || nr_requeue < 0 || array.is_null() {
        return Err(Errno::EINVAL);
    }
    let entries = read_entries(k, array, 2)?;
    let (src, dst) = (entries[0], entries[1]);
    let val = src.check()?;
    dst.check()?;
    let kind = FUTEX2_PRIVATE | FUTEX2_NUMA | FUTEX2_MPOL;
    if src.flags & kind != dst.flags & kind {
        return Err(Errno::EINVAL);
    }
    let private = src.flags & FUTEX2_PRIVATE != 0;
    let (from_addr, to_addr) = (UserPtr(src.uaddr), UserPtr(dst.uaddr));
    numa_node(k, from_addr, src.flags)?;
    let from = futex::key(k, from_addr, private)?;
    numa_node(k, to_addr, dst.flags)?;
    let to = futex::key(k, to_addr, private)?;
    let (woken, moved) = futex::requeue(k, from_addr, from, to, nr_wake as usize, nr_requeue as usize, Some(val))?;
    Ok((woken + moved) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_need_a_32_bit_size() {
        assert_eq!(check_flags(FUTEX2_SIZE_U32), Ok(()));
        assert_eq!(check_flags(FUTEX2_SIZE_U32 | FUTEX2_PRIVATE | FUTEX2_NUMA | FUTEX2_MPOL), Ok(()));
        for bad in [FUTEX2_SIZE_U8, FUTEX2_SIZE_U16, FUTEX2_SIZE_U64, FUTEX2_SIZE_U32 | 0x10, FUTEX2_SIZE_U32 | 0x100] {
            assert_eq!(check_flags(bad), Err(Errno::EINVAL), "{bad:#x}");
        }
    }

    #[test]
    fn waitv_entries_decode_and_check() {
        let mut b = [0u8; 24];
        b[0..8].copy_from_slice(&7u64.to_ne_bytes());
        b[8..16].copy_from_slice(&0x1000u64.to_ne_bytes());
        b[16..20].copy_from_slice(&(FUTEX2_SIZE_U32 | FUTEX2_PRIVATE).to_ne_bytes());
        let e = WaitvEntry::from_bytes(&b);
        assert_eq!(e, WaitvEntry { val: 7, uaddr: 0x1000, flags: 0x82, reserved: 0 });
        assert_eq!(e.check(), Ok(7));
        assert_eq!(WaitvEntry { reserved: 1, ..e }.check(), Err(Errno::EINVAL));
        assert_eq!(WaitvEntry { val: 1 << 32, ..e }.check(), Err(Errno::EINVAL));
        assert_eq!(WaitvEntry { uaddr: 0x1002, ..e }.check(), Err(Errno::EINVAL));
        assert_eq!(
            WaitvEntry { uaddr: 0x1004, flags: 0x86, ..e }.check(),
            Err(Errno::EINVAL),
            "NUMA pairs are 8-aligned"
        );
    }

    #[test]
    fn deadlines_only_on_two_clocks() {
        assert_eq!(deadline_clock(0), Ok(Clock::Realtime));
        assert_eq!(deadline_clock(1), Ok(Clock::Monotonic));
        for bad in [2, 7, 11, -1] {
            assert_eq!(deadline_clock(bad), Err(Errno::EINVAL));
        }
    }
}
