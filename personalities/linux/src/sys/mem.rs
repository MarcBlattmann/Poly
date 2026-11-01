//! Memory: mmap(2), munmap(2), mprotect(2), mremap(2), msync(2), brk(2).
//!
//! The personality validates arguments as the man pages describe and turns
//! them into a [`MapRequest`] or a page-aligned range; the kernel's VMA
//! manager does the bookkeeping. Where the man pages list an error but not
//! the order of the checks, the order below is ours (the diff runner will
//! compare it with real Linux).

use crate::abi::{map, page_aligned, page_round_up, prot};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{Backing, LinuxKernel, MapHints, MapRequest, Placement, Prot, Remap};

/// Flags `MAP_SHARED_VALIDATE` accepts: every flag mmap(2) documents.
/// Anything else fails with `EOPNOTSUPP` under `MAP_SHARED_VALIDATE` and is
/// ignored under `MAP_SHARED` / `MAP_PRIVATE` (mmap(2), MAP_SHARED_VALIDATE).
const KNOWN_FLAGS: u32 = map::MAP_TYPE
    | map::MAP_FIXED
    | map::MAP_ANONYMOUS
    | map::MAP_32BIT
    | map::MAP_GROWSDOWN
    | map::MAP_DENYWRITE
    | map::MAP_EXECUTABLE
    | map::MAP_LOCKED
    | map::MAP_NORESERVE
    | map::MAP_POPULATE
    | map::MAP_NONBLOCK
    | map::MAP_STACK
    | map::MAP_HUGETLB
    | map::MAP_SYNC
    | map::MAP_FIXED_NOREPLACE
    | (map::MAP_HUGE_MASK << map::MAP_HUGE_SHIFT);

/// Protection bits mmap accepts.
const MMAP_PROT: u32 = prot::PROT_READ | prot::PROT_WRITE | prot::PROT_EXEC | prot::PROT_SEM;

/// Protection bits mprotect accepts (mprotect(2): the access flags plus one
/// of `PROT_GROWSUP` / `PROT_GROWSDOWN`).
const MPROTECT_PROT: u32 = MMAP_PROT | prot::PROT_GROWSDOWN | prot::PROT_GROWSUP;

/// mmap(addr, length, prot, flags, fd, offset)
///
/// Checks, in this order (mmap(2) ERRORS):
/// 1. mapping type (`flags & MAP_TYPE`) is `MAP_SHARED`, `MAP_PRIVATE` or
///    `MAP_SHARED_VALIDATE`, else `EINVAL`;
/// 2. under `MAP_SHARED_VALIDATE`, unknown flags give `EOPNOTSUPP`, and so
///    does `MAP_SYNC` (only DAX files support it; Poly has none); then
///    `MAP_ANONYMOUS` gives `EINVAL` (its type bits are `MAP_SHARED |
///    MAP_PRIVATE`; the visible test `tests/linux/mmap.c` expects it);
/// 3. `length` is not 0, else `EINVAL`;
/// 4. `offset` is page aligned, else `EINVAL` (checked for anonymous
///    mappings too: "offset must be a multiple of the page size");
/// 5. with `MAP_FIXED` / `MAP_FIXED_NOREPLACE`, `addr` is page aligned,
///    else `EINVAL`;
/// 6. `prot` has only `PROT_READ|WRITE|EXEC|SEM`, else `EINVAL`;
/// 7. `length` rounded up to pages fits in 64 bits, else `ENOMEM`;
/// 8. `MAP_HUGETLB` gives `ENOMEM`: Poly has no huge page pool, and with an
///    empty pool there is no memory to back the mapping;
/// 9. a file mapping needs `fd >= 0` (`EBADF`) and `offset + length` must
///    not overflow (`EINVAL`, "too large").
///
/// `MAP_FIXED_NOREPLACE` wins over `MAP_FIXED` when both are set (it is the
/// stricter of the two). `MAP_32BIT` is ignored with fixed placements, as
/// documented. `MAP_DENYWRITE`, `MAP_EXECUTABLE`, `MAP_NONBLOCK` and
/// `MAP_UNINITIALIZED` are ignored, as on Linux.
pub fn mmap(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(0);
    let length = a.ulong(1);
    let prot_bits = a.uint(2);
    let flags = a.uint(3);
    let fd = a.fd(4);
    let offset = a.ulong(5);

    let shared = match flags & map::MAP_TYPE {
        map::MAP_SHARED => true,
        map::MAP_PRIVATE => false,
        map::MAP_SHARED_VALIDATE => {
            if flags & !KNOWN_FLAGS != 0 || flags & map::MAP_SYNC != 0 {
                return Err(Errno::EOPNOTSUPP);
            }
            // The type bits of MAP_SHARED_VALIDATE are MAP_SHARED |
            // MAP_PRIVATE; with MAP_ANONYMOUS that is EINVAL (the visible
            // test tests/linux/mmap.c, mmap_shared_and_private_einval).
            if flags & map::MAP_ANONYMOUS != 0 {
                return Err(Errno::EINVAL);
            }
            true
        }
        _ => return Err(Errno::EINVAL),
    };
    if length == 0 || !page_aligned(offset) {
        return Err(Errno::EINVAL);
    }
    let placement = if flags & map::MAP_FIXED_NOREPLACE != 0 {
        Placement::FixedNoReplace
    } else if flags & map::MAP_FIXED != 0 {
        Placement::Fixed
    } else {
        Placement::Hint
    };
    if placement != Placement::Hint && !page_aligned(addr) {
        return Err(Errno::EINVAL);
    }
    if prot_bits & !MMAP_PROT != 0 {
        return Err(Errno::EINVAL);
    }
    let len = page_round_up(length).ok_or(Errno::ENOMEM)?;
    if flags & map::MAP_HUGETLB != 0 {
        return Err(Errno::ENOMEM);
    }
    let backing = if flags & map::MAP_ANONYMOUS != 0 {
        Backing::Anonymous
    } else {
        if fd < 0 {
            return Err(Errno::EBADF);
        }
        offset.checked_add(len).ok_or(Errno::EINVAL)?;
        Backing::File { fd, offset }
    };
    let hints = MapHints {
        low_2g: flags & map::MAP_32BIT != 0 && placement == Placement::Hint,
        grows_down: flags & map::MAP_GROWSDOWN != 0,
        stack: flags & map::MAP_STACK != 0,
        populate: flags & map::MAP_POPULATE != 0,
        locked: flags & map::MAP_LOCKED != 0,
        no_reserve: flags & map::MAP_NORESERVE != 0,
    };
    // personality(2) READ_IMPLIES_EXEC: readable means executable too.
    let prot_bits = crate::sys::personality::effective_prot(crate::sys::personality::current(k), prot_bits);
    let request = MapRequest { addr, len, prot: Prot(prot_bits), placement, shared, backing, hints };
    // Under mlockall(MCL_FUTURE) the mapping is made locked (the kernel's
    // region map does that) and, without MCL_ONFAULT, populated here.
    let future = crate::sys::mlock::future_mmap_check(k, len)?;
    let at = k.mmap(&request)?;
    if future == Some(crate::kernel::MemLock::Lock) {
        // Pages that cannot come in (PROT_NONE) do not fail the mmap.
        let _ = k.mlock(at, len, crate::kernel::MemLock::Lock);
    }
    Ok(at)
}

/// munmap(addr, length)
///
/// munmap(2): `addr` must be page aligned and `length` non-zero (`EINVAL`);
/// "all pages containing a part of the indicated range are unmapped", so
/// the length is rounded up to pages; a range that wraps around is
/// `EINVAL` ("we don't like addr, length"). Unmapped holes are not errors.
/// Unmapping pages of System V shared memory attachments is recorded in
/// their segments ([`crate::sys::shm::unmapped`]).
pub fn munmap(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(0);
    let length = a.ulong(1);
    if !page_aligned(addr) || length == 0 {
        return Err(Errno::EINVAL);
    }
    let len = page_round_up(length).ok_or(Errno::EINVAL)?;
    addr.checked_add(len).ok_or(Errno::EINVAL)?;
    let shm = k.shm_mapped(addr, len);
    k.munmap(addr, len)?;
    if !shm.is_empty() {
        crate::sys::shm::unmapped(k, &shm);
    }
    Ok(0)
}

/// mprotect(addr, size, prot)
///
/// mprotect(2): `EINVAL` if `addr` is not page aligned, if `prot` has
/// unknown bits, or if it has both `PROT_GROWSUP` and `PROT_GROWSDOWN`;
/// the pages "containing any part of" `[addr, addr+size-1]` change, so the
/// size is rounded up; a range that wraps is "invalid for the address
/// space of the process" (`ENOMEM`). `size` 0 covers no page and succeeds
/// without asking the kernel.
pub fn mprotect(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(0);
    let size = a.ulong(1);
    let prot_bits = a.uint(2);
    if !page_aligned(addr) || prot_bits & !MPROTECT_PROT != 0 {
        return Err(Errno::EINVAL);
    }
    let grows = prot::PROT_GROWSDOWN | prot::PROT_GROWSUP;
    if prot_bits & grows == grows {
        return Err(Errno::EINVAL);
    }
    if size == 0 {
        return Ok(0);
    }
    let len = page_round_up(size).ok_or(Errno::ENOMEM)?;
    addr.checked_add(len).ok_or(Errno::ENOMEM)?;
    let prot_bits = crate::sys::personality::effective_prot(crate::sys::personality::current(k), prot_bits);
    k.mprotect(addr, len, Prot(prot_bits)).map(|()| 0)
}

/// mremap(2) flags (`linux/mman.h`).
pub const MREMAP_MAYMOVE: u32 = 1;
pub const MREMAP_FIXED: u32 = 2;
pub const MREMAP_DONTUNMAP: u32 = 4;

/// mremap(old_address, old_size, new_size, flags, new_address) (ADR 0059;
/// the C library's `realloc` of large blocks, Python's `mmap.resize`).
///
/// mremap(2) ERRORS, checked in this order (ours, to be confirmed by the
/// diff runner): unknown flags, `MREMAP_FIXED` or `MREMAP_DONTUNMAP`
/// without `MREMAP_MAYMOVE`, a misaligned `old_address` or a zero
/// `new_size` (`EINVAL`). `MREMAP_DONTUNMAP` (Linux 5.7) is `EINVAL`, as
/// on a kernel that predates it: leaving the old range mapped but empty is
/// behaviour Poly lacks. An `old_size` of 0 asks for a second mapping of
/// the same shared pages, which Poly cannot make (`EINVAL`). The sizes are
/// rounded up to pages; the kernel checks the range (`EFAULT`), the
/// destination (`EINVAL`) and the room (`ENOMEM`).
pub fn mremap(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (old, old_size, new_size, flags, new_addr) = (a.ulong(0), a.ulong(1), a.ulong(2), a.uint(3), a.ulong(4));
    if flags & !(MREMAP_MAYMOVE | MREMAP_FIXED | MREMAP_DONTUNMAP) != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & (MREMAP_FIXED | MREMAP_DONTUNMAP) != 0 && flags & MREMAP_MAYMOVE == 0 {
        return Err(Errno::EINVAL);
    }
    if !page_aligned(old) || new_size == 0 || flags & MREMAP_DONTUNMAP != 0 {
        return Err(Errno::EINVAL);
    }
    // old_size 0: a second mapping of a shared mapping's pages, which
    // needs MREMAP_MAYMOVE (mremap(2)); the kernel refuses a private one.
    if old_size == 0 && flags & MREMAP_MAYMOVE == 0 {
        return Err(Errno::EINVAL);
    }
    let old_len = page_round_up(old_size).ok_or(Errno::EFAULT)?;
    let new_len = page_round_up(new_size).ok_or(Errno::ENOMEM)?;
    let to = if flags & MREMAP_FIXED != 0 {
        Remap::Fixed(new_addr)
    } else if flags & MREMAP_MAYMOVE != 0 {
        Remap::MayMove
    } else {
        Remap::InPlace
    };
    k.mremap(old, old_len, new_len, to)
}

/// msync(2) flags (`asm-generic/mman-common.h`).
pub const MS_ASYNC: u32 = 1;
pub const MS_INVALIDATE: u32 = 2;
pub const MS_SYNC: u32 = 4;

/// msync(addr, length, flags) (ADR 0059; Python's `mmap.flush`).
///
/// msync(2): `EINVAL` if `addr` is not page aligned, `flags` has unknown
/// bits or both `MS_SYNC` and `MS_ASYNC`; `ENOMEM` if part of the range is
/// not mapped (the kernel checks). The length is rounded up to pages; a
/// zero length succeeds. `MS_SYNC` writes the files of shared mappings in
/// the range to their disks before returning; `MS_ASYNC` has nothing to
/// start (the page cache writes dirty pages back on its own, and every
/// mapping of a file shares its pages), nor has `MS_INVALIDATE` (no other
/// copies of the pages exist).
pub fn msync(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (addr, length, flags) = (a.ulong(0), a.ulong(1), a.uint(2));
    if !page_aligned(addr) || flags & !(MS_ASYNC | MS_INVALIDATE | MS_SYNC) != 0 {
        return Err(Errno::EINVAL);
    }
    if flags & MS_SYNC != 0 && flags & MS_ASYNC != 0 {
        return Err(Errno::EINVAL);
    }
    let len = page_round_up(length).ok_or(Errno::ENOMEM)?;
    addr.checked_add(len).ok_or(Errno::ENOMEM)?;
    if len == 0 {
        return Ok(0);
    }
    k.msync(addr, len, flags & MS_SYNC != 0).map(|()| 0)
}

/// brk(addr)
///
/// brk(2), "C library/kernel differences": "the actual Linux system call
/// returns the new program break on success. On failure, the system call
/// returns the current break." So this never returns an error: `brk(0)`
/// (the usual query) and every request the kernel refuses return the
/// current break, and the C library compares it with what it asked for.
pub fn brk(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(0);
    if addr == 0 {
        return Ok(k.brk());
    }
    match k.set_brk(addr) {
        Ok(new) => Ok(new),
        Err(_) => Ok(k.brk()),
    }
}
