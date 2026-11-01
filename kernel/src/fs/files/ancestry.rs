//! What Landlock needs of [`Files`]: the chain of objects a path or a
//! descriptor names, up to the process root (`poly_vfs::ancestry`).

use super::Files;
use alloc::vec::Vec;
use poly_vfs::ancestry::Ancestry;
use poly_vfs::inotify::WatchKey;
use poly_vfs::{Fd, FileType, Result};

impl Files {
    /// The chain of the object `path` names from `dirfd`.
    pub fn path_ancestry(&self, dirfd: Fd, path: &[u8], follow: bool) -> Result<Ancestry> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.path_ancestry(ctx, &self.cred(), at, path, follow))
    }

    /// The chain of the directory the last component of `path` is in.
    pub fn parent_ancestry(&self, dirfd: Fd, path: &[u8]) -> Result<Vec<WatchKey>> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.parent_ancestry(ctx, &self.cred(), at, path))
    }

    /// The object `fd` is open on and its type; `None` for a file that is
    /// on no file system's tree (a pipe, a socket, an event file).
    pub fn fd_object(&self, fd: Fd) -> Result<Option<(WatchKey, FileType)>> {
        let file = self.file(fd)?;
        if file.object().is_some() {
            return Ok(None);
        }
        let node = file.node();
        Ok(Some((WatchKey::of_node(node), node.kind()?)))
    }
}
