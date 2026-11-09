//! Extended attributes for the Linux personality (`LinuxFs::getxattr` and
//! friends): the target named by a path or a descriptor, then the VFS's
//! checks and the file system's store (`poly_vfs::xattr`), with the
//! calling thread's file system credentials.

use super::Thread;
use super::services::vfs_errno;
use alloc::vec::Vec;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::fs::XattrTarget;
use poly_vfs::{AT_FDCWD, Cred, NodeRef};

/// The node and credentials an attribute call works with. A descriptor
/// opened with `O_PATH` is not open for I/O: `EBADF`.
fn target(t: &Thread<'_>, target: XattrTarget<'_>) -> Result<(NodeRef, Cred), Errno> {
    let files = &t.process().files;
    let node = match target {
        XattrTarget::Path { path, follow } => files.lookupat(AT_FDCWD, path, follow).map_err(vfs_errno)?,
        XattrTarget::Fd(fd) => open_node(files, fd)?,
        XattrTarget::At { dirfd: AT_FDCWD, path: b"", .. } => files.node_at(AT_FDCWD).map_err(vfs_errno)?,
        XattrTarget::At { dirfd, path: b"", .. } => open_node(files, dirfd)?,
        XattrTarget::At { dirfd, path, follow } => files.lookupat(dirfd, path, follow).map_err(vfs_errno)?,
    };
    Ok((node, files.cred()))
}

/// The node descriptor `fd` is open on, not with `O_PATH` (`EBADF`).
fn open_node(files: &crate::fs::Files, fd: i32) -> Result<NodeRef, Errno> {
    let file = files.file(fd).map_err(vfs_errno)?;
    if file.is_path_only() {
        return Err(Errno::EBADF);
    }
    Ok(file.node().clone())
}

pub(super) fn get(t: &Thread<'_>, at: XattrTarget<'_>, name: &[u8]) -> Result<Vec<u8>, Errno> {
    let (node, cred) = target(t, at)?;
    node.getxattr(&cred, name).map_err(vfs_errno)
}

pub(super) fn set(t: &Thread<'_>, at: XattrTarget<'_>, name: &[u8], value: &[u8], flags: u32) -> Result<(), Errno> {
    let (node, cred) = target(t, at)?;
    node.setxattr(&cred, name, value, flags).map_err(vfs_errno)
}

pub(super) fn list(t: &Thread<'_>, at: XattrTarget<'_>) -> Result<Vec<u8>, Errno> {
    let (node, cred) = target(t, at)?;
    node.listxattr(&cred).map_err(vfs_errno)
}

pub(super) fn remove(t: &Thread<'_>, at: XattrTarget<'_>, name: &[u8]) -> Result<(), Errno> {
    let (node, cred) = target(t, at)?;
    node.removexattr(&cred, name).map_err(vfs_errno)
}
