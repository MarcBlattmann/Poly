//! Mounts: crossing mount points with names and `..`, stacking, mounting
//! over `/`, bind mounts, `EXDEV`, `EBUSY`, mount flags, and the lookup
//! cache.

use super::{Env, tmpfs};
use crate::*;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

impl Env {
    fn mount(&self, path: &str, fs: Arc<dyn FileSystem>, flags: MountFlags) -> MountId {
        self.vfs.mount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), fs, flags).unwrap()
    }

    fn umount(&self, path: &str) -> Result<()> {
        self.vfs.umount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), false)
    }
}

/// A tmpfs with a file in it.
fn other_fs(file: &str, data: &[u8]) -> Arc<Tmpfs> {
    let fs = tmpfs();
    let vfs = Vfs::new(fs.clone());
    let env = Env { fs: fs.clone(), ctx: vfs.root_context().unwrap(), vfs, root: Cred::root() };
    env.write(file, data);
    fs
}

#[test]
fn crossing_a_mount_point() {
    let mut env = Env::new();
    env.mkdir("/mnt");
    env.write("/mnt/hidden", b"under");
    env.write("/top", b"top");
    let fs = other_fs("/inside", b"mounted");
    env.mount("/mnt", fs, MountFlags::default());
    assert_eq!(env.read("/mnt/inside").unwrap(), b"mounted");
    assert_eq!(env.stat("/mnt/hidden"), Err(Errno::ENOENT));
    assert_ne!(env.stat("/mnt").unwrap().dev, env.stat("/").unwrap().dev);
    assert_eq!(env.ino("/mnt/.."), env.ino("/"));
    assert_eq!(env.read("/mnt/../top").unwrap(), b"top");
    env.mkdir("/mnt/sub");
    env.cd("/mnt/sub").unwrap();
    assert_eq!(env.cwd(), b"/mnt/sub");
    assert_eq!(env.read("../inside").unwrap(), b"mounted");
    assert_eq!(env.read("../../top").unwrap(), b"top");
    env.symlink("/top", "/mnt/sub/up");
    assert_eq!(env.read("up").unwrap(), b"top");
    // Busy while the working directory is inside.
    assert_eq!(env.umount("/mnt"), Err(Errno::EBUSY));
    env.cd("/").unwrap();
    // The mount point itself cannot go away.
    assert_eq!(env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/mnt"), Err(Errno::EBUSY));
    assert_eq!(
        env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/mnt", At::Cwd, b"/mnt2", RenameFlags::default()),
        Err(Errno::EBUSY)
    );
    // Only a mount's root can be unmounted.
    assert_eq!(env.umount("/mnt/sub"), Err(Errno::EINVAL));
    assert_eq!(env.umount("/"), Err(Errno::EBUSY));
    env.umount("/mnt").unwrap();
    assert_eq!(env.read("/mnt/hidden").unwrap(), b"under");
    assert_eq!(env.umount("/mnt"), Err(Errno::EINVAL));
}

#[test]
fn open_files_keep_a_mount_busy_and_lazy_unmount_detaches() {
    let env = Env::new();
    env.mkdir("/mnt");
    env.mount("/mnt", other_fs("/f", b"data"), MountFlags::default());
    let f = env.open("/mnt/f", OpenFlags::RDONLY).unwrap();
    assert_eq!(env.umount("/mnt"), Err(Errno::EBUSY));
    env.vfs.umount(&env.ctx, &env.root, At::Cwd, b"/mnt", true).unwrap();
    assert_eq!(env.stat("/mnt/f"), Err(Errno::ENOENT));
    // The open file still works.
    assert_eq!(super::read_all(&f).unwrap(), b"data");
}

#[test]
fn stacked_mounts() {
    let env = Env::new();
    env.mkdir("/mnt");
    env.mount("/mnt", other_fs("/one", b"1"), MountFlags::default());
    env.mount("/mnt", other_fs("/two", b"2"), MountFlags::default());
    assert_eq!(env.read("/mnt/two").unwrap(), b"2");
    assert_eq!(env.stat("/mnt/one"), Err(Errno::ENOENT));
    assert_eq!(env.ino("/mnt/.."), env.ino("/"));
    // The second mount sits on the first one's root: the first is busy.
    assert_eq!(env.vfs.mounts().len(), 3);
    env.umount("/mnt").unwrap();
    assert_eq!(env.read("/mnt/one").unwrap(), b"1");
    env.umount("/mnt").unwrap();
    assert_eq!(env.vfs.mounts().len(), 1);
}

#[test]
fn mounting_over_the_root() {
    let mut env = Env::new();
    env.write("/old", b"old root");
    env.mkdir("/dir");
    env.cd("/dir").unwrap();
    let new_root = other_fs("/new", b"new root");
    env.mount("/", new_root, MountFlags::default());
    assert_eq!(env.read("/new").unwrap(), b"new root");
    assert_eq!(env.stat("/old"), Err(Errno::ENOENT));
    assert_eq!(env.ino("/.."), env.ino("/"));
    assert_eq!(env.read("/../../new").unwrap(), b"new root");
    // The old working directory still works, and ".." from it climbs to
    // what "/" is now.
    assert!(env.stat(".").is_ok());
    assert_eq!(env.read("../new").unwrap(), b"new root");
    let info = env.vfs.mounts();
    assert_eq!(info[1].parent, Some((info[0].id, info[0].root)));
    assert_eq!(info[1].fs_name, "tmpfs");
}

#[test]
fn bind_mounts() {
    let env = Env::new();
    env.mkdir("/a");
    env.mkdir("/a/b");
    env.write("/a/b/f", b"bound");
    env.mkdir("/mnt");
    env.vfs.bind(&env.ctx, &env.root, At::Cwd, b"/a/b", At::Cwd, b"/mnt", MountFlags::default()).unwrap();
    assert_eq!(env.read("/mnt/f").unwrap(), b"bound");
    let (s1, s2) = (env.stat("/a/b/f").unwrap(), env.stat("/mnt/f").unwrap());
    assert_eq!((s1.dev, s1.ino), (s2.dev, s2.ino));
    env.write("/mnt/g", b"via bind");
    assert_eq!(env.read("/a/b/g").unwrap(), b"via bind");
    // ".." from the bind root leaves the bind mount.
    assert_eq!(env.ino("/mnt/.."), env.ino("/"));
    // Renames and links do not cross mounts, even of one file system.
    let ctx = &env.ctx;
    let r = &env.root;
    assert_eq!(
        env.vfs.rename(ctx, r, At::Cwd, b"/mnt/g", At::Cwd, b"/a/g2", RenameFlags::default()),
        Err(Errno::EXDEV)
    );
    assert_eq!(env.vfs.link(ctx, r, At::Cwd, b"/mnt/g", At::Cwd, b"/a/g2", false), Err(Errno::EXDEV));
    // A file onto a directory is refused.
    env.write("/file", b"");
    assert_eq!(env.vfs.bind(ctx, r, At::Cwd, b"/file", At::Cwd, b"/a", MountFlags::default()), Err(Errno::ENOTDIR));
    assert_eq!(
        env.vfs.bind(ctx, &Cred::user(1, 1), At::Cwd, b"/a", At::Cwd, b"/mnt", MountFlags::default()),
        Err(Errno::EPERM)
    );
}

#[test]
fn mount_flags() {
    let env = Env::new();
    for d in ["/ro", "/nodev", "/noexec"] {
        env.mkdir(d);
    }
    let ro = other_fs("/f", b"read only");
    ro.mkdir(ro.root(), b"d", 0o755, 0, 0).unwrap();
    ro.create(
        ro.root(),
        b"null",
        &NewNode { kind: FileType::CharDevice, perm: 0o666, uid: 0, gid: 0, rdev: device::NULL },
    )
    .unwrap();
    env.vfs.register_char_device(device::NULL, Arc::new(device::Null)).unwrap();
    env.mount("/ro", ro.clone(), MountFlags { read_only: true, ..MountFlags::default() });
    let ctx = &env.ctx;
    let r = &env.root;
    assert_eq!(env.read("/ro/f").unwrap(), b"read only");
    assert_eq!(env.open("/ro/f", OpenFlags::WRONLY).map(|_| ()), Err(Errno::EROFS));
    assert_eq!(env.open("/ro/f", OpenFlags::RDONLY | OpenFlags::TRUNC).map(|_| ()), Err(Errno::EROFS));
    assert_eq!(env.open("/ro/new", OpenFlags::WRONLY | OpenFlags::CREAT).map(|_| ()), Err(Errno::EROFS));
    assert_eq!(env.vfs.mkdir(ctx, r, At::Cwd, b"/ro/x", 0o755), Err(Errno::EROFS));
    assert_eq!(env.vfs.mkdir(ctx, r, At::Cwd, b"/ro/d", 0o755), Err(Errno::EEXIST));
    assert_eq!(env.vfs.unlink(ctx, r, At::Cwd, b"/ro/f"), Err(Errno::EROFS));
    assert_eq!(env.vfs.access(ctx, r, At::Cwd, b"/ro/f", Access::WRITE, true), Err(Errno::EROFS));
    assert_eq!(env.vfs.chmod(ctx, r, At::Cwd, b"/ro/f", 0o600, true), Err(Errno::EROFS));
    assert_eq!(env.vfs.truncate(ctx, r, At::Cwd, b"/ro/f", 0), Err(Errno::EROFS));
    // A device on a read-only mount can still be written.
    assert_eq!(env.open("/ro/null", OpenFlags::WRONLY).unwrap().write(b"x"), Ok(1));
    // The same file system mounted again writable shares its device number.
    env.mount("/nodev", ro, MountFlags { no_dev: true, no_exec: true, ..MountFlags::default() });
    assert_eq!(env.stat("/nodev/f").unwrap().dev, env.stat("/ro/f").unwrap().dev);
    assert_eq!(env.open("/nodev/null", OpenFlags::WRONLY).map(|_| ()), Err(Errno::EACCES));
    env.vfs.chmod(ctx, r, At::Cwd, b"/nodev/f", 0o755, true).unwrap();
    assert_eq!(env.vfs.open_exec(ctx, r, At::Cwd, b"/nodev/f").map(|_| ()), Err(Errno::EACCES));
    assert!(env.vfs.open_exec(ctx, r, At::Cwd, b"/ro/f").is_ok());
    assert_eq!(
        env.vfs.mount(ctx, &Cred::user(1, 1), At::Cwd, b"/noexec", tmpfs(), MountFlags::default()),
        Err(Errno::EPERM)
    );
    assert_eq!(env.vfs.mount(ctx, r, At::Cwd, b"/ro/f", tmpfs(), MountFlags::default()), Err(Errno::ENOTDIR));
}

/// A tmpfs that lets the VFS cache its lookups and counts the ones that
/// reach it.
struct Counting {
    inner: Tmpfs,
    lookups: AtomicUsize,
}

impl FileSystem for Counting {
    fn info(&self) -> FsInfo {
        FsInfo { name: "counting", read_only: false, cache_lookups: true }
    }
    fn root(&self) -> Ino {
        self.inner.root()
    }
    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        self.inner.lookup(dir, name)
    }
    fn getattr(&self, ino: Ino) -> Result<Attr> {
        self.inner.getattr(ino)
    }
    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        self.inner.setattr(ino, set)
    }
    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino> {
        self.inner.create(dir, name, node)
    }
    fn mkdir(&self, dir: Ino, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<Ino> {
        self.inner.mkdir(dir, name, perm, uid, gid)
    }
    fn symlink(&self, dir: Ino, name: &[u8], target: &[u8], uid: u32, gid: u32) -> Result<Ino> {
        self.inner.symlink(dir, name, target, uid, gid)
    }
    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> Result<()> {
        self.inner.link(dir, name, ino)
    }
    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.inner.unlink(dir, name)
    }
    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.inner.rmdir(dir, name)
    }
    fn rename(&self, od: Ino, on: &[u8], nd: Ino, nn: &[u8], flags: RenameFlags) -> Result<()> {
        self.inner.rename(od, on, nd, nn, flags)
    }
    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        self.inner.readlink(ino)
    }
    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.inner.read(ino, offset, buf)
    }
    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        self.inner.write(ino, offset, buf)
    }
    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        self.inner.readdir(dir, pos, emit)
    }
    fn open(&self, ino: Ino) -> Result<()> {
        self.inner.open(ino)
    }
    fn release(&self, ino: Ino) {
        self.inner.release(ino)
    }
}

#[test]
fn lookup_cache_hits_and_is_invalidated() {
    let fs =
        Arc::new(Counting { inner: Tmpfs::new(Arc::new(super::TestClock::default())), lookups: AtomicUsize::new(0) });
    let vfs = Vfs::new(fs.clone());
    let mut env = Env { fs: tmpfs(), ctx: vfs.root_context().unwrap(), vfs, root: Cred::root() };
    let count = || fs.lookups.load(Ordering::Relaxed);
    env.mkdir("/d");
    env.write("/d/f", b"1");
    // Creating looked "f" up and missed; misses are not cached, the first
    // successful lookup is.
    env.stat("/d/f").unwrap();
    let before = count();
    for _ in 0..10 {
        env.stat("/d/f").unwrap();
    }
    assert_eq!(count(), before, "repeated lookups are served from the cache");
    let sb = env.vfs.mounts.lock().mounts[0].sb.clone();
    assert!(sb.cached() >= 2);

    // unlink + recreate: the new inode is found, not the cached old one.
    let old = env.ino("/d/f").unwrap();
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/f").unwrap();
    assert_eq!(env.stat("/d/f"), Err(Errno::ENOENT));
    env.write("/d/f", b"2");
    assert_ne!(env.ino("/d/f").unwrap(), old);
    assert_eq!(env.read("/d/f").unwrap(), b"2");

    // rename: both names are forgotten.
    env.write("/d/g", b"3");
    env.stat("/d/g").unwrap();
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/d/g", At::Cwd, b"/d/f", RenameFlags::default()).unwrap();
    assert_eq!(env.read("/d/f").unwrap(), b"3");
    assert_eq!(env.stat("/d/g"), Err(Errno::ENOENT));
    // Moving a directory: ".." is never cached, so it follows the move.
    env.mkdir("/e");
    env.mkdir("/d/sub");
    env.cd("/d/sub").unwrap();
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/d/sub", At::Cwd, b"/e/sub", RenameFlags::default()).unwrap();
    assert_eq!(env.ino(".."), env.ino("/e"));
    assert_eq!(env.cwd(), b"/e/sub");
    // rmdir and mkdir again.
    env.cd("/").unwrap();
    let old = env.ino("/e/sub").unwrap();
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/e/sub").unwrap();
    env.mkdir("/e/sub");
    assert_ne!(env.ino("/e/sub").unwrap(), old);
}

#[test]
fn mount_paths_name_mount_points() {
    let env = Env::new();
    env.mkdir("/mnt");
    let first = env.vfs.mounts()[0].id;
    let mnt = env.mount("/mnt", tmpfs(), MountFlags::default());
    env.mkdir("/mnt/deep");
    let deep = env.mount("/mnt/deep", tmpfs(), MountFlags::default());
    // Stacked on the same point: both are named by it.
    let stacked = env.mount("/mnt", other_fs("/f", b"x"), MountFlags::default());
    let path = |id| env.vfs.mount_path(&env.ctx, id);
    assert_eq!(path(first).unwrap(), b"/");
    assert_eq!(path(mnt).unwrap(), b"/mnt");
    assert_eq!(path(stacked).unwrap(), b"/mnt");
    assert_eq!(path(deep).unwrap(), b"/mnt/deep");
    assert_eq!(path(9999), Err(Errno::ENOENT));
}

#[test]
fn remounting_changes_a_mounts_flags() {
    let env = Env::new();
    env.mkdir("/m");
    env.mkdir("/m2");
    let fs = other_fs("/f", b"data");
    let id = env.mount("/m", fs.clone(), MountFlags::default());
    env.mount("/m2", fs, MountFlags::default());
    let (ctx, r) = (&env.ctx, &env.root);
    assert_eq!(env.vfs.mount_at(ctx, r, At::Cwd, b"/m", true), Ok(id));
    assert_eq!(env.vfs.mount_at(ctx, r, At::Cwd, b"/m/f", true), Err(Errno::EINVAL));
    let ro = MountFlags { read_only: true, no_exec: true, ..MountFlags::default() };
    env.vfs.remount(ctx, r, At::Cwd, b"/m", ro).unwrap();
    assert_eq!(env.open("/m/f", OpenFlags::WRONLY).map(|_| ()), Err(Errno::EROFS));
    assert!(env.vfs.mounts().iter().any(|m| m.id == id && m.flags == ro));
    let st = env.vfs.lookup(ctx, r, At::Cwd, b"/m", true).unwrap().statfs().unwrap();
    assert_eq!(st.flags & (ST_RDONLY | ST_NOEXEC), ST_RDONLY | ST_NOEXEC);
    // The other mount of the same file system keeps its own flags.
    assert_eq!(env.open("/m2/f", OpenFlags::WRONLY).map(|_| ()), Ok(()));
    env.vfs.remount(ctx, r, At::Cwd, b"/m", MountFlags::default()).unwrap();
    assert_eq!(env.open("/m/f", OpenFlags::WRONLY).map(|_| ()), Ok(()));
    // Only the root of a mount, and only with CAP_SYS_ADMIN.
    assert_eq!(env.vfs.remount(ctx, r, At::Cwd, b"/m/f", ro), Err(Errno::EINVAL));
    assert_eq!(env.vfs.remount(ctx, &Cred::user(1, 1), At::Cwd, b"/m", ro), Err(Errno::EPERM));
    // The root itself.
    env.vfs.remount(ctx, r, At::Cwd, b"/", ro).unwrap();
    assert_eq!(env.vfs.mkdir(ctx, r, At::Cwd, b"/x", 0o755), Err(Errno::EROFS));
}

#[test]
fn mount_namespaces_are_copies_of_the_tree() {
    let mut env = Env::new();
    env.mkdir("/m");
    env.mkdir("/n");
    env.mount("/m", other_fs("/f", b"shared"), MountFlags::default());
    env.mkdir("/m/d");
    env.cd("/m/d").unwrap();
    let initial = env.vfs.namespace_of(&env.ctx);
    let mut ctx = env.ctx.clone();
    let copy = env.vfs.copy_namespace(&mut ctx).unwrap();
    assert_ne!(copy, initial);
    assert_eq!(env.vfs.namespace_of(&ctx), copy);
    assert_eq!(env.vfs.namespace_mounts(initial).len(), 2);
    assert_eq!(env.vfs.namespace_mounts(copy).len(), 2);
    // The working directory moved into the copy, on the same file.
    assert_eq!(env.vfs.path_of(&ctx, ctx.cwd()).unwrap(), b"/m/d");
    let r = &env.root;
    let f = env.vfs.open(&ctx, r, At::Cwd, b"../f", OpenFlags::RDONLY, 0).unwrap();
    assert_eq!(super::read_all(&f).unwrap(), b"shared");
    drop(f);
    // A mount in the copy is not seen from the initial namespace.
    let id = env.vfs.mount(&ctx, r, At::Cwd, b"/n", other_fs("/g", b"copy only"), MountFlags::default()).unwrap();
    assert_eq!(env.read("/n/g"), Err(Errno::ENOENT));
    assert!(env.vfs.namespace_mounts(copy).iter().any(|m| m.id == id));
    assert!(!env.vfs.namespace_mounts(initial).iter().any(|m| m.id == id));
    assert!(env.vfs.mount_path(&ctx, id).is_ok());
    assert_eq!(env.vfs.mount_path(&env.ctx, id), Err(Errno::ENOENT));
    // Nor is an unmount: /m stays mounted in the initial namespace.
    let top = env.vfs.lookup(&ctx, r, At::Cwd, b"/", true).unwrap();
    ctx.set_cwd(r, top).unwrap();
    env.vfs.umount(&ctx, r, At::Cwd, b"/n", false).unwrap();
    env.vfs.umount(&ctx, r, At::Cwd, b"/m", false).unwrap();
    assert_eq!(env.read("/m/f").unwrap(), b"shared");
    // Entering a namespace puts root and working directory at its root.
    let mut other = env.vfs.root_context().unwrap();
    env.vfs.enter_namespace(&mut other, copy).unwrap();
    assert_eq!(env.vfs.namespace_of(&other), copy);
    assert_eq!(env.vfs.enter_namespace(&mut other, id), Err(Errno::EINVAL), "not a namespace's root");
    // A root moved by chroot is not the namespace's root; "/" is, also
    // when a file system is mounted over it.
    assert!(!env.vfs.is_chrooted(&env.ctx) && !env.vfs.is_chrooted(&other));
    let mut jail = env.vfs.root_context().unwrap();
    jail.set_root(r, env.vfs.lookup(&jail, r, At::Cwd, b"/m", true).unwrap()).unwrap();
    assert!(env.vfs.is_chrooted(&jail));
    jail.set_root(r, env.vfs.lookup(&env.ctx, r, At::Cwd, b"/", true).unwrap()).unwrap();
    assert!(!env.vfs.is_chrooted(&jail));
    drop(jail);
    // The initial namespace cannot be dropped; a copy can.
    assert_eq!(env.vfs.drop_namespace(initial), Err(Errno::EBUSY));
    drop((ctx, other));
    env.vfs.drop_namespace(copy).unwrap();
    assert!(env.vfs.namespace_mounts(copy).is_empty());
    assert_eq!(env.vfs.mounts().len(), 2);
}
