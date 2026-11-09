//! sysfs(2) and ustat(2), the two obsolete calls about file systems
//! (ADR 0135).
//!
//! sysfs(2) numbers the registered file system types, in the order of
//! `/proc/filesystems` ([`FILE_SYSTEM_TYPES`], which procfs shows too).
//! ustat(2) reports the free space of the mounted file system whose
//! device number it is given, from the kernel's statfs.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs;
use crate::kernel::LinuxKernel;
use crate::sys::io::bounce;
use crate::sys::path::{PATH_MAX, fs as kernel_fs};
use crate::table::{Handler, nr};

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::SYSFS, sysfs), (nr::USTAT, ustat)];

/// The file system types, in the order `/proc/filesystems` lists them,
/// with whether each has no device (`nodev`).
pub const FILE_SYSTEM_TYPES: &[(&str, bool)] = &[
    ("sysfs", true),
    ("tmpfs", true),
    ("proc", true),
    ("ext2", false),
    ("ext3", false),
    ("ext4", false),
    ("squashfs", false),
    ("overlay", false),
];

/// sysfs(option, ...): option 1, the index of the type named by the
/// string; option 2, the name of type `index` (a 32-bit value) written
/// with its NUL to the buffer; option 3, the number of types.
pub fn sysfs(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    match a.int(0) {
        1 => {
            let mut buf = bounce(PATH_MAX)?;
            let n = k.strncpy_from_user(a.ptr(1), &mut buf)?;
            if n == 0 {
                return Err(Errno::ENOENT);
            }
            FILE_SYSTEM_TYPES
                .iter()
                .position(|(name, _)| name.as_bytes() == &buf[..n])
                .map(|i| i as u64)
                .ok_or(Errno::EINVAL)
        }
        2 => {
            let (name, _) = FILE_SYSTEM_TYPES.get(a.uint(1) as usize).ok_or(Errno::EINVAL)?;
            let mut out = name.as_bytes().to_vec();
            out.push(0);
            k.copy_to_user(a.ptr(2), &out)?;
            Ok(0)
        }
        3 => Ok(FILE_SYSTEM_TYPES.len() as u64),
        _ => Err(Errno::EINVAL),
    }
}

/// `sizeof(struct ustat)` on x86-64 (`linux/types.h`'s layout as the sheet
/// gives it): `f_tfree` (i64), `f_tinode` (u64), `f_fname[6]`,
/// `f_fpack[6]`, padding.
pub const USTAT_SIZE: usize = 32;

/// The bytes of a `struct ustat` for a file system with these free
/// counts; the name and pack fields are zero.
pub fn ustat_bytes(free_blocks: u64, free_inodes: u64) -> [u8; USTAT_SIZE] {
    let mut b = [0u8; USTAT_SIZE];
    b[0..8].copy_from_slice(&free_blocks.to_le_bytes());
    b[8..16].copy_from_slice(&free_inodes.to_le_bytes());
    b
}

/// ustat(dev, ubuf): `dev` is a 32-bit `st_dev`; no mounted file system
/// with it is `EINVAL`, even for an unwritable buffer.
pub fn ustat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let dev = u64::from(a.uint(0));
    let st = kernel_fs(k)?.statfs_dev(fs::major(dev), fs::minor(dev))?;
    k.copy_to_user(a.ptr(1), &ustat_bytes(st.bfree, st.ffree))?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ustat_layout() {
        let b = ustat_bytes(5, 7);
        assert_eq!(&b[0..8], &5u64.to_le_bytes());
        assert_eq!(&b[8..16], &7u64.to_le_bytes());
        assert!(b[16..].iter().all(|&x| x == 0));
    }

    #[test]
    fn the_types_are_the_proc_filesystems_list() {
        assert_eq!(FILE_SYSTEM_TYPES[0].0, "sysfs");
        assert!(FILE_SYSTEM_TYPES.iter().any(|(n, _)| *n == "proc"));
    }
}
