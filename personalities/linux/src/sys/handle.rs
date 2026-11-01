//! name_to_handle_at(2), open_by_handle_at(2): persistent, opaque names
//! for files.
//!
//! A handle travels in a `struct file_handle` (the man page;
//! `__u32 handle_bytes; int handle_type; unsigned char f_handle[]`):
//! `handle_bytes` is the room in `f_handle` on entry (at most
//! [`MAX_HANDLE_SZ`]) and the handle's size on return. What the bytes mean
//! is the file system's business (the kernel's `LinuxFs::name_to_handle`).
//!
//! - **name_to_handle_at** (its spec sheet): unknown `flags` `EINVAL`;
//!   `handle_bytes` above 128 `EINVAL`; the path (a final symbolic link is
//!   followed only with `AT_SYMLINK_FOLLOW`; `AT_EMPTY_PATH` with "" names
//!   `dirfd`'s file); a file system that makes no handles `EOPNOTSUPP`;
//!   room for fewer bytes than the handle has `EOVERFLOW`, with
//!   `handle_bytes` set to the size needed. `*mount_id` gets the mount's
//!   id as an `int`, or with `AT_HANDLE_MNT_ID_UNIQUE` its unique id as a
//!   `u64` ([`unique_mount_id`]). `AT_HANDLE_FID` and
//!   `AT_HANDLE_CONNECTABLE` change nothing for the file systems Poly
//!   makes handles for.
//! - **open_by_handle_at**: the header (`EFAULT`), `handle_bytes` 0 or
//!   above 128 (`EINVAL`), then `CAP_DAC_READ_SEARCH` (`EPERM`), the bytes
//!   (`EFAULT`), the descriptor limit, then the kernel: `mount_fd`
//!   (`EBADF`), the handle (`ESTALE` when it names no live file on that
//!   file system) and the open. `flags` are open(2)'s; as there,
//!   `O_LARGEFILE` is added to all but `O_PATH` descriptors.

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::fs::at::{AT_EMPTY_PATH, AT_SYMLINK_FOLLOW};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::sys::path::{O_LARGEFILE, O_PATH, fs};
use crate::sys::xattrat::at_path;
use crate::table::{Handler, nr};
use alloc::vec::Vec;

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] =
    &[(nr::NAME_TO_HANDLE_AT, name_to_handle_at), (nr::OPEN_BY_HANDLE_AT, open_by_handle_at)];

/// `MAX_HANDLE_SZ` (the man page of name_to_handle_at; glibc
/// `bits/fcntl-linux.h`).
pub const MAX_HANDLE_SZ: u32 = 128;
/// name_to_handle_at's flags (UAPI `linux/fcntl.h`).
pub const AT_HANDLE_MNT_ID_UNIQUE: u32 = 0x001;
pub const AT_HANDLE_CONNECTABLE: u32 = 0x002;
pub const AT_HANDLE_FID: u32 = 0x200;
/// Every flag name_to_handle_at accepts.
pub const HANDLE_FLAGS: u32 =
    AT_EMPTY_PATH | AT_SYMLINK_FOLLOW | AT_HANDLE_MNT_ID_UNIQUE | AT_HANDLE_CONNECTABLE | AT_HANDLE_FID;
/// The size of `struct file_handle` without its bytes.
const HEADER: usize = 8;

/// A file handle: the file system's type and bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileHandle {
    pub kind: i32,
    pub bytes: Vec<u8>,
}

impl FileHandle {
    /// `struct file_handle` with this handle.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(HEADER + self.bytes.len());
        b.extend_from_slice(&(self.bytes.len() as u32).to_le_bytes());
        b.extend_from_slice(&self.kind.to_le_bytes());
        b.extend_from_slice(&self.bytes);
        b
    }
}

/// The unique 64-bit id of the mount whose `/proc/<pid>/mountinfo` id is
/// `id`: the one statx(2) reports for `STATX_MNT_ID_UNIQUE` in
/// `stx_mnt_id` ([`crate::mountapi::unique_id`]); both must agree
/// (name_to_handle_at's spec sheet compares them).
pub const fn unique_mount_id(id: u64) -> u64 {
    crate::mountapi::unique_id(id)
}

fn read_u32(k: &dyn LinuxKernel, p: UserPtr) -> Result<u32, Errno> {
    let mut b = [0u8; 4];
    k.copy_from_user(p, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// name_to_handle_at(dirfd, pathname, handle, mount_id, flags)
pub fn name_to_handle_at(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (dirfd, handle, mount_id, flags) = (a.fd(0), a.ptr(2), a.ptr(3), a.uint(4));
    if flags & !HANDLE_FLAGS != 0 {
        return Err(Errno::EINVAL);
    }
    let room = read_u32(k, handle)?;
    if room > MAX_HANDLE_SZ {
        return Err(Errno::EINVAL);
    }
    let path = at_path(k, a.ptr(1), flags & AT_EMPTY_PATH)?;
    let (h, mnt) = fs(k)?.name_to_handle(dirfd, &path, flags & AT_SYMLINK_FOLLOW != 0)?;
    let needed = h.bytes.len() as u32;
    if room < needed {
        k.copy_to_user(handle, &needed.to_le_bytes())?;
        return Err(Errno::EOVERFLOW);
    }
    k.copy_to_user(handle, &h.to_bytes())?;
    if flags & AT_HANDLE_MNT_ID_UNIQUE != 0 {
        k.copy_to_user(mount_id, &unique_mount_id(mnt).to_le_bytes())?;
    } else {
        k.copy_to_user(mount_id, &(mnt as i32).to_le_bytes())?;
    }
    Ok(0)
}

/// open_by_handle_at(mount_fd, handle, flags)
pub fn open_by_handle_at(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (mount_fd, handle, flags) = (a.fd(0), a.ptr(1), a.uint(2));
    let mut header = [0u8; HEADER];
    k.copy_from_user(handle, &mut header)?;
    let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let kind = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if len == 0 || len > MAX_HANDLE_SZ {
        return Err(Errno::EINVAL);
    }
    if !cred::current(k).capable(cap::DAC_READ_SEARCH) {
        return Err(Errno::EPERM);
    }
    let mut bytes = alloc::vec![0u8; len as usize];
    k.copy_from_user(handle.offset(HEADER as u64).ok_or(Errno::EFAULT)?, &mut bytes)?;
    fs(k)?.check_fd_free()?;
    let flags = if flags & O_PATH == 0 { flags | O_LARGEFILE } else { flags };
    fs(k)?.open_by_handle(mount_fd, &FileHandle { kind, bytes }, flags).map(|fd| fd as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_file_handle() {
        let h = FileHandle { kind: 1, bytes: alloc::vec![9, 8, 7] };
        assert_eq!(h.to_bytes(), [3, 0, 0, 0, 1, 0, 0, 0, 9, 8, 7]);
        assert_eq!(HANDLE_FLAGS & 0x8000, 0, "the spec sheet's unknown flag");
        assert_eq!(unique_mount_id(42), crate::mountapi::unique_id(42));
    }
}
