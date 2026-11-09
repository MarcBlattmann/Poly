//! Locking memory and asking what is resident (ADR 0064, ADR 0121):
//! mlock(2), mlock2(2), munlock(2), mlockall(2), munlockall(2) and
//! mincore(2).
//!
//! Poly never pages memory out, so every page that is in stays in: a lock
//! keeps nothing in that would otherwise go. Locks are still recorded per
//! region of the address space (the kernel's region map), because they are
//! visible: `VmLck` in `/proc/<pid>/status`, the `RLIMIT_MEMLOCK`
//! accounting, mlockall(2)'s `MCL_FUTURE`, and a fork(2) child that does
//! not inherit them. Locking without `MLOCK_ONFAULT` / `MCL_ONFAULT` brings
//! the pages in at once (mlock(2): "all pages that contain a part of the
//! specified address range are guaranteed to be resident in RAM when the
//! call returns successfully"); with it nothing is brought in.
//!
//! - mlock / mlock2 / munlock: `addr` is rounded down to a page and the
//!   length, plus `addr`'s offset in its page, up to pages; a length that
//!   rounds to 0 succeeds without looking at the range, one whose end
//!   wraps around is `EINVAL`; unmapped pages are `ENOMEM` (regions before
//!   the hole stay locked), and so are `PROT_NONE` pages for a lock that
//!   populates. mlock2's only flag is `MLOCK_ONFAULT` (`EINVAL` for
//!   others, checked first).
//! - Without `CAP_IPC_LOCK` the soft `RLIMIT_MEMLOCK` applies ([`limit`]):
//!   0 refuses every lock (`EPERM`), else the pages already locked plus
//!   the newly locked ones may not exceed it (`ENOMEM` from mlock,
//!   mlockall(`MCL_CURRENT`) when the whole address space is larger,
//!   `EAGAIN` from an mmap under `MCL_FUTURE`: [`future_mmap_check`]).
//! - mlockall: flags `MCL_CURRENT`, `MCL_FUTURE`, `MCL_ONFAULT`; none,
//!   unknown bits, or `MCL_ONFAULT` alone are `EINVAL`. Each call replaces
//!   the `MCL_FUTURE` setting. munlockall: always 0.
//! - mincore: `addr` must be page aligned (`EINVAL`); one byte per page of
//!   `[addr, addr + length)` rounded up, bit 0 for resident; unmapped
//!   pages are `ENOMEM`; a `vec` that cannot be written is `EFAULT`; bytes
//!   past the last page are not touched.
//!
//! Constants are from `asm-generic/mman-common.h` and `asm-generic/mman.h`.

use crate::abi::{PAGE_SIZE, page_aligned, page_round_up};
use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::fs::rlimit::RLIMIT_MEMLOCK;
use crate::kernel::{LinuxKernel, MemLock};
use crate::signal::deliver::with_own;
use crate::table::{Handler, nr};

pub const MLOCK_ONFAULT: u32 = 0x01;
pub const MCL_CURRENT: u32 = 1;
pub const MCL_FUTURE: u32 = 2;
pub const MCL_ONFAULT: u32 = 4;

/// Pages mincore asks the kernel about at a time.
const MINCORE_CHUNK: usize = 512;

/// The pages of `[addr, addr + length)`: the start rounded down and the
/// length (with `addr`'s offset in its page) rounded up, as mlock(2) does;
/// the length wraps to 0 for lengths near `u64::MAX`.
const fn pages_of(addr: u64, length: u64) -> (u64, u64) {
    let offset = addr & (PAGE_SIZE - 1);
    let len = length.wrapping_add(offset).wrapping_add(PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    (addr - offset, len)
}

/// How many bytes the caller may have locked: `None` with `CAP_IPC_LOCK`
/// (no limit), else the soft `RLIMIT_MEMLOCK` in whole pages; `EPERM` if
/// that is 0 (mlock(2): "the caller is not privileged ... and its
/// RLIMIT_MEMLOCK soft resource limit was 0").
pub fn limit(k: &mut dyn LinuxKernel) -> Result<Option<u64>, Errno> {
    if cred::current(k).capable(cap::IPC_LOCK) {
        return Ok(None);
    }
    let soft = with_own(k, |p| p.limits.get(RLIMIT_MEMLOCK)).map_or(u64::MAX, |l| l.cur);
    let pages = soft / PAGE_SIZE;
    if pages == 0 { Err(Errno::EPERM) } else { Ok(Some(pages * PAGE_SIZE)) }
}

/// Whether locking `new` more bytes stays within `limit` given `locked`.
const fn within(limit: Option<u64>, locked: u64, new: u64) -> bool {
    match limit {
        None => true,
        Some(max) => match locked.checked_add(new) {
            Some(total) => total <= max,
            None => false,
        },
    }
}

/// mlock(2) and mlock2(2) after the flags: the limit, then the range.
fn lock(k: &mut dyn LinuxKernel, addr: u64, length: u64, how: MemLock) -> SysResult {
    let limit = limit(k)?;
    let (start, len) = pages_of(addr, length);
    let end = start.checked_add(len);
    if limit.is_some() {
        let held = k.locked_memory(start, if end.is_some() { len } else { 0 });
        if !within(limit, held.locked, len - held.in_range) {
            return Err(Errno::ENOMEM);
        }
    }
    if len == 0 {
        return Ok(0);
    }
    if end.is_none() {
        return Err(Errno::EINVAL);
    }
    k.mlock(start, len, how).map(|()| 0)
}

/// mlock(addr, len)
pub fn mlock(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    lock(k, a.ulong(0), a.ulong(1), MemLock::Lock)
}

/// mlock2(addr, len, flags): `flags` is a 32-bit int.
pub fn mlock2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(2);
    if flags & !MLOCK_ONFAULT != 0 {
        return Err(Errno::EINVAL);
    }
    let how = if flags & MLOCK_ONFAULT != 0 { MemLock::LockOnFault } else { MemLock::Lock };
    lock(k, a.ulong(0), a.ulong(1), how)
}

/// munlock(addr, len): no privilege or limit involved.
pub fn munlock(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (start, len) = pages_of(a.ulong(0), a.ulong(1));
    if len == 0 {
        return Ok(0);
    }
    start.checked_add(len).ok_or(Errno::EINVAL)?;
    k.mlock(start, len, MemLock::Unlock).map(|()| 0)
}

/// mlockall(flags): `flags` is a 32-bit int. In order `EINVAL` (flags),
/// `EPERM` (limit 0), `ENOMEM` (`MCL_CURRENT` and an address space larger
/// than the limit; nothing changes).
pub fn mlockall(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(0);
    let known = MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT;
    if flags & !known != 0 || flags & (MCL_CURRENT | MCL_FUTURE) == 0 {
        return Err(Errno::EINVAL);
    }
    let limit = limit(k)?;
    let how = if flags & MCL_ONFAULT != 0 { MemLock::LockOnFault } else { MemLock::Lock };
    let current = (flags & MCL_CURRENT != 0).then_some(how);
    if current.is_some() && !within(limit, 0, k.locked_memory(0, 0).mapped) {
        return Err(Errno::ENOMEM);
    }
    let future = (flags & MCL_FUTURE != 0).then_some(how);
    k.mlockall(current, future).map(|()| 0)
}

/// munlockall(): always 0; the argument registers are ignored.
pub fn munlockall(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    k.munlockall().map(|()| 0)
}

/// Before an mmap of `len` bytes: under mlockall(`MCL_FUTURE`) the new
/// mapping will be locked, so without `CAP_IPC_LOCK` it must fit in
/// `RLIMIT_MEMLOCK` with what is locked already (`EAGAIN`, mlockall's spec
/// sheet). Returns how the new mapping will be locked, if at all.
pub fn future_mmap_check(k: &mut dyn LinuxKernel, len: u64) -> Result<Option<MemLock>, Errno> {
    let held = k.locked_memory(0, 0);
    let Some(how) = held.future else { return Ok(None) };
    let limit = limit(k).map_err(|_| Errno::EAGAIN)?;
    if !within(limit, held.locked, len) {
        return Err(Errno::EAGAIN);
    }
    Ok(Some(how))
}

/// mincore(addr, length, vec)
pub fn mincore(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(0);
    let length = a.ulong(1);
    let vec = a.ptr(2);
    if !page_aligned(addr) {
        return Err(Errno::EINVAL);
    }
    let len = page_round_up(length).ok_or(Errno::ENOMEM)?;
    addr.checked_add(len).ok_or(Errno::ENOMEM)?;
    let pages = len / PAGE_SIZE;
    let mut buf = [0u8; MINCORE_CHUNK];
    let mut done = 0u64;
    while done < pages {
        let n = (pages - done).min(MINCORE_CHUNK as u64) as usize;
        k.mincore(addr + done * PAGE_SIZE, &mut buf[..n])?;
        k.copy_to_user(vec.offset(done).ok_or(Errno::EFAULT)?, &buf[..n])?;
        done += n as u64;
    }
    Ok(0)
}

/// The calls of this module, merged into the table (`table::GROUPS`).
pub const CALLS: &[(u64, Handler)] = &[
    (nr::MLOCK, mlock),
    (nr::MLOCK2, mlock2),
    (nr::MUNLOCK, munlock),
    (nr::MLOCKALL, mlockall),
    (nr::MUNLOCKALL, munlockall),
    (nr::MINCORE, mincore),
];
