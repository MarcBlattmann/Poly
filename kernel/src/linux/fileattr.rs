//! Inode flags and file handles for the Linux personality
//! (`LinuxFs::file_getattr`, `file_setattr`, `name_to_handle`,
//! `open_by_handle`): the node named by `(dirfd, path)`, then the VFS
//! (`poly_vfs::fileattr`) with the calling thread's credentials.

use super::Thread;
use super::services::vfs_errno;
use crate::fs::Files;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::sys::fileattr::FileAttr;
use poly_personality_linux::sys::handle::FileHandle;
use poly_vfs::{NodeRef, OpenFlags, fileattr};

/// The node `(dirfd, path)` names; an empty path is `dirfd`'s file (any
/// descriptor, `O_PATH` included) or, for `AT_FDCWD`, the working
/// directory.
fn node(files: &Files, dirfd: i32, path: &[u8], follow: bool) -> Result<NodeRef, Errno> {
    if path.is_empty() { files.node_at(dirfd) } else { files.lookupat(dirfd, path, follow) }.map_err(vfs_errno)
}

pub(super) fn get(t: &Thread<'_>, dirfd: i32, path: &[u8], follow: bool) -> Result<FileAttr, Errno> {
    let a = node(&t.process().files, dirfd, path, follow)?.file_getattr().map_err(vfs_errno)?;
    Ok(FileAttr {
        xflags: a.xflags,
        extsize: a.extsize,
        nextents: a.nextents,
        projid: a.projid,
        cowextsize: a.cowextsize,
    })
}

pub(super) fn set(t: &Thread<'_>, dirfd: i32, path: &[u8], follow: bool, attr: &FileAttr) -> Result<(), Errno> {
    let files = &t.process().files;
    let n = node(files, dirfd, path, follow)?;
    let a = fileattr::FileAttr {
        xflags: attr.xflags,
        extsize: attr.extsize,
        nextents: attr.nextents,
        projid: attr.projid,
        cowextsize: attr.cowextsize,
    };
    n.file_setattr(&files.cred(), &a).map_err(vfs_errno)
}

pub(super) fn name_to_handle(
    t: &Thread<'_>,
    dirfd: i32,
    path: &[u8],
    follow: bool,
) -> Result<(FileHandle, u64), Errno> {
    let n = node(&t.process().files, dirfd, path, follow)?;
    let h = n.file_handle().map_err(vfs_errno)?;
    Ok((FileHandle { kind: h.kind, bytes: h.bytes().to_vec() }, u64::from(n.mount().id())))
}

pub(super) fn open_by_handle(t: &Thread<'_>, mount_fd: i32, handle: &FileHandle, flags: u32) -> Result<i32, Errno> {
    let files = &t.process().files;
    let mount = files.node_at(mount_fd).map_err(vfs_errno)?.mount().clone();
    if handle.bytes.len() > fileattr::MAX_HANDLE_SZ {
        return Err(Errno::EINVAL);
    }
    let h = fileattr::FileHandle::new(handle.kind, &handle.bytes);
    let flags = OpenFlags::from_bits(flags);
    let file = crate::fs::vfs().open_handle(&files.cred(), &mount, &h, flags).map_err(vfs_errno)?;
    files.install(file, flags.contains(OpenFlags::CLOEXEC)).map_err(vfs_errno)
}
