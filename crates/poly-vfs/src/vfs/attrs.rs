//! Inspecting and changing attributes.

use super::*;

impl Vfs {
    /// `stat(2)` (`follow`) and `lstat(2)`.
    pub fn stat(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], follow: bool) -> Result<Stat> {
        let loc = self.resolve(ctx, cred, at, path, follow)?;
        Ok(Stat::new(loc.mount.dev(), &loc.attr()?))
    }

    /// `readlink(2)`: `EINVAL` unless `path` names a symbolic link.
    pub fn readlink(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<Vec<u8>> {
        let loc = self.resolve(ctx, cred, at, path, false)?;
        loc.mount.sb.fs.readlink(loc.ino)
    }

    /// `faccessat(2)`. `cred` should hold the real ids unless the caller
    /// asked for `AT_EACCESS`.
    pub fn access(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        want: Access,
        follow: bool,
    ) -> Result<()> {
        self.lookup(ctx, cred, at, path, follow)?.access(cred, want)
    }

    /// `chmod(2)`.
    pub fn chmod(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], perm: u16, follow: bool) -> Result<()> {
        let (node, named) = self.lookup_named(ctx, cred, at, path, follow)?;
        node.chmod(cred, perm)?;
        Self::changed_in_dir(&node, named, inotify::mask::IN_ATTRIB);
        Ok(())
    }

    /// [`lookup`](Self::lookup), and the directory and name the node was
    /// found under (for inotify(7)'s events in that directory).
    pub(super) fn lookup_named(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        follow: bool,
    ) -> Result<(NodeRef, Named)> {
        check_path(path)?;
        let (loc, named) = self.walk(ctx, cred).resolve_named(Self::start(ctx, at), path, follow)?;
        Ok((loc.pin()?, named))
    }

    /// inotify(7): `mask` (an attribute change or modification of `node`)
    /// for the watches of the directory it was named in.
    pub(super) fn changed_in_dir(node: &NodeRef, named: Named, mask: u32) {
        if inotify::active()
            && let Some((dir, name)) = named
        {
            let is_dir = node.kind().is_ok_and(|k| k == FileType::Directory);
            inotify::entry_event(&dir, &name, mask, is_dir, 0);
        }
    }

    /// `chown(2)` / `lchown(2)`; `None` leaves an id alone.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn chown(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        uid: Option<u32>,
        gid: Option<u32>,
        follow: bool,
    ) -> Result<()> {
        let (node, named) = self.lookup_named(ctx, cred, at, path, follow)?;
        node.chown(cred, uid, gid)?;
        Self::changed_in_dir(&node, named, inotify::mask::IN_ATTRIB);
        Ok(())
    }

    /// `utimensat(2)` on a path: `None` leaves a time alone (`UTIME_OMIT`);
    /// see [`NodeRef::set_times`] for who may.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn utimens(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        atime: Option<crate::SetTime>,
        mtime: Option<crate::SetTime>,
        follow: bool,
    ) -> Result<()> {
        let (node, named) = self.lookup_named(ctx, cred, at, path, follow)?;
        node.set_times(cred, atime, mtime)?;
        Self::changed_in_dir(&node, named, inotify::mask::IN_ATTRIB);
        Ok(())
    }

    /// `truncate(2)`.
    pub fn truncate(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], size: u64) -> Result<()> {
        let (node, named) = self.lookup_named(ctx, cred, at, path, true)?;
        node.truncate(cred, size)?;
        Self::changed_in_dir(&node, named, inotify::mask::IN_MODIFY);
        Ok(())
    }
}
