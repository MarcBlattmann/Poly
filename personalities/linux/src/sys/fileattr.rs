//! file_getattr(2), file_setattr(2): a file's inode flags by path, as the
//! `FS_IOC_FSGETXATTR` / `FS_IOC_FSSETXATTR` ioctls give them, in a
//! `struct file_attr` (UAPI `linux/fs.h`: `__u64 fa_xflags; __u32
//! fa_extsize, fa_nextents, fa_projid, fa_cowextsize`, 24 bytes,
//! `FILE_ATTR_SIZE_VER0`).
//!
//! The order (their spec sheets): `usize` (below 24 `EINVAL`, above a page
//! `E2BIG`), `at_flags` (`AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH`, else
//! `EINVAL`), for file_setattr the structure (`EFAULT`, a non-zero byte
//! past the 24 `E2BIG`), the path (as the other *at calls), then the
//! kernel: `EOPNOTSUPP` for a file without inode flags, file_setattr's
//! flag and permission checks (`poly_vfs::fileattr`). file_getattr writes
//! `usize` bytes, the ones past the structure zero.

use crate::args::Args;
use crate::errno::SysResult;
use crate::fs::at::AT_SYMLINK_NOFOLLOW;
use crate::kernel::LinuxKernel;
use crate::sys::path::fs;
use crate::sys::xattrat::{at_path, check_at_flags, check_ext_size, ext_struct};
use crate::table::{Handler, nr};
use alloc::vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::FILE_GETATTR, file_getattr), (nr::FILE_SETATTR, file_setattr)];

/// `struct file_attr`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileAttr {
    pub xflags: u64,
    pub extsize: u32,
    pub nextents: u32,
    pub projid: u32,
    pub cowextsize: u32,
}

impl FileAttr {
    /// `FILE_ATTR_SIZE_VER0`.
    pub const SIZE: usize = 24;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..8].copy_from_slice(&self.xflags.to_le_bytes());
        for (i, w) in [self.extsize, self.nextents, self.projid, self.cowextsize].iter().enumerate() {
            b[8 + 4 * i..12 + 4 * i].copy_from_slice(&w.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> FileAttr {
        let word = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let mut x = [0u8; 8];
        x.copy_from_slice(&b[..8]);
        FileAttr {
            xflags: u64::from_le_bytes(x),
            extsize: word(8),
            nextents: word(12),
            projid: word(16),
            cowextsize: word(20),
        }
    }
}

/// file_getattr(dfd, filename, ufattr, usize, at_flags)
pub fn file_getattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (dfd, buf, at_flags) = (a.fd(0), a.ptr(2), a.uint(4));
    let size = check_ext_size(a.ulong(3), FileAttr::SIZE)?;
    check_at_flags(at_flags)?;
    let path = at_path(k, a.ptr(1), at_flags)?;
    let attr = fs(k)?.file_getattr(dfd, &path, at_flags & AT_SYMLINK_NOFOLLOW == 0)?;
    let mut out = vec![0u8; size];
    out[..FileAttr::SIZE].copy_from_slice(&attr.to_bytes());
    k.copy_to_user(buf, &out)?;
    Ok(0)
}

/// file_setattr(dfd, filename, ufattr, usize, at_flags)
pub fn file_setattr(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (dfd, buf, at_flags) = (a.fd(0), a.ptr(2), a.uint(4));
    check_ext_size(a.ulong(3), FileAttr::SIZE)?;
    check_at_flags(at_flags)?;
    let attr = FileAttr::from_bytes(&ext_struct(k, buf, a.ulong(3), FileAttr::SIZE)?);
    let path = at_path(k, a.ptr(1), at_flags)?;
    fs(k)?.file_setattr(dfd, &path, at_flags & AT_SYMLINK_NOFOLLOW == 0, &attr).map(|()| 0)
}

#[cfg(test)]
mod tests {
    use super::FileAttr;

    #[test]
    fn layout_round_trips() {
        let a = FileAttr { xflags: 0x80, extsize: 1, nextents: 2, projid: 3, cowextsize: 4 };
        let b = a.to_bytes();
        assert_eq!(&b[..8], &0x80u64.to_le_bytes());
        assert_eq!(&b[16..20], &3u32.to_le_bytes());
        assert_eq!(FileAttr::from_bytes(&b), a);
    }
}
