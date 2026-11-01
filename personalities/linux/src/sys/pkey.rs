//! Memory protection keys (pkeys(7), ADR 0121): pkey_alloc(2),
//! pkey_free(2), pkey_mprotect(2).
//!
//! Protection keys need a CPU with PKU and the OS enabling it (`OSPKE`).
//! Poly does not enable it, and QEMU's default CPU has none, so Poly
//! behaves as Linux does on such a CPU: no key can ever be allocated
//! (pkey_alloc is `ENOSPC` once its arguments are valid), no key is ever
//! allocated (pkey_free is always `EINVAL`, the default key 0 included),
//! and pkey_mprotect accepts only key -1 ("no key"), with which it is
//! mprotect(2).
//!
//! Constants from `linux/mman.h` / `asm-generic/mman-common.h`
//! (`PKEY_DISABLE_ACCESS`, `PKEY_DISABLE_WRITE`).

use crate::abi::prot;
use crate::abi::{page_aligned, page_round_up};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, Prot};
use crate::table::{Handler, nr};

pub const PKEY_DISABLE_ACCESS: u64 = 0x1;
pub const PKEY_DISABLE_WRITE: u64 = 0x2;

/// The `prot` bits pkey_mprotect accepts (as mprotect does).
const PROT_BITS: u64 =
    (prot::PROT_READ | prot::PROT_WRITE | prot::PROT_EXEC | prot::PROT_SEM | prot::PROT_GROWSDOWN | prot::PROT_GROWSUP)
        as u64;

pub const CALLS: &[(u64, Handler)] =
    &[(nr::PKEY_MPROTECT, pkey_mprotect), (nr::PKEY_ALLOC, pkey_alloc), (nr::PKEY_FREE, pkey_free)];

/// pkey_alloc(flags, access_rights): both full 64-bit values; `flags`
/// must be 0 and `access_rights` only `PKEY_DISABLE_ACCESS` /
/// `PKEY_DISABLE_WRITE` (`EINVAL`), then `ENOSPC`: no key exists.
pub fn pkey_alloc(_k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ulong(0) != 0 || a.ulong(1) & !(PKEY_DISABLE_ACCESS | PKEY_DISABLE_WRITE) != 0 {
        return Err(Errno::EINVAL);
    }
    Err(Errno::ENOSPC)
}

/// pkey_free(pkey): `EINVAL`, since no key is ever allocated.
pub fn pkey_free(_k: &mut dyn LinuxKernel, _a: &Args) -> SysResult {
    Err(Errno::EINVAL)
}

/// pkey_mprotect(addr, len, prot, pkey): `pkey` is a 32-bit int, the
/// others full 64-bit values. In order (pkey_mprotect's spec sheet):
/// `addr` alignment (`EINVAL`), `len` 0 (0), the rounded end wrapping
/// (`ENOMEM`), `prot` (`EINVAL`), the key (`EINVAL` for any but -1), then
/// mprotect of the range.
pub fn pkey_mprotect(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = a.ulong(0);
    let len = a.ulong(1);
    let bits = a.ulong(2);
    if !page_aligned(addr) {
        return Err(Errno::EINVAL);
    }
    if len == 0 {
        return Ok(0);
    }
    let len = page_round_up(len).filter(|&l| addr.checked_add(l).is_some()).ok_or(Errno::ENOMEM)?;
    let grows = (prot::PROT_GROWSDOWN | prot::PROT_GROWSUP) as u64;
    if bits & !PROT_BITS != 0 || bits & grows == grows {
        return Err(Errno::EINVAL);
    }
    if a.int(3) != -1 {
        return Err(Errno::EINVAL);
    }
    k.mprotect(addr, len, Prot(bits as u32)).map(|()| 0)
}
