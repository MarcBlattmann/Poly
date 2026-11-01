//! memfd_create(2): an anonymous file in memory (ADR 0111).
//!
//! The personality checks the flags and the name and decides the new
//! file's mode and first seals ([`plan`]); the kernel makes the file on a
//! memory file system no path reaches (`LinuxFs::memfd_create`). The seals
//! themselves (fcntl `F_ADD_SEALS` / `F_GET_SEALS`) live with the file in
//! the VFS (`poly_vfs::memfd`).
//!
//! Order of the checks: the flags (`EINVAL`), the name (`EFAULT` when it is
//! unreadable before its terminator, `EINVAL` when it has 250 bytes or
//! more), then the descriptor (`EMFILE`).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::path::fs;
use crate::table::{Handler, nr};

/// `memfd_create` flags (`linux/memfd.h`).
pub const MFD_CLOEXEC: u32 = 0x1;
pub const MFD_ALLOW_SEALING: u32 = 0x2;
pub const MFD_HUGETLB: u32 = 0x4;
pub const MFD_NOEXEC_SEAL: u32 = 0x8;
pub const MFD_EXEC: u32 = 0x10;
/// The huge page size field (`MFD_HUGE_SHIFT` 26, `MFD_HUGE_MASK` 0x3f).
pub const MFD_HUGE_MASK: u32 = 0x3f << 26;

/// Seals (`linux/fcntl.h`) a new file starts with.
pub const F_SEAL_SEAL: u32 = 0x1;
pub const F_SEAL_EXEC: u32 = 0x20;

/// The longest name: `NAME_MAX` (255) less the `memfd:` prefix.
pub const MFD_NAME_MAX: usize = 249;

/// What a valid call makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The mode's permission bits (the umask is not applied).
    pub perm: u32,
    /// The seals the file starts with.
    pub seals: u32,
    /// `FD_CLOEXEC` on the descriptor.
    pub cloexec: bool,
}

/// The new file for `flags` (the low 32 bits of the argument), or `EINVAL`.
pub fn plan(flags: u32) -> Result<Plan, Errno> {
    let mut known = MFD_CLOEXEC | MFD_ALLOW_SEALING | MFD_HUGETLB | MFD_NOEXEC_SEAL | MFD_EXEC;
    if flags & MFD_HUGETLB != 0 {
        known |= MFD_HUGE_MASK;
    }
    if flags & !known != 0 || flags & (MFD_EXEC | MFD_NOEXEC_SEAL) == MFD_EXEC | MFD_NOEXEC_SEAL {
        return Err(Errno::EINVAL);
    }
    let cloexec = flags & MFD_CLOEXEC != 0;
    Ok(if flags & MFD_NOEXEC_SEAL != 0 {
        // Sealing is allowed, with the execute bits sealed off.
        Plan { perm: 0o666, seals: F_SEAL_EXEC, cloexec }
    } else {
        let seals = if flags & MFD_ALLOW_SEALING != 0 { 0 } else { F_SEAL_SEAL };
        Plan { perm: 0o777, seals, cloexec }
    })
}

/// The name at `p`, without its NUL: `EFAULT` if a byte before the NUL
/// cannot be read, `EINVAL` if there is no NUL within the first
/// [`MFD_NAME_MAX`] + 1 bytes.
pub fn name_arg(k: &dyn LinuxKernel, p: UserPtr) -> Result<alloc::vec::Vec<u8>, Errno> {
    let mut buf = [0u8; MFD_NAME_MAX + 1];
    match k.strncpy_from_user(p, &mut buf) {
        Ok(n) => Ok(buf[..n].to_vec()),
        Err(e) if e == Errno::ENAMETOOLONG => Err(Errno::EINVAL),
        Err(e) => Err(e),
    }
}

/// memfd_create(name, flags)
pub fn memfd_create(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let plan = plan(a.uint(1))?;
    let name = name_arg(k, a.ptr(0))?;
    fs(k)?.memfd_create(&name, plan.perm, plan.seals, plan.cloexec).map(|fd| fd as u64)
}

pub const CALLS: &[(u64, Handler)] = &[(nr::MEMFD_CREATE, memfd_create)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_choose_mode_and_seals() {
        assert_eq!(plan(0), Ok(Plan { perm: 0o777, seals: F_SEAL_SEAL, cloexec: false }));
        assert_eq!(plan(MFD_CLOEXEC), Ok(Plan { perm: 0o777, seals: F_SEAL_SEAL, cloexec: true }));
        assert_eq!(plan(MFD_ALLOW_SEALING), Ok(Plan { perm: 0o777, seals: 0, cloexec: false }));
        assert_eq!(plan(MFD_EXEC), Ok(Plan { perm: 0o777, seals: F_SEAL_SEAL, cloexec: false }));
        assert_eq!(plan(MFD_EXEC | MFD_ALLOW_SEALING), Ok(Plan { perm: 0o777, seals: 0, cloexec: false }));
        assert_eq!(plan(MFD_NOEXEC_SEAL), Ok(Plan { perm: 0o666, seals: F_SEAL_EXEC, cloexec: false }));
        assert_eq!(
            plan(MFD_NOEXEC_SEAL | MFD_ALLOW_SEALING),
            Ok(Plan { perm: 0o666, seals: F_SEAL_EXEC, cloexec: false })
        );
        assert!(plan(MFD_HUGETLB).is_ok());
        assert!(plan(MFD_HUGETLB | (21 << 26)).is_ok());
    }

    #[test]
    fn unknown_flags_and_contradictions_are_einval() {
        for bad in [0x20, 0x40, 0x100, 0x10000, 0x8000_0000, 21 << 26, MFD_EXEC | MFD_NOEXEC_SEAL] {
            assert_eq!(plan(bad), Err(Errno::EINVAL), "{bad:#x}");
        }
    }
}
