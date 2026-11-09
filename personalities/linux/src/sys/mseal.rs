//! mseal(2): seal mappings so they can no longer change (ADR 0122).
//!
//! The kernel's region map keeps the seal per region (`poly_vm`'s
//! `Vma::sealed`); `munmap`, `mprotect`, `mremap` and `mmap(MAP_FIXED)`
//! over a sealed page fail with `EPERM` there, changing nothing, and so do
//! discarding madvise(2) calls on sealed private anonymous memory without
//! write permission. This checks the arguments in the order the call
//! reports errors.

use crate::abi::page_aligned;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::MSEAL, mseal)];

/// What mseal(addr, len, flags) asks for once its arguments are checked:
/// `None` for a length of 0 (success, nothing to do), else the page
/// aligned range.
pub fn plan(addr: u64, len: u64, flags: u64) -> Result<Option<(u64, u64)>, Errno> {
    if flags != 0 || !page_aligned(addr) {
        return Err(Errno::EINVAL);
    }
    let len = crate::abi::page_round_up(len).ok_or(Errno::EINVAL)?;
    addr.checked_add(len).ok_or(Errno::EINVAL)?;
    Ok((len != 0).then_some((addr, len)))
}

/// mseal(addr, len, flags): `EINVAL` for flags other than 0, a misaligned
/// `addr` or a range that wraps around; `ENOMEM` (the kernel) if any part
/// of the range is unmapped, nothing sealed then.
pub fn mseal(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    match plan(a.ulong(0), a.ulong(1), a.ulong(2))? {
        Some((addr, len)) => k.mseal(addr, len).map(|()| 0),
        None => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        assert_eq!(plan(0x1000, 1, 0), Ok(Some((0x1000, 0x1000))));
        assert_eq!(plan(0x1000, 0, 0), Ok(None));
        assert_eq!(plan(0x1000, 0x1000, 1), Err(Errno::EINVAL));
        assert_eq!(plan(0x1001, 0x1000, 0), Err(Errno::EINVAL));
        assert_eq!(plan(0xffff_ffff_ffff_f000, 0x2000, 0), Err(Errno::EINVAL));
        assert_eq!(plan(0x1000, u64::MAX, 0), Err(Errno::EINVAL));
    }
}
