//! memfd_secret(2) and cachestat(2) for the running thread (ADR 0122).

use super::Thread;
use super::services::vfs_errno;
use crate::fs::{self, FileType};
use poly_personality_linux::errno::Errno;
use poly_vfs::memfd::Memfd;

/// memfd_secret(2): a secret memory file (`Memfd::secret`) on the memfd
/// file system, mode 0600, installed as a descriptor.
pub fn memfd_secret(t: &Thread<'_>, cloexec: bool) -> Result<i32, Errno> {
    let files = &t.process().files;
    let file =
        fs::vfs().memfd_file(&fs::memfs::memfd_mount(), &files.cred(), 0o600, Memfd::secret()).map_err(vfs_errno)?;
    files.install(file, cloexec).map_err(vfs_errno)
}

/// cachestat(2): the pages `[first, end)` of the regular file behind `fd`
/// that hold data (its file system's `data_pages`); 0 for every other
/// kind of descriptor. `EBADF` for an `O_PATH` descriptor.
pub fn cachestat(t: &Thread<'_>, fd: i32, first: u64, end: u64) -> Result<u64, Errno> {
    let file = t.process().files.file(fd).map_err(vfs_errno)?;
    if file.is_path_only() {
        return Err(Errno::EBADF);
    }
    if file.kind() != FileType::Regular || file.object().is_some() || file.device().is_some() {
        return Ok(0);
    }
    let node = file.node();
    Ok(node.mount().fs().data_pages(node.ino(), first, end).map_or(0, |pages| pages.len() as u64))
}
