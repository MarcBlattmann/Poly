//! Locking memory and asking what is resident (ADR 0064): mlock(2),
//! mlock2(2), munlock(2), mlockall(2), munlockall(2) and mincore(2).
//!
//! Poly never pages memory out, so every page that is in stays in: a lock
//! has nothing to keep, and mlock only brings the pages of the range in
//! (mlock(2): "all pages that contain a part of the specified address
//! range are guaranteed to be resident in RAM when the call returns
//! successfully"). The locks themselves are not recorded, so nothing is
//! inherited by a fork(2) child (as the man page says of locks) and
//! munlock never finds a lock missing. Every process runs with
//! `CAP_IPC_LOCK` (root), so `RLIMIT_MEMLOCK` does not apply.
//!
//! - mlock / mlock2 / munlock: `addr` is rounded down and the end up to
//!   pages; a range that wraps around the address space is `EINVAL`; a
//!   length of 0 succeeds; unmapped pages are `ENOMEM` (kernel addresses
//!   too), and for a lock so are `PROT_NONE` pages. mlock2's only flag is
//!   `MLOCK_ONFAULT` (`EINVAL` for others).
//! - mlockall: flags `MCL_CURRENT`, `MCL_FUTURE`, `MCL_ONFAULT`; none,
//!   unknown bits, or `MCL_ONFAULT` alone are `EINVAL`. munlockall: 0.
//! - mincore: `addr` must be page aligned (`EINVAL`); one byte per page of
//!   `[addr, addr + length)` rounded up, bit 0 for resident; unmapped
//!   pages are `ENOMEM`; a `vec` that cannot be written is `EFAULT`; bytes
//!   past the last page are not touched.
//!
//! Constants are from `asm-generic/mman-common.h` and `asm-generic/mman.h`.

use crate::abi::{PAGE_SIZE, page_aligned, page_round_up};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

pub const MLOCK_ONFAULT: u32 = 0x01;
pub const MCL_CURRENT: u32 = 1;
pub const MCL_FUTURE: u32 = 2;
pub const MCL_ONFAULT: u32 = 4;

/// Pages mincore asks the kernel about at a time.
const MINCORE_CHUNK: usize = 512;

/// The page range of `[addr, addr + length)`: start and length, both page
/// aligned. `EINVAL` if it wraps around.
fn page_range(addr: u64, length: u64) -> Result<(u64, u64), Errno> {
    let start = addr & !(PAGE_SIZE - 1);
    let end = addr.checked_add(length).and_then(page_round_up).ok_or(Errno::EINVAL)?;
    Ok((start, end - start))
}

fn lock_range(k: &mut dyn LinuxKernel, addr: u64, length: u64, lock: bool) -> SysResult {
    let (start, len) = page_range(addr, length)?;
    if length == 0 || len == 0 {
        return Ok(0);
    }
    k.mlock(start, len, lock).map(|()| 0)
}

/// mlock(addr, len)
pub fn mlock(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    lock_range(k, a.ulong(0), a.ulong(1), true)
}

/// mlock2(addr, len, flags)
pub fn mlock2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.uint(2) & !MLOCK_ONFAULT != 0 {
        return Err(Errno::EINVAL);
    }
    lock_range(k, a.ulong(0), a.ulong(1), true)
}

/// munlock(addr, len)
pub fn munlock(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    lock_range(k, a.ulong(0), a.ulong(1), false)
}

/// mlockall(flags)
pub fn mlockall(_: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(0);
    let known = MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT;
    if flags == 0 || flags & !known != 0 || flags & (MCL_CURRENT | MCL_FUTURE) == 0 {
        return Err(Errno::EINVAL);
    }
    Ok(0)
}

/// munlockall()
pub fn munlockall(_: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    Ok(0)
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
