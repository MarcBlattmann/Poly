//! BSD memory calls (`specs/macos/bsd/{mprotect,madvise,minherit,msync,
//! mincore,mlock}.yaml`) and the range helpers the Mach VM traps share.
//!
//! Each call checks its arguments and the mapped ranges the way the sheets
//! record; the kernel changes the address space.

use crate::errno::*;
use crate::kernel::{Arch, Backing, Inherit, XnuKernel};
use alloc::vec;

/// `len` rounded up to whole pages; `None` if it overflows.
pub fn page_up(len: u64, ps: u64) -> Option<u64> {
    len.checked_add(ps - 1).map(|l| l & !(ps - 1))
}

/// The pages `[addr, addr + len)` touches: the start rounded down, the end
/// up. `None` if the end overflows.
pub fn touched(addr: u64, len: u64, ps: u64) -> Option<(u64, u64)> {
    let end = addr.checked_add(len)?;
    Some((addr & !(ps - 1), page_up(end, ps)?))
}

/// How much of a range is mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    Full,
    Partial,
    None,
}

pub fn coverage(k: &dyn XnuKernel, start: u64, end: u64) -> Coverage {
    let mut at = start;
    let mut any = false;
    let mut holes = false;
    for r in k.regions(start, end) {
        if r.start > at {
            holes = true;
        }
        any = true;
        at = at.max(r.end);
    }
    if at < end {
        holes = true;
    }
    match (any, holes) {
        (false, _) => Coverage::None,
        (true, true) => Coverage::Partial,
        (true, false) => Coverage::Full,
    }
}

/// arm64 programs may not have anonymous memory writable and executable
/// at once (mprotect's sheet: `EACCES`).
pub fn wx_refused(k: &dyn XnuKernel, prot: u32) -> bool {
    k.arch() == Arch::Arm64 && prot & (PROT_WRITE | PROT_EXEC) == (PROT_WRITE | PROT_EXEC)
}

pub const PROT_READ: u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC: u32 = 4;

/// `<sys/mman.h>` msync flags.
pub const MS_ASYNC: u32 = 0x1;
pub const MS_SYNC: u32 = 0x10;

/// `<sys/mman.h>` madvise values.
pub const MADV_PAGEOUT: u32 = 10;
pub const MADV_ZERO: u32 = 11;

/// `<sys/mman.h>` mincore bits.
pub const MINCORE_INCORE: u8 = 0x1;
pub const MINCORE_REFERENCED: u8 = 0x2;
pub const MINCORE_MODIFIED: u8 = 0x4;
pub const MINCORE_ANONYMOUS: u8 = 0x80;

/// mprotect(addr, len, prot): the page-aligned address is required; a
/// length of 0 changes nothing; every page must be mapped (`ENOMEM`); the
/// protection must be within each region's maximum and the mapping's file
/// mode (`EACCES`).
pub fn mprotect(k: &mut dyn XnuKernel, addr: u64, len: u64, prot: u32) -> Result<u64, u32> {
    let ps = k.page_size();
    if !addr.is_multiple_of(ps) || (len as i64) < 0 {
        return Err(EINVAL);
    }
    if len == 0 {
        return Ok(0);
    }
    let (start, end) = touched(addr, len, ps).ok_or(ENOMEM)?;
    if coverage(k, start, end) != Coverage::Full {
        return Err(ENOMEM);
    }
    // Bits the call does not know are ignored.
    let prot = prot & (PROT_READ | PROT_WRITE | PROT_EXEC);
    let wx = prot & (PROT_WRITE | PROT_EXEC) == (PROT_WRITE | PROT_EXEC);
    for r in k.regions(start, end) {
        let refused = prot & !r.max_prot != 0
            || match r.backing {
                Backing::File { shared: true, writable: false } => prot & PROT_WRITE != 0,
                Backing::File { shared: false, .. } => wx && k.arch() == Arch::Arm64,
                Backing::Anonymous { .. } => wx_refused(k, prot),
                Backing::File { .. } => false,
            };
        if refused {
            return Err(EACCES);
        }
    }
    k.protect(start, end, prot);
    Ok(0)
}

/// madvise(addr, len, advice): the advice values 0..=9 and MADV_ZERO (11)
/// are taken, MADV_PAGEOUT is `ENOTSUP`, others `EINVAL`. An address below
/// the first mapping is `ENOMEM`; a range with a hole or past a mapping is
/// `EINVAL`; an unaligned address is taken (the pages it touches).
/// Nothing but MADV_ZERO changes what the memory holds: Poly does not page
/// anonymous memory out.
pub fn madvise(k: &mut dyn XnuKernel, addr: u64, len: u64, advice: u32) -> Result<u64, u32> {
    match advice {
        0..=9 | MADV_ZERO => {}
        MADV_PAGEOUT => return Err(ENOTSUP),
        _ => return Err(EINVAL),
    }
    if len == 0 {
        return Ok(0);
    }
    let (start, end) = touched(addr, len, k.page_size()).ok_or(EINVAL)?;
    let first = k.regions(0, u64::MAX).first().map(|r| r.start);
    if first.is_none_or(|f| start < f) {
        return Err(ENOMEM);
    }
    if coverage(k, start, end) != Coverage::Full {
        return Err(EINVAL);
    }
    if advice == MADV_ZERO {
        k.zero(start, end);
    }
    Ok(0)
}

/// minherit(addr, len, inheritance): SHARE (0), COPY (1), NONE (2); others
/// `EINVAL`. Unaligned and unmapped ranges are taken; the mapped pages of
/// the range change.
pub fn minherit(k: &mut dyn XnuKernel, addr: u64, len: u64, inherit: u32) -> Result<u64, u32> {
    let inherit = match inherit {
        0 => Inherit::Share,
        1 => Inherit::Copy,
        2 => Inherit::None,
        _ => return Err(EINVAL),
    };
    if let Some((start, end)) = touched(addr, len, k.page_size())
        && start < end
    {
        for r in k.regions(start, end) {
            k.set_inherit(r.start.max(start), r.end.min(end), inherit);
        }
    }
    Ok(0)
}

/// msync(addr, len, flags): the address must be page aligned and the length
/// not 0 (`EINVAL`); MS_SYNC with MS_ASYNC is `EINVAL`; unknown flag bits
/// are taken; every page must be mapped (`ENOMEM`).
pub fn msync(k: &mut dyn XnuKernel, addr: u64, len: u64, flags: u32) -> Result<u64, u32> {
    let ps = k.page_size();
    if !addr.is_multiple_of(ps) || len == 0 || flags & (MS_SYNC | MS_ASYNC) == (MS_SYNC | MS_ASYNC) {
        return Err(EINVAL);
    }
    let (start, end) = touched(addr, len, ps).ok_or(ENOMEM)?;
    if coverage(k, start, end) != Coverage::Full {
        return Err(ENOMEM);
    }
    k.sync(start, end, flags & MS_SYNC != 0);
    Ok(0)
}

/// mincore(addr, len, vec): one byte per page touched; a page that is not
/// mapped reads 0. `EFAULT` for a vector that cannot be written.
pub fn mincore(k: &mut dyn XnuKernel, addr: u64, len: u64, vec_ptr: u64) -> Result<u64, u32> {
    if len == 0 {
        return Ok(0);
    }
    let ps = k.page_size();
    let (start, end) = touched(addr, len, ps).ok_or(ENOMEM)?;
    let pages = usize::try_from((end - start) / ps).map_err(|_| ENOMEM)?;
    let mut out = vec![0u8; pages];
    let regions = k.regions(start, end);
    for (i, b) in out.iter_mut().enumerate() {
        let page = start + i as u64 * ps;
        let Some(r) = regions.iter().find(|r| r.start <= page && page < r.end) else { continue };
        let info = k.page_info(page);
        if info.resident {
            *b |= MINCORE_INCORE;
        }
        if info.referenced {
            *b |= MINCORE_REFERENCED;
        }
        if info.modified {
            *b |= MINCORE_MODIFIED;
        }
        if matches!(r.backing, Backing::Anonymous { .. }) {
            *b |= MINCORE_ANONYMOUS;
        }
    }
    k.copy_out(vec_ptr, &out).map_err(|_| EFAULT)?;
    Ok(0)
}

/// mlock / munlock(addr, len): the pages the range touches. A range with no
/// mapping at all is taken; one that is only partly mapped is `ENOMEM`.
pub fn mlock(k: &mut dyn XnuKernel, addr: u64, len: u64, lock: bool) -> Result<u64, u32> {
    if len == 0 {
        return Ok(0);
    }
    let (start, end) = touched(addr, len, k.page_size()).ok_or(ENOMEM)?;
    match coverage(k, start, end) {
        Coverage::None => Ok(0),
        Coverage::Partial => Err(ENOMEM),
        Coverage::Full => {
            k.set_locked(start, end, lock);
            Ok(0)
        }
    }
}
