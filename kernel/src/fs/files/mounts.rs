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

    /// chroot(2).
    pub fn chroot(&self, path: &[u8]) -> Result<()> {
        // Resolved unlocked, like chdir.
        let dir = self.lookupat(AT_FDCWD, path, true)?;
        self.ctx.lock().set_root(&self.cred(), dir)
    }
}
