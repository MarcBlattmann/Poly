//! The permission side of [`Files`] (ADR 0045): what a write does to
//! set-user-ID and set-group-ID bits, `O_NOATIME`, and the ownership and
//! time calls (`chown`, `utimensat`). Every check is made as the calling
//! thread ([`Files::cred`]).

use super::Files;
use poly_vfs::{Errno, Fd, FileType, NodeRef, OpenFile, OpenFlags, Result, S_ISGID, S_ISUID, SetTime};

impl Files {
    /// Before `len` bytes are written to `file` (or it is truncated): a
    /// writer without `CAP_FSETID` clears the set-user-ID bit, and the
    /// set-group-ID bit of a group-executable file (POSIX write(): "the
    /// S_ISUID and S_ISGID bits of the file mode may be cleared";
    /// `NodeRef::clear_setid`). Only the mode is read when neither bit is
    /// set, which is nearly always.
    pub fn before_write(&self, file: &OpenFile, len: usize) -> Result<()> {
        if len == 0 || file.kind() != FileType::Regular || !file.writable() {
            return Ok(());
        }
        let attr = file.node().attr()?;
        if attr.perm & (S_ISUID | S_ISGID) == 0 {
            return Ok(());
        }
        file.node().clear_setid(&self.cred())
    }

    /// `fcntl(F_SETFL)` turning `O_NOATIME` on: the caller must own the
    /// file or have `CAP_FOWNER` (`EPERM`).
    pub(super) fn check_noatime(&self, file: &OpenFile, flags: OpenFlags) -> Result<()> {
        if !flags.contains(OpenFlags::NOATIME) || file.flags().contains(OpenFlags::NOATIME) {
            return Ok(());
        }
        let attr = file.node().attr()?;
        if self.cred().owns(&attr) { Ok(()) } else { Err(Errno::EPERM) }
    }

    /// The node `path` names from `dirfd`, or with an empty `path` what
    /// `dirfd` is open on (`AT_EMPTY_PATH`).
    fn target(&self, dirfd: Fd, path: &[u8], follow: bool) -> Result<NodeRef> {
        if path.is_empty() { self.node_at(dirfd) } else { self.lookupat(dirfd, path, follow) }
    }

    /// The node `fd` is open on, not for `O_PATH` (`EBADF`).
    fn open_node(&self, fd: Fd) -> Result<NodeRef> {
        let file = self.file(fd)?;
        if file.is_path_only() { Err(Errno::EBADF) } else { Ok(file.node().clone()) }
    }

    /// `fchownat(2)`: `None` leaves an id alone; an empty `path` is
    /// `dirfd`'s file.
    pub fn fchownat(&self, dirfd: Fd, path: &[u8], uid: Option<u32>, gid: Option<u32>, follow: bool) -> Result<()> {
        self.target(dirfd, path, follow)?.chown(&self.cred(), uid, gid)
    }

    /// `fchown(2)`.
    pub fn fchown(&self, fd: Fd, uid: Option<u32>, gid: Option<u32>) -> Result<()> {
        self.open_node(fd)?.chown(&self.cred(), uid, gid)
    }

    /// `utimensat(2)`: `None` leaves a time alone; `path` `None` is the
    /// file `dirfd` is open on (futimens(3)), an empty one `AT_EMPTY_PATH`.
    pub fn utimensat(
        &self,
        dirfd: Fd,
        path: Option<&[u8]>,
        atime: Option<SetTime>,
        mtime: Option<SetTime>,
        follow: bool,
    ) -> Result<()> {
        let node = match path {
            None => self.open_node(dirfd)?,
            Some(p) => self.target(dirfd, p, follow)?,
        };
        node.set_times(&self.cred(), atime, mtime)
    }
}
