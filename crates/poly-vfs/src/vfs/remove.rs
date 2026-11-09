//! Removing and renaming.

use super::*;

impl Vfs {
    /// `unlink(2)`.
    pub fn unlink(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<()> {
        let p = self.parent_of(ctx, cred, at, path)?;
        // The order (the spec of unlink): a last "." or ".." EISDIR, a
        // read-only mount EROFS (also for a missing name), the last name
        // (missing, then a trailing slash after a non-directory), then
        // write permission on the parent, the sticky bit, a directory
        // (EISDIR) and a mount point (EBUSY).
        let Last::Name(name) = &p.last else { return Err(Errno::EISDIR) };
        if p.dir.mount.read_only() {
            return Err(Errno::EROFS);
        }
        let victim = self.raw_child(&p.dir, name)?;
        let vattr = victim.attr()?;
        if p.trailing_slash && vattr.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let dattr = self.may_change(cred, &p.dir)?;
        Self::sticky_allows(cred, &dattr, &vattr)?;
        if vattr.kind == FileType::Directory {
            return Err(Errno::EISDIR);
        }
        crate::fileattr::check_remove(&victim)?;
        if self.is_mount_point(&victim) {
            return Err(Errno::EBUSY);
        }
        let sb = &p.dir.mount.sb;
        sb.forget(p.dir.ino, name);
        self.name_changed(sb.fs.unlink(p.dir.ino, name))?;
        inotify::removed(&p.dir, name, &victim, false);
        Ok(())
    }

    /// `rmdir(2)`: `/` is `EBUSY`, a final `.` is `EINVAL`, `..` is
    /// `ENOTEMPTY`, a mount point or the caller's root is `EBUSY`.
    pub fn rmdir(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<()> {
        let p = self.parent_of(ctx, cred, at, path)?;
        let name = match &p.last {
            Last::Root => return Err(Errno::EBUSY),
            Last::Dot => return Err(Errno::EINVAL),
            Last::DotDot => return Err(Errno::ENOTEMPTY),
            Last::Name(name) => name,
        };
        // Then (the spec of rmdir): a read-only mount EROFS before the last
        // name is looked up, the name (missing ENOENT), write permission on
        // the parent, the sticky bit, not a directory, a mount point or
        // the caller's root (EBUSY), and the file system's ENOTEMPTY.
        if p.dir.mount.read_only() {
            return Err(Errno::EROFS);
        }
        let victim = self.raw_child(&p.dir, name)?;
        let vattr = victim.attr()?;
        let dattr = self.may_change(cred, &p.dir)?;
        Self::sticky_allows(cred, &dattr, &vattr)?;
        if vattr.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        crate::fileattr::check_remove(&victim)?;
        if self.is_mount_point(&victim) || victim.same(ctx.root.loc()) {
            return Err(Errno::EBUSY);
        }
        let sb = &p.dir.mount.sb;
        sb.forget(p.dir.ino, name);
        self.name_changed(sb.fs.rmdir(p.dir.ino, name))?;
        inotify::removed(&p.dir, name, &victim, true);
        Ok(())
    }

    /// `renameat2(2)`.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn rename(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        old_at: At<'_>,
        old: &[u8],
        new_at: At<'_>,
        new: &[u8],
        flags: RenameFlags,
    ) -> Result<()> {
        self.rename_paths(ctx, cred, PathArg::new(old_at, old), PathArg::new(new_at, new), flags)
    }

    /// [`rename`](Self::rename) with each path's errors in their place
    /// (renameat2's `path_order_two_paths`): the directories first, the
    /// last names after them. (1) `old`'s string, start and every component
    /// but the last; (2) the same for `new`; (3) `old`'s last name (over
    /// [`NAME_MAX`](crate::NAME_MAX) bytes `ENAMETOOLONG`, missing
    /// `ENOENT`); (4) `new`'s last name (`ENAMETOOLONG`); then the errors of
    /// the rename itself (`EXDEV`, `EISDIR`, ...). A last component that is
    /// `/`, `.` or `..` on either side is `EBUSY`, before (3).
    pub fn rename_paths(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        old: PathArg<'_>,
        new: PathArg<'_>,
        flags: RenameFlags,
    ) -> Result<()> {
        if flags.noreplace && flags.exchange {
            return Err(Errno::EINVAL);
        }
        // The order (the spec of rename): both paths' directories, EXDEV,
        // a last "."/".."/"/" (EBUSY), EROFS, the last names, the trailing
        // slashes, then the rename's own errors.
        let op = self.dirs_of(ctx, cred, old)?;
        let np = self.dirs_of(ctx, cred, new)?;
        if !Arc::ptr_eq(&op.dir.mount, &np.dir.mount) {
            return Err(Errno::EXDEV);
        }
        let (Last::Name(old_name), Last::Name(new_name)) = (&op.last, &np.last) else {
            return Err(Errno::EBUSY);
        };
        if op.dir.mount.read_only() {
            return Err(Errno::EROFS);
        }
        op.check_name()?;
        let src = self.raw_child(&op.dir, old_name)?;
        np.check_name()?;
        let sattr = src.attr()?;
        let dst = match self.raw_child(&np.dir, new_name) {
            Ok(d) => Some(d),
            Err(Errno::ENOENT) if !flags.exchange => None,
            Err(e) => return Err(e),
        };
        let dattr = dst.as_ref().map(Location::attr).transpose()?;
        let src_dir = sattr.kind == FileType::Directory;
        let dst_dir = dattr.is_some_and(|a| a.kind == FileType::Directory);
        // A trailing slash demands a directory on that side (for the
        // target: the source, unless exchanging).
        if (op.trailing_slash && !src_dir) || (np.trailing_slash && !(if flags.exchange { dst_dir } else { src_dir })) {
            return Err(Errno::ENOTDIR);
        }
        if let Some(dattr) = &dattr {
            // RENAME_NOREPLACE: an existing target, even the source's own
            // name or another link of it, is EEXIST (the spec of renameat2).
            if flags.noreplace {
                return Err(Errno::EEXIST);
            }
            if dattr.ino == sattr.ino {
                // Two names of one inode (or the same name): nothing to do.
                return Ok(());
            }
        }
        // A directory cannot move below itself.
        if src_dir && self.is_within(&np.dir, src.ino)? {
            return Err(Errno::EINVAL);
        }
        if flags.exchange && dst_dir && self.is_within(&op.dir, dattr.as_ref().map_or(0, |a| a.ino))? {
            return Err(Errno::EINVAL);
        }
        // A directory over one of its own ancestors: that target is not
        // empty (it holds the source).
        if !flags.exchange && src_dir && dst_dir && self.is_within(&op.dir, dattr.as_ref().map_or(0, |a| a.ino))? {
            return Err(Errno::ENOTEMPTY);
        }
        let old_dattr = self.may_change(cred, &op.dir)?;
        let new_dattr = self.may_change(cred, &np.dir)?;
        Self::sticky_allows(cred, &old_dattr, &sattr)?;
        if let Some(dattr) = &dattr {
            Self::sticky_allows(cred, &new_dattr, dattr)?;
        }
        // Moving a directory to another parent rewrites its "..", which
        // needs write permission on it (`rename(2)`, EACCES).
        let moves_parent = op.dir.ino != np.dir.ino;
        if moves_parent && src_dir {
            cred.check(&sattr, Access::WRITE)?;
        }
        if moves_parent && flags.exchange && dst_dir {
            cred.check(dattr.as_ref().ok_or(Errno::ENOENT)?, Access::WRITE)?;
        }
        // Then the kinds, a non-empty target (the file system's), a mount
        // point.
        if dattr.is_some() && !flags.exchange {
            if src_dir && !dst_dir {
                return Err(Errno::ENOTDIR);
            }
            if !src_dir && dst_dir {
                return Err(Errno::EISDIR);
            }
        }
        if self.is_mount_point(&src) || dst.as_ref().is_some_and(|d| self.is_mount_point(d)) {
            return Err(Errno::EBUSY);
        }
        let sb = &op.dir.mount.sb;
        sb.forget(op.dir.ino, old_name);
        sb.forget(np.dir.ino, new_name);
        self.name_changed(sb.fs.rename(op.dir.ino, old_name, np.dir.ino, new_name, flags))?;
        if inotify::active() {
            use inotify::mask::{IN_MOVE_SELF, IN_MOVED_FROM, IN_MOVED_TO};
            let cookie = inotify::next_cookie();
            inotify::entry_event(&op.dir, old_name, IN_MOVED_FROM, src_dir, cookie);
            inotify::entry_event(&np.dir, new_name, IN_MOVED_TO, src_dir, cookie);
            inotify::self_event(&src, IN_MOVE_SELF, src_dir);
            if let Some(dst) = &dst {
                if flags.exchange {
                    let cookie = inotify::next_cookie();
                    inotify::entry_event(&np.dir, new_name, IN_MOVED_FROM, dst_dir, cookie);
                    inotify::entry_event(&op.dir, old_name, IN_MOVED_TO, dst_dir, cookie);
                    inotify::self_event(dst, IN_MOVE_SELF, dst_dir);
                } else {
                    // The replaced inode may be gone with its name.
                    inotify::inode_may_be_gone(dst);
                }
            }
        }
        Ok(())
    }

    /// Whether `dir` is the directory `ancestor` or below it, within one
    /// file system (walking `..` up to the file system's root).
    pub(super) fn is_within(&self, dir: &Location, ancestor: Ino) -> Result<bool> {
        let fs = &dir.mount.sb.fs;
        let root = fs.root();
        let mut cur = dir.ino;
        // Bounded so a corrupt file system cannot loop us forever.
        for _ in 0..1 << 16 {
            if cur == ancestor {
                return Ok(true);
            }
            if cur == root {
                return Ok(false);
            }
            let up = fs.lookup(cur, b"..")?;
            if up == cur {
                return Ok(false);
            }
            cur = up;
        }
        Err(Errno::ELOOP)
    }
}
