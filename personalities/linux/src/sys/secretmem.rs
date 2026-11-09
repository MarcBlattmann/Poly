//! memfd_secret(2): an anonymous "secret memory" file (ADR 0122).
//!
//! The file is a memfd marked secret (`poly_vfs::memfd::Memfd::secret`):
//! a regular file, mode 0600, open `O_RDWR | O_LARGEFILE`, whose memory is
//! used only through `MAP_SHARED` mappings. Its size is set once with
//! ftruncate(2); read and write are `EINVAL`, pread and lseek `ESPIPE`,
//! fallocate `EOPNOTSUPP`, a `MAP_PRIVATE` mapping `EINVAL`. Poly has no
//! direct map of user memory to take the pages out of, and never swaps,
//! so the memory is as private as the man page asks for already.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::flags::O_CLOEXEC;
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::MEMFD_SECRET, memfd_secret)];

/// Whether the descriptor gets `FD_CLOEXEC`; `EINVAL` for any flag but
/// `O_CLOEXEC` (`O_NONBLOCK` included).
pub fn plan(flags: u32) -> Result<bool, Errno> {
    if flags & !O_CLOEXEC != 0 {
        return Err(Errno::EINVAL);
    }
    Ok(flags & O_CLOEXEC != 0)
}

/// memfd_secret(flags)
pub fn memfd_secret(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let cloexec = plan(a.uint(0))?;
    k.memfd_secret(cloexec).map(|fd| fd as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cloexec_is_accepted() {
        assert_eq!(plan(0), Ok(false));
        assert_eq!(plan(O_CLOEXEC), Ok(true));
        assert_eq!(plan(1), Err(Errno::EINVAL));
        assert_eq!(plan(crate::fileio::flags::O_NONBLOCK), Err(Errno::EINVAL));
    }
}
