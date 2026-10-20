//! madvise(2): advice about a range of memory (ADR 0049).
//!
//! The C library's thread code gives back the pages of a finished thread's
//! stack with `MADV_DONTNEED`, and allocators do the same with freed heap
//! memory, so every dynamically linked glibc program with threads calls it.
//! The personality checks the arguments and sorts the advice into what the
//! kernel has to do ([`Advice`]):
//!
//! | Advice (`asm-generic/mman-common.h`) | Here |
//! |---|---|
//! | `MADV_DONTNEED`, `MADV_DONTNEED_LOCKED` | [`Advice::DontNeed`]: the pages go; private anonymous memory reads as zeros again, file and shared mappings read the file's (or the shared object's) contents |
//! | `MADV_FREE` | [`Advice::Free`]: private anonymous memory only (`EINVAL` otherwise); Poly frees the pages at once, which the man page allows ("the kernel can free these pages") |
//! | `MADV_REMOVE` | [`Advice::Remove`]: shared writable mappings only (`EINVAL` otherwise); no file system of Poly can free a file's range yet, so the kernel answers `EOPNOTSUPP` |
//! | `MADV_NORMAL`, `MADV_RANDOM`, `MADV_SEQUENTIAL`, `MADV_WILLNEED`, `MADV_COLD`, `MADV_PAGEOUT`, `MADV_MERGEABLE`, `MADV_UNMERGEABLE`, `MADV_HUGEPAGE`, `MADV_NOHUGEPAGE`, `MADV_DONTDUMP`, `MADV_DODUMP`, `MADV_DONTFORK`, `MADV_DOFORK` | [`Advice::Hint`]: accepted, nothing changes (the range must be mapped). `MADV_DONTFORK` is the one that changes behaviour on Linux (the child of a fork does not get the range); Poly's child still gets a copy |
//! | `MADV_WIPEONFORK`, `MADV_KEEPONFORK`, `MADV_POPULATE_READ`, `MADV_POPULATE_WRITE`, `MADV_COLLAPSE`, `MADV_HWPOISON`, `MADV_SOFT_OFFLINE`, `MADV_GUARD_INSTALL`, `MADV_GUARD_REMOVE`, unknown values | `EINVAL`, as on a kernel that predates them (the man page names the version each came with). `MADV_WIPEONFORK` must not be accepted without wiping: a forked child would keep, say, a random generator's state |
//!
//! Order of the checks (madvise(2) ERRORS; the order is ours, to be
//! confirmed by the diff runner): unknown or unsupported advice, an `addr`
//! that is not page aligned, a range that wraps around (`EINVAL`, "length
//! is negative"); a length of 0 then succeeds. The kernel applies the
//! advice to the mapped parts of the range and fails with `ENOMEM` if
//! part of it is not mapped.

use crate::abi::{page_aligned, page_round_up};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

pub const MADV_NORMAL: u32 = 0;
pub const MADV_RANDOM: u32 = 1;
pub const MADV_SEQUENTIAL: u32 = 2;
pub const MADV_WILLNEED: u32 = 3;
pub const MADV_DONTNEED: u32 = 4;
pub const MADV_FREE: u32 = 8;
pub const MADV_REMOVE: u32 = 9;
pub const MADV_DONTFORK: u32 = 10;
pub const MADV_DOFORK: u32 = 11;
pub const MADV_MERGEABLE: u32 = 12;
pub const MADV_UNMERGEABLE: u32 = 13;
pub const MADV_HUGEPAGE: u32 = 14;
pub const MADV_NOHUGEPAGE: u32 = 15;
pub const MADV_DONTDUMP: u32 = 16;
pub const MADV_DODUMP: u32 = 17;
pub const MADV_WIPEONFORK: u32 = 18;
pub const MADV_KEEPONFORK: u32 = 19;
pub const MADV_COLD: u32 = 20;
pub const MADV_PAGEOUT: u32 = 21;
pub const MADV_POPULATE_READ: u32 = 22;
pub const MADV_POPULATE_WRITE: u32 = 23;
pub const MADV_DONTNEED_LOCKED: u32 = 24;
pub const MADV_COLLAPSE: u32 = 25;
pub const MADV_HWPOISON: u32 = 100;
pub const MADV_SOFT_OFFLINE: u32 = 101;

/// What the kernel does for a madvise(2) call (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advice {
    /// Nothing, once the range is known to be mapped.
    Hint,
    /// Drop the pages: the next access finds zeros (private anonymous
    /// memory) or the backing object's contents.
    DontNeed,
    /// Like [`DontNeed`](Self::DontNeed), for private anonymous memory
    /// only (`EINVAL` for any other part of the range).
    Free,
    /// Free the backing store of a shared writable mapping (`EINVAL` for
    /// anything else).
    Remove,
}

impl Advice {
    /// The advice for `value`; `None` for values Poly refuses with
    /// `EINVAL` (unknown, or not supported).
    pub const fn from_raw(value: u32) -> Option<Advice> {
        match value {
            MADV_DONTNEED | MADV_DONTNEED_LOCKED => Some(Advice::DontNeed),
            MADV_FREE => Some(Advice::Free),
            MADV_REMOVE => Some(Advice::Remove),
            MADV_NORMAL | MADV_RANDOM | MADV_SEQUENTIAL | MADV_WILLNEED | MADV_COLD | MADV_PAGEOUT | MADV_MERGEABLE
            | MADV_UNMERGEABLE | MADV_HUGEPAGE | MADV_NOHUGEPAGE | MADV_DONTDUMP | MADV_DODUMP | MADV_DONTFORK
            | MADV_DOFORK => Some(Advice::Hint),
            _ => None,
        }
    }
}

/// madvise(addr, length, advice)
pub fn madvise(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(0);
    let length = a.ulong(1);
    let advice = Advice::from_raw(a.uint(2)).ok_or(Errno::EINVAL)?;
    if !page_aligned(addr) {
        return Err(Errno::EINVAL);
    }
    let len = page_round_up(length).ok_or(Errno::EINVAL)?;
    addr.checked_add(len).ok_or(Errno::EINVAL)?;
    if len == 0 {
        return Ok(0);
    }
    k.madvise(addr, len, advice).map(|()| 0)
}

/// The calls of this module, merged into the table (`table::GROUPS`).
pub const CALLS: &[(u64, Handler)] = &[(nr::MADVISE, madvise)];
