//! More of [`Files`]: mounting and unmounting where the process names it,
//! device nodes and `chroot`, for the Linux personality's administration
//! calls (ADR 0051). Paths resolve from the process's root and working
//! directory, as the calling thread's credentials (`Files::cred`).

use super::Files;
use crate::fs::vfs;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{AT_FDCWD, DeviceId, Fd, FileSystem, FileType, MountFlags, MountId, Result};

impl Files {
    /// mount(2) of a new file system on the directory `target`.
    pub fn mount_fs(&self, target: &[u8], fs: Arc<dyn FileSystem>, flags: MountFlags) -> Result<MountId> {
        self.at(AT_FDCWD, target, |vfs, ctx, at| vfs.mount(ctx, &self.cred(), at, target, fs, flags))
    }

    /// mount(2) with `MS_BIND`.
    pub fn mount_bind(&self, source: &[u8], target: &[u8], flags: MountFlags) -> Result<MountId> {
        let ctx = self.context();
        let cred = self.cred();
        vfs().bind(&ctx, &cred, poly_vfs::At::Cwd, source, poly_vfs::At::Cwd, target, flags)
    }

    /// mount(2) with `MS_REMOUNT`.
    pub fn remount(&self, target: &[u8], flags: MountFlags) -> Result<()> {
        self.at(AT_FDCWD, target, |vfs, ctx, at| vfs.remount(ctx, &self.cred(), at, target, flags))
    }

    /// The mount whose root `target` names (`EINVAL` if it is not one).
    pub fn mount_at(&self, target: &[u8], follow: bool) -> Result<MountId> {
        self.at(AT_FDCWD, target, |vfs, ctx, at| vfs.mount_at(ctx, &self.cred(), at, target, follow))
    }

    /// umount2(2) of the mount at `target` (`lazy`: `MNT_DETACH`).
    pub fn umount(&self, target: &[u8], lazy: bool) -> Result<()> {
        self.at(AT_FDCWD, target, |vfs, ctx, at| vfs.umount(ctx, &self.cred(), at, target, lazy))
    }

    /// Where mount `id` is, as a path from the process's root.
    pub fn mount_path(&self, id: MountId) -> Result<Vec<u8>> {
        vfs().mount_path(&self.context(), id)
    }

    /// mknodat(2): `mode` is the permission bits (the umask applies).
    pub fn mknodat(&self, dirfd: Fd, path: &[u8], kind: FileType, mode: u16, rdev: DeviceId) -> Result<()> {
        self.at(dirfd, path, |vfs, ctx, at| vfs.mknod(ctx, &self.cred(), at, path, kind, mode, rdev))
    }

    /// unshare(2) / clone(2) `CLONE_NEWNS` (ADR 0054): the process moves to a copy of
    /// its mount namespace; returns the copy.
    pub fn copy_mount_namespace(&self) -> Result<MountId> {
        vfs().copy_namespace(&mut self.ctx.lock())
    }

    /// setns(2) into mount namespace `base`: root and working directory
    /// move to its root.
    pub fn enter_mount_namespace(&self, base: MountId) -> Result<()> {
        vfs().enter_namespace(&mut self.ctx.lock(), base)
    }

    /// pivot_root(2) (ADR 0122): `new_root`'s mount becomes the first of
    /// the process's mount namespace, the old one moves to `put_old`; the
    /// root and working directory move with it where they were the old
    /// root, and so do those of every other process that used the old
    /// root (the same mount: processes of the namespace).
    pub fn pivot_root(&self, new_root: &poly_vfs::NodeRef, put_old: &poly_vfs::NodeRef) -> Result<()> {
        let cred = self.cred();
        let (old, new) = vfs().pivot_root(&mut self.ctx.lock(), &cred, new_root, put_old)?;
        for p in crate::process::all() {
            if !core::ptr::eq(&p.files, self) {
                poly_vfs::Vfs::repoint_context(&mut p.files.ctx.lock(), &old, &new);
            }
        }
        Ok(())
    }

    /// Whether the root directory is not its mount namespace's root
    /// (chroot(2) moved it).
    pub fn chrooted(&self) -> bool {
        vfs().is_chrooted(&self.ctx.lock())
    }

    /// chroot(2).
    pub fn chroot(&self, path: &[u8]) -> Result<()> {
        // Resolved unlocked, like chdir.
        let dir = self.lookupat(AT_FDCWD, path, true)?;
        self.ctx.lock().set_root(&self.cred(), dir)
    }
}
