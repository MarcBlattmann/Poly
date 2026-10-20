//! The permission side of [`Files`] (ADR 0045): what a write does to
//! set-user-ID and set-group-ID bits, `O_NOATIME`, and lookups with
//! explicit credentials for the attribute calls (`chmod`, `chown`,
//! `access`, `utimensat`), which may check as other ids than the caller's
//! file system ids (access(2) uses the real ones).

use super::{Files, vfs};
use poly_vfs::{AT_FDCWD, At, Cred, Errno, Fd, FileType, NodeRef, OpenFile, OpenFlags, Result, S_ISGID, S_ISUID};

impl Files {
    /// Before `len` bytes are written to `file` (or it is truncated): a
    /// writer without `CAP_FSETID` clears the set-user-ID bit, and the
    /// set-group-ID bit of a group-executable file (POSIX write(): "the
    /// S_ISUID and S_ISGID bits of the file mode may be cleared";
    /// `NodeRef::clear_setid`). Only the mode is read when neither bit is
    /// set, which is nearly always.
    pub(super) fn before_write(&self, file: &OpenFile, len: usize) -> Result<()> {
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

    /// The node `path` names from `dirfd` (as for `openat`), resolved and
    /// checked (search permission on the way) as `cred`.
    pub fn lookup_as(&self, cred: &Cred, dirfd: Fd, path: &[u8], follow: bool) -> Result<NodeRef> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.lookup(ctx, cred, at, path, follow))
    }

    /// What `dirfd` names with an empty path and `AT_EMPTY_PATH`: the node
    /// it is open on (`O_PATH` included), or the working directory for
    /// `AT_FDCWD`.
    pub fn empty_path(&self, dirfd: Fd) -> Result<NodeRef> {
        if dirfd == AT_FDCWD {
            return Ok(self.ctx.lock().cwd().clone());
        }
        Ok(self.file(dirfd)?.node().clone())
    }

    /// `fchmodat(2)` as the calling thread, for the kernel's own use (the
    /// shell, tests): `path` from `dirfd`.
    pub fn chmodat(&self, dirfd: Fd, path: &[u8], perm: u16) -> Result<()> {
        let cred = self.cred();
        self.lookup_as(&cred, dirfd, path, true)?.chmod(&cred, perm)
    }

    /// `fchownat(2)` as the calling thread; `None` leaves an id alone.
    pub fn chownat(&self, dirfd: Fd, path: &[u8], uid: Option<u32>, gid: Option<u32>) -> Result<()> {
        let cred = self.cred();
        self.lookup_as(&cred, dirfd, path, true)?.chown(&cred, uid, gid)
    }

    /// `access(2)`-style check of `path` as `cred`.
    pub fn access_as(&self, cred: &Cred, path: &[u8], want: poly_vfs::Access) -> Result<()> {
        let ctx = self.ctx.lock();
        vfs().access(&ctx, cred, At::Cwd, path, want, true)
    }
}
