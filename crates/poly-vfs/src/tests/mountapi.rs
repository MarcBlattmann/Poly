//! The mount API on the mount table (ADR 0122): detached mounts and their
//! lifetime, attaching and moving mounts, pivot_root, writers, and the
//! namespace a pivot renames.

use super::{Env, tmpfs};
use crate::*;
use alloc::sync::Arc;

impl Env {
    fn found(&self, path: &str) -> NodeRef {
        self.vfs.lookup(&self.ctx, &self.root, At::Cwd, path.as_bytes(), true).unwrap()
    }

    fn new_tmpfs(&self, path: &str) -> MountId {
        self.vfs.mount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), tmpfs(), MountFlags::default()).unwrap()
    }
}

#[test]
fn a_detached_mount_lives_with_its_files() {
    let env = Env::new();
    env.mkdir("/a");
    let fs = tmpfs();
    let m = env.vfs.new_detached(fs.clone(), MountFlags::default());
    assert!(!env.vfs.is_attached(&m));
    let root = env.vfs.mount_root_node(&m).unwrap();
    let file = env.vfs.open_path(root.clone());
    assert!(file.is_path_only());
    assert_eq!(file.flags(), OpenFlags::PATH);
    // Files can be made through it, and are not seen at /a.
    let flags = OpenFlags::CREAT | OpenFlags::RDWR;
    env.vfs.open(&env.ctx, &env.root, At::Node(&root), b"x", flags, 0o644).unwrap();
    assert_eq!(env.stat("/a/x"), Err(Errno::ENOENT));
    drop(root);
    let weak = Arc::downgrade(&m);
    drop(m);
    assert!(weak.upgrade().is_some(), "the open file holds it");
    drop(file);
    assert!(weak.upgrade().is_none(), "gone with its last file");
}

#[test]
fn attach_then_move_with_submounts() {
    let env = Env::new();
    env.mkdir("/a");
    env.mkdir("/b");
    let m = env.vfs.new_detached(tmpfs(), MountFlags::default());
    let root = env.vfs.mount_root_node(&m).unwrap();
    env.vfs.open(&env.ctx, &env.root, At::Node(&root), b"marker", OpenFlags::CREAT | OpenFlags::RDWR, 0o644).unwrap();
    env.vfs.move_mount(&env.root, &root, &env.found("/a")).unwrap();
    assert!(env.vfs.is_attached(&m));
    assert!(env.stat("/a/marker").is_ok());
    env.mkdir("/a/sub");
    env.new_tmpfs("/a/sub");
    env.write("/a/sub/deep", b"d");
    // Moved by path, with what is mounted below it.
    env.vfs.move_mount(&env.root, &env.found("/a"), &env.found("/b")).unwrap();
    assert!(env.stat("/a/marker").is_err());
    assert_eq!(env.read("/b/sub/deep").unwrap(), b"d");
    assert_ne!(env.stat("/b/sub").unwrap().dev, env.stat("/b").unwrap().dev);
    // Not below itself; not a plain directory; not the root.
    assert_eq!(env.vfs.move_mount(&env.root, &env.found("/b"), &env.found("/b/sub")), Err(Errno::ELOOP));
    env.mkdir("/plain");
    assert_eq!(env.vfs.move_mount(&env.root, &env.found("/plain"), &env.found("/a")), Err(Errno::EINVAL));
    assert_eq!(env.vfs.move_mount(&env.root, &env.found("/"), &env.found("/a")), Err(Errno::ELOOP));
    // By its descriptor's node, back to /a.
    env.vfs.move_mount(&env.root, &root, &env.found("/a")).unwrap();
    assert_eq!(env.read("/a/sub/deep").unwrap(), b"d");
    let user = Cred::user(1000, 1000);
    assert_eq!(env.vfs.move_mount(&user, &root, &env.found("/b")), Err(Errno::EPERM));
}

#[test]
fn a_clone_shares_the_file_system() {
    let env = Env::new();
    env.mkdir("/a");
    env.new_tmpfs("/a");
    env.mkdir("/a/dir");
    let c = env.vfs.clone_mount(&env.root, &env.found("/a/dir"), false).unwrap();
    let root = env.vfs.mount_root_node(&c).unwrap();
    env.vfs.open(&env.ctx, &env.root, At::Node(&root), b"f", OpenFlags::CREAT | OpenFlags::RDWR, 0o644).unwrap();
    assert!(env.stat("/a/dir/f").is_ok());
    assert_eq!(env.vfs.mount_root_path(&c).unwrap(), b"/dir");
    assert!(!env.vfs.is_attached(&c));
}

#[test]
fn writers_are_counted_per_mount_and_superblock() {
    let env = Env::new();
    env.mkdir("/a");
    env.new_tmpfs("/a");
    let m = env.found("/a").mount().clone();
    assert_eq!(m.writers(), 0);
    let w = env.open("/a/w", OpenFlags::CREAT | OpenFlags::WRONLY).unwrap();
    let r = env.open("/a/w", OpenFlags::RDONLY).unwrap();
    assert_eq!((m.writers(), m.sb_writers()), (1, 1));
    drop(w);
    drop(r);
    assert_eq!((m.writers(), m.sb_writers()), (0, 0));
    m.set_sb_read_only(true);
    assert!(m.read_only());
    assert_eq!(env.open("/a/x", OpenFlags::CREAT | OpenFlags::WRONLY).err(), Some(Errno::EROFS));
    m.set_sb_read_only(false);
    assert!(env.open("/a/x", OpenFlags::CREAT | OpenFlags::WRONLY).is_ok());
}

impl Env {
    /// mount(2) with `MS_REMOUNT` of the mount at `path`, read-only or
    /// not, of the whole file system or (`bind`) of that mount only.
    fn remount(&self, path: &str, read_only: bool, bind: bool) -> Result<()> {
        let flags = MountFlags { read_only, ..MountFlags::default() };
        self.vfs.remount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), flags, None, bind)
    }
}

/// mount's spec sheet, `busy_writers`: which open files keep a file
/// system from being remounted read-only as a whole.
#[test]
fn remounting_read_only_is_busy_while_files_are_open_for_writing() {
    let env = Env::new();
    env.mkdir("/a");
    env.new_tmpfs("/a");
    env.write("/a/f", b"data");
    for flags in [OpenFlags::RDWR, OpenFlags::WRONLY, OpenFlags::WRONLY | OpenFlags::APPEND] {
        let w = env.open("/a/f", flags).unwrap();
        assert_eq!(env.remount("/a", true, false), Err(Errno::EBUSY), "{flags:?}");
        drop(w);
    }
    // A dup shares the open file: closing one of the two is not enough.
    // (So does a mapping made from it, which holds it until munmap.)
    let w = env.open("/a/f", OpenFlags::WRONLY).unwrap();
    let dup = w.clone();
    drop(w);
    assert_eq!(env.remount("/a", true, false), Err(Errno::EBUSY));
    // The refused remount changed nothing: still writable, flags kept.
    let m = env.found("/a").mount().clone();
    assert!(!m.read_only());
    assert_eq!(dup.write(b"more"), Ok(4));
    let nosuid = MountFlags { read_only: true, no_suid: true, ..MountFlags::default() };
    assert_eq!(env.vfs.remount(&env.ctx, &env.root, At::Cwd, b"/a", nosuid, None, false), Err(Errno::EBUSY));
    assert_eq!(m.flags(), MountFlags::default());
    assert!(env.open("/a/new", OpenFlags::CREAT | OpenFlags::WRONLY).is_ok());
    drop(dup);
    // Once the last writer is gone the same remount succeeds.
    assert_eq!(env.remount("/a", true, false), Ok(()));
    assert!(m.read_only());
    // Already read-only, or going writable: never busy.
    assert_eq!(env.remount("/a", true, false), Ok(()));
    assert_eq!(env.remount("/a", false, false), Ok(()));
    let w = env.open("/a/f", OpenFlags::WRONLY).unwrap();
    assert_eq!(env.remount("/a", false, false), Ok(()));
    drop(w);
}

#[test]
fn readers_paths_directories_and_other_file_systems_do_not_keep_it_busy() {
    let env = Env::new();
    env.mkdir("/a");
    env.mkdir("/b");
    env.new_tmpfs("/a");
    env.new_tmpfs("/b");
    env.write("/a/f", b"data");
    env.mkdir("/a/d");
    let r = env.open("/a/f", OpenFlags::RDONLY).unwrap();
    let p = env.open("/a/f", OpenFlags::PATH | OpenFlags::RDWR).unwrap();
    let d = env.open("/a/d", OpenFlags::RDONLY | OpenFlags::DIRECTORY).unwrap();
    let elsewhere = env.open("/b/w", OpenFlags::CREAT | OpenFlags::WRONLY).unwrap();
    assert_eq!(env.remount("/a", true, false), Ok(()));
    assert_eq!(env.remount("/a", false, false), Ok(()));
    drop((r, p, d, elsewhere));
}

#[test]
fn an_unlinked_file_still_open_keeps_it_busy() {
    let env = Env::new();
    env.mkdir("/a");
    env.new_tmpfs("/a");
    env.write("/a/u", b"u");
    env.write("/a/kept", b"k");
    let r = env.open("/a/u", OpenFlags::RDONLY).unwrap();
    let kept = env.open("/a/kept", OpenFlags::RDONLY).unwrap();
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/a/u").unwrap();
    assert_eq!(env.remount("/a", true, false), Err(Errno::EBUSY));
    // Not for a remount of one mount only.
    assert_eq!(env.remount("/a", true, true), Ok(()));
    assert_eq!(env.remount("/a", false, true), Ok(()));
    drop(r);
    assert_eq!(env.remount("/a", true, false), Ok(()));
    drop(kept);
}

#[test]
fn writers_through_any_mount_count_for_the_file_system_only_its_own_for_a_mount() {
    let env = Env::new();
    env.mkdir("/a");
    env.mkdir("/b");
    env.new_tmpfs("/a");
    env.write("/a/f", b"data");
    env.vfs.bind(&env.ctx, &env.root, At::Cwd, b"/a", At::Cwd, b"/b", false).unwrap();
    // Through the bind mount: busy for the whole file system and for /b.
    let w = env.open("/b/f", OpenFlags::WRONLY).unwrap();
    assert_eq!(env.remount("/a", true, false), Err(Errno::EBUSY));
    assert_eq!(env.remount("/b", true, true), Err(Errno::EBUSY));
    assert!(!env.found("/b").mount().read_only());
    drop(w);
    // Through the other mount: /b alone may go read-only.
    let w = env.open("/a/f", OpenFlags::WRONLY).unwrap();
    assert_eq!(env.remount("/b", true, true), Ok(()));
    assert_eq!(env.open("/b/x", OpenFlags::CREAT | OpenFlags::WRONLY).err(), Some(Errno::EROFS));
    assert!(env.open("/a/x", OpenFlags::CREAT | OpenFlags::WRONLY).is_ok());
    assert_eq!(env.remount("/b", false, true), Ok(()));
    drop(w);
}

#[test]
fn pivot_root_in_a_copied_namespace() {
    let mut env = Env::new();
    env.mkdir("/new");
    env.write("/top", b"t");
    let name = env.vfs.copy_namespace(&mut env.ctx, false).unwrap();
    env.new_tmpfs("/new");
    env.mkdir("/new/old");
    env.write("/new/inside", b"i");
    env.cd("/new").unwrap();
    // The root's own mount, a plain directory, put_old outside new_root.
    let (new, old) = (env.found("/new"), env.found("/new/old"));
    assert_eq!(env.vfs.pivot_root(&mut env.ctx.clone(), &env.root, &env.found("/"), &old).err(), Some(Errno::EBUSY));
    assert_eq!(env.vfs.pivot_root(&mut env.ctx.clone(), &env.root, &new, &env.found("/")).err(), Some(Errno::EBUSY));
    assert_eq!(env.vfs.pivot_root(&mut env.ctx.clone(), &env.root, &old, &old).err(), Some(Errno::EINVAL));
    // Another process of the namespace, at the old root.
    let mut other = env.ctx.clone();
    other.cwd = other.root.clone();
    let mut ctx = env.ctx.clone();
    let (was, now) = env.vfs.pivot_root(&mut ctx, &env.root, &new, &old).unwrap();
    env.ctx = ctx;
    assert_eq!(env.cwd(), b"/");
    Vfs::repoint_context(&mut other, &was, &now);
    assert!(other.root.same(&now) && other.cwd.same(&now));

    assert_eq!(env.read("/inside").unwrap(), b"i");
    assert_eq!(env.read("/old/top").unwrap(), b"t");
    env.vfs.umount(&env.ctx, &env.root, At::Cwd, b"/old", true).unwrap();
    assert_eq!(env.stat("/old/top"), Err(Errno::ENOENT));
    // The namespace keeps its name.
    assert_eq!(env.vfs.namespace_mounts(name).len(), 1);
    env.vfs.drop_namespace(name).unwrap();
}

#[test]
fn pivot_root_dot_dot_stacks_the_old_root() {
    let mut env = Env::new();
    env.mkdir("/new");
    env.vfs.copy_namespace(&mut env.ctx, false).unwrap();
    env.new_tmpfs("/new");
    env.cd("/new").unwrap();
    let new = env.found("/new");
    let mut ctx = env.ctx.clone();
    env.write("/new/g", b"y");
    env.mkdir("/tmpdir");
    env.vfs.pivot_root(&mut ctx, &env.root, &new, &new).unwrap();
    env.ctx = ctx;
    // The spec sheet of pivot_root_flow, `stacked`: the new tree is "/" at
    // once and the old root is hidden beneath it.
    assert_eq!(env.cwd(), b"/");
    assert_eq!(env.read("/g").unwrap(), b"y");
    assert_eq!(env.read("g").unwrap(), b"y");
    assert_eq!(env.stat("/new"), Err(Errno::ENOENT), "the old root is hidden");
    assert_eq!(env.stat("/tmpdir"), Err(Errno::ENOENT));
    assert_eq!(env.ino(".."), env.ino("/"));
    // A second pivot_root(".", "."): the new root is the root's mount.
    let here = env.found(".");
    assert_eq!(env.vfs.pivot_root(&mut env.ctx.clone(), &env.root, &here, &here).err(), Some(Errno::EBUSY));
    // The old root is what an unmount of "." (or "/") removes.
    let before = env.vfs.namespace_tree(&env.ctx).len();
    assert_eq!(before, 2);
    assert_eq!(env.vfs.unmount_id(&env.ctx, &env.root, At::Cwd, b".", true).ok(), Some(old_root_id(&env)));
    env.vfs.umount(&env.ctx, &env.root, At::Cwd, b".", true).unwrap();
    assert_eq!(env.vfs.namespace_tree(&env.ctx).len(), 1);
    assert_eq!(env.cwd(), b"/");
    assert_eq!(env.read("/g").unwrap(), b"y");
    // Nothing is left to unmount there: the root itself cannot go.
    assert_eq!(env.vfs.umount(&env.ctx, &env.root, At::Cwd, b".", true), Err(Errno::EBUSY));
}

/// The mount stacked beneath the root of `env`'s namespace.
fn old_root_id(env: &Env) -> MountId {
    env.vfs.namespace_tree(&env.ctx).iter().find(|m| m.is_beneath()).map(|m| m.id()).expect("an old root")
}

/// The spec sheet of mntapi_move_mount: kinds, namespaces, loops.
#[test]
fn move_mount_refuses_mismatched_kinds_other_namespaces_and_loops() {
    let mut env = Env::new();
    for d in ["/a", "/b", "/c"] {
        env.mkdir(d);
    }
    env.write("/file", b"f");
    env.write("/file2", b"other");
    env.new_tmpfs("/a");
    let mv = |env: &Env, src: &NodeRef, dst: &str| env.vfs.move_mount(&env.root, src, &env.found(dst));
    // A directory mount needs a directory, the clone of a file a file.
    let dir = env.vfs.mount_root_node(&env.vfs.new_detached(tmpfs(), MountFlags::default())).unwrap();
    assert_eq!(mv(&env, &dir, "/file"), Err(Errno::EINVAL));
    let clone = env.vfs.clone_mount(&env.root, &env.found("/file"), false).unwrap();
    let file = env.vfs.mount_root_node(&clone).unwrap();
    assert_eq!(mv(&env, &file, "/c"), Err(Errno::EINVAL));
    assert_eq!(mv(&env, &file, "/file2"), Ok(()));
    assert_eq!(env.read("/file2").unwrap(), b"f");
    assert_eq!(mv(&env, &env.found("/a"), "/file"), Err(Errno::EINVAL));
    // Onto itself, below itself, and the first mount of the namespace.
    env.mkdir("/a/inner");
    assert_eq!(mv(&env, &env.found("/a"), "/a"), Err(Errno::ELOOP));
    assert_eq!(mv(&env, &env.found("/a"), "/a/inner"), Err(Errno::ELOOP));
    assert_eq!(mv(&env, &env.found("/"), "/b"), Err(Errno::ELOOP));
    // A mount attached in another namespace cannot be moved; a detached
    // one is attached once, in whichever namespace.
    let attached = env.found("/a");
    let here = env.ctx.clone();
    let name = env.vfs.copy_namespace(&mut env.ctx, false).unwrap();
    assert_eq!(mv(&env, &attached, "/c"), Err(Errno::EINVAL));
    assert_eq!(mv(&env, &dir, "/c"), Ok(()));
    assert!(is_mount(&env, "/c"));
    // Back in the first namespace, where the copy's mounts are not.
    env.ctx = here;
    assert!(!is_mount(&env, "/c"));
    assert_eq!(mv(&env, &dir, "/b"), Err(Errno::EINVAL), "attached elsewhere already");
    // Also after the other namespace ended.
    env.vfs.drop_namespace(name).unwrap();
    assert_eq!(mv(&env, &dir, "/b"), Err(Errno::EINVAL));
    assert_eq!(mv(&env, &attached, "/b"), Ok(()));
}

#[test]
fn copies_keep_attributes() {
    let mut env = Env::new();
    env.mkdir("/a");
    env.new_tmpfs("/a");
    let m = env.found("/a").mount().clone();
    m.set_attrs(0x80);
    env.vfs.set_propagation(core::slice::from_ref(&m), 1 << 20);
    assert_eq!(env.found("/a").statfs().unwrap().flags & ST_NODIRATIME_BIT, ST_NODIRATIME_BIT);
    env.vfs.copy_namespace(&mut env.ctx, false).unwrap();
    let copy = env.found("/a").mount().clone();
    assert_ne!(copy.id(), m.id());
    assert_eq!((copy.attrs(), copy.propagation()), (0x80, 1 << 20));
}

/// `ST_NODIRATIME` (`sys/statvfs.h`).
const ST_NODIRATIME_BIT: u64 = 2048;

#[test]
fn a_recursive_clone_keeps_its_submounts_until_attached_or_closed() {
    let env = Env::new();
    env.mkdir("/a");
    env.new_tmpfs("/a");
    env.mkdir("/a/s");
    env.new_tmpfs("/a/s");
    env.mkdir("/a/s/t");
    env.new_tmpfs("/a/s/t");
    env.write("/a/s/t/f", b"deep");
    env.mkdir("/a/other");
    env.new_tmpfs("/a/other");
    // Plain clone: the submount is not there.
    let flat = env.vfs.clone_mount(&env.root, &env.found("/a"), false).unwrap();
    let flat_root = env.vfs.mount_root_node(&flat).unwrap();
    assert_eq!(env.vfs.lookup(&env.ctx, &env.root, At::Node(&flat_root), b"s/t/f", true).err(), Some(Errno::ENOENT));
    // Recursive: both levels, found through the detached top.
    let c = env.vfs.clone_mount(&env.root, &env.found("/a"), true).unwrap();
    let root = env.vfs.mount_root_node(&c).unwrap();
    let f = env.vfs.lookup(&env.ctx, &env.root, At::Node(&root), b"s/t/f", true).unwrap();
    assert!(!Arc::ptr_eq(f.mount(), env.found("/a/s/t/f").mount()), "a copy of the mount, not the mount");
    assert_eq!(env.vfs.mount_subtree(&c, true).len(), 4, "top, s, t, other");
    // A clone of a directory takes only what is mounted below it.
    let part = env.vfs.clone_mount(&env.root, &env.found("/a/s"), true).unwrap();
    assert_eq!(env.vfs.mount_subtree(&part, true).len(), 2);
    // Attached: the table keeps them all.
    env.mkdir("/b");
    env.vfs.move_mount(&env.root, &root, &env.found("/b")).unwrap();
    drop((f, root));
    drop(c);
    assert_eq!(env.read("/b/s/t/f").unwrap(), b"deep");
    // Closed without attaching: the copies go with the top.
    let weak = {
        let d = env.vfs.clone_mount(&env.root, &env.found("/a"), true).unwrap();
        let below = d.subtree();
        assert_eq!(below.len(), 3);
        Arc::downgrade(&below[0])
    };
    assert!(weak.upgrade().is_none());
}

#[test]
fn nosymfollow_and_the_access_time_mode_in_statfs() {
    let env = Env::new();
    env.mkdir("/a");
    env.new_tmpfs("/a");
    env.write("/a/f", b"x");
    env.symlink("f", "/a/l");
    let m = env.found("/a").mount().clone();
    assert_eq!(env.found("/a").statfs().unwrap().flags & crate::ST_RELATIME, crate::ST_RELATIME);
    m.set_attrs(0x10);
    let flags = env.found("/a").statfs().unwrap().flags;
    assert_eq!(flags & (crate::ST_RELATIME | 1024), 1024, "noatime: ST_NOATIME, no ST_RELATIME");
    m.set_attrs(0x20);
    assert_eq!(env.found("/a").statfs().unwrap().flags & (crate::ST_RELATIME | 1024), 0, "strictatime: neither");
    m.set_attrs(crate::mount::ATTR_NOSYMFOLLOW);
    assert_eq!(env.read("/a/l").err(), Some(Errno::ELOOP));
    assert_eq!(env.read("/a/f").unwrap(), b"x");
    assert!(env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/a/l", false).is_ok(), "the link itself");
    m.set_attrs(0);
    assert_eq!(env.read("/a/l").unwrap(), b"x");
}

fn is_mount(env: &Env, path: &str) -> bool {
    let n = env.found(path);
    n.loc().is_mount_root()
}

#[test]
fn peers_and_slaves_receive_mounts_and_unmounts() {
    use crate::propagation::{MS_PRIVATE, MS_SHARED, MS_SLAVE, MS_UNBINDABLE};
    let env = Env::new();
    env.mkdir("/a");
    env.mkdir("/b");
    env.new_tmpfs("/a");
    let a = env.found("/a").mount().clone();
    env.vfs.set_propagation(core::slice::from_ref(&a), MS_SHARED);
    env.vfs.bind(&env.ctx, &env.root, At::Cwd, b"/a", At::Cwd, b"/b", false).unwrap();
    let b = env.found("/b").mount().clone();
    assert_eq!(a.peer_group(), b.peer_group());
    assert_ne!(a.peer_group(), 0);
    // A mount below one appears below the other.
    env.mkdir("/a/s");
    env.new_tmpfs("/a/s");
    assert!(is_mount(&env, "/b/s"));
    // A mount below a shared mount is shared as well.
    assert_ne!(env.found("/a/s").mount().peer_group(), 0);
    // The umount follows.
    env.vfs.umount(&env.ctx, &env.root, At::Cwd, b"/a/s", false).unwrap();
    assert!(!is_mount(&env, "/b/s"));
    // A slave receives but does not send.
    env.vfs.set_propagation(core::slice::from_ref(&b), MS_SLAVE);
    assert_eq!((b.peer_group(), b.master_group()), (0, a.peer_group()));
    env.new_tmpfs("/a/s");
    assert!(is_mount(&env, "/b/s"));
    env.mkdir("/b/t");
    env.new_tmpfs("/b/t");
    assert!(!is_mount(&env, "/a/t"));
    // Private: nothing new arrives; unbindable: no bind.
    env.vfs.set_propagation(core::slice::from_ref(&b), MS_PRIVATE);
    env.mkdir("/a/u");
    env.new_tmpfs("/a/u");
    assert!(!is_mount(&env, "/b/u"));
    env.vfs.set_propagation(core::slice::from_ref(&a), MS_UNBINDABLE);
    assert_eq!(env.vfs.bind(&env.ctx, &env.root, At::Cwd, b"/a", At::Cwd, b"/b", false), Err(Errno::EINVAL));
}

// ---- recursive binds and mount locks (ADR 0177) ---------------------------

impl Env {
    fn bind(&self, source: &str, target: &str, recursive: bool) -> Result<MountId> {
        self.vfs.bind(&self.ctx, &self.root, At::Cwd, source.as_bytes(), At::Cwd, target.as_bytes(), recursive)
    }

    fn unmount(&self, path: &str, lazy: bool) -> Result<()> {
        self.vfs.umount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), lazy)
    }

    fn remount_flags(&self, path: &str, flags: MountFlags, atime: Option<u32>) -> Result<()> {
        self.vfs.remount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), flags, atime, true)
    }

    fn mount_flags(&self, path: &str, flags: MountFlags) -> MountId {
        self.vfs.mount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), tmpfs(), flags).unwrap()
    }

    /// A source tree: `/s` with a mount on `/s/a` (nodev) and one inside
    /// it on `/s/a/deep`, and the directories to bind onto.
    fn tree(&self) {
        for d in ["/s", "/s/a", "/s/plain", "/r", "/p", "/ra", "/pa", "/u"] {
            self.mkdir(d);
        }
        self.write("/s/top", b"T");
        self.mount_flags("/s/a", MountFlags { no_dev: true, ..MountFlags::default() });
        self.mkdir("/s/a/deep");
        self.write("/s/a/fa", b"in-a");
        self.new_tmpfs("/s/a/deep");
        self.write("/s/a/deep/fd", b"in-deep");
    }
}

/// The spec sheet of mount_bind_propagation, "Recursive binds in detail".
#[test]
fn a_recursive_bind_copies_every_mount_below_as_a_mount_of_its_own() {
    let env = Env::new();
    env.tree();
    env.bind("/s", "/r", true).unwrap();
    assert_eq!(env.read("/r/top").unwrap(), b"T");
    assert_eq!(env.read("/r/a/fa").unwrap(), b"in-a");
    assert_eq!(env.read("/r/a/deep/fd").unwrap(), b"in-deep");
    assert!(is_mount(&env, "/r") && is_mount(&env, "/r/a") && is_mount(&env, "/r/a/deep"));
    // The same file systems: a write through the copy reaches the original.
    env.write("/r/a/deep/w", b"W");
    assert_eq!(env.read("/s/a/deep/w").unwrap(), b"W");
    // A copy keeps the flags of its original; a remount of the top changes the top only.
    let flags = |p: &str| env.found(p).mount().flags();
    assert!(flags("/r/a").no_dev && !flags("/r/a/deep").no_dev && !flags("/r").no_dev);
    env.remount_flags("/r", MountFlags { no_suid: true, ..MountFlags::default() }, None).unwrap();
    assert!(flags("/r").no_suid && !flags("/r/a").no_suid && flags("/r/a").no_dev && !flags("/s").no_suid);

    // A plain bind: only the top; the directory of the submount is the
    // empty directory it covers.
    env.bind("/s", "/p", false).unwrap();
    assert_eq!(env.read("/p/top").unwrap(), b"T");
    assert!(!is_mount(&env, "/p/a"));
    assert_eq!(env.stat("/p/a/fa"), Err(Errno::ENOENT));
    assert_eq!(env.stat("/p/a/deep"), Err(Errno::ENOENT));
    env.unmount("/p", false).unwrap();

    // Starting at a submount.
    env.bind("/s/a", "/ra", true).unwrap();
    assert_eq!(env.read("/ra/deep/fd").unwrap(), b"in-deep");
    env.bind("/s/a", "/pa", false).unwrap();
    assert_eq!(env.read("/pa/fa").unwrap(), b"in-a");
    assert_eq!(env.stat("/pa/deep/fd"), Err(Errno::ENOENT));
    env.unmount("/pa", false).unwrap();
    assert_eq!(env.unmount("/ra", false), Err(Errno::EBUSY));
    env.unmount("/ra", true).unwrap();
    assert!(!is_mount(&env, "/ra"));

    // From the bottom up; the original stays.
    assert_eq!(env.unmount("/r", false), Err(Errno::EBUSY));
    assert_eq!(env.unmount("/r/a", false), Err(Errno::EBUSY));
    env.unmount("/r/a/deep", false).unwrap();
    assert_eq!(env.stat("/r/a/deep/fd"), Err(Errno::ENOENT));
    assert_eq!(env.read("/s/a/deep/fd").unwrap(), b"in-deep");
    env.unmount("/r/a", false).unwrap();
    env.unmount("/r", false).unwrap();
    assert_eq!(env.unmount("/r", false), Err(Errno::EINVAL));
    assert_eq!(env.read("/s/a/fa").unwrap(), b"in-a");

    // An unbindable mount is left out, with what is below it; the
    // bindable ones above it are copied.
    let deep = env.found("/s/a/deep").mount().clone();
    env.vfs.set_propagation(core::slice::from_ref(&deep), crate::propagation::MS_UNBINDABLE);
    drop(deep);
    env.bind("/s", "/u", true).unwrap();
    assert_eq!(env.read("/u/a/fa").unwrap(), b"in-a");
    assert!(is_mount(&env, "/u/a") && !is_mount(&env, "/u/a/deep"));
    assert_eq!(env.stat("/u/a/deep/fd"), Err(Errno::ENOENT));
    env.unmount("/u", true).unwrap();
    env.unmount("/s/a/deep", false).unwrap();
    env.unmount("/s/a", false).unwrap();
}

#[test]
fn a_recursive_bind_into_its_own_source_copies_the_tree_once() {
    let env = Env::new();
    env.tree();
    env.mkdir("/s/a/in");
    let before = env.vfs.mounts().len();
    // The target is inside a mount that is copied.
    env.bind("/s", "/s/a/in", true).unwrap();
    assert_eq!(env.vfs.mounts().len(), before + 3, "the top and one copy of each of the two mounts below");
    assert_eq!(env.read("/s/a/in/a/deep/fd").unwrap(), b"in-deep");
    assert_eq!(env.stat("/s/a/in/a/in/top"), Err(Errno::ENOENT), "the copy shows the directory, not itself");
    env.unmount("/s/a/in", true).unwrap();
    assert_eq!(env.vfs.mounts().len(), before);
}

#[test]
fn a_recursive_bind_onto_a_shared_mount_reaches_its_peers_whole() {
    let env = Env::new();
    env.tree();
    env.mkdir("/t");
    env.mkdir("/peer");
    env.new_tmpfs("/t");
    env.mkdir("/t/x");
    let t = env.found("/t").mount().clone();
    env.vfs.set_propagation(core::slice::from_ref(&t), crate::propagation::MS_SHARED);
    env.bind("/t", "/peer", false).unwrap();
    env.bind("/s", "/t/x", true).unwrap();
    assert_eq!(env.read("/peer/x/top").unwrap(), b"T");
    assert_eq!(env.read("/peer/x/a/deep/fd").unwrap(), b"in-deep");
    // Detached, the tree goes from the peer as well, whole.
    let before = env.vfs.mounts().len();
    env.unmount("/t/x", true).unwrap();
    assert_eq!(env.vfs.mounts().len(), before - 6);
    assert_eq!(env.stat("/peer/x/top"), Err(Errno::ENOENT));
    assert_eq!(env.read("/s/a/deep/fd").unwrap(), b"in-deep");
}

/// The spec sheet of userns_mount, "The locks, rule by rule".
#[test]
fn a_namespace_copied_for_another_user_namespace_is_locked() {
    use crate::mount::lock;
    let mut env = Env::new();
    for d in ["/x", "/y", "/c1", "/c2", "/c3"] {
        env.mkdir(d);
    }
    let noexec = MountFlags { no_exec: true, ..MountFlags::default() };
    let nodev = MountFlags { no_dev: true, ..MountFlags::default() };
    let none = MountFlags::default();
    env.mount_flags("/x", noexec);
    env.found("/x").mount().set_attrs(0x10); // noatime
    env.new_tmpfs("/y");
    env.mkdir("/y/in");
    env.mount_flags("/y/in", nodev);
    env.write("/y/in/f", b"f");
    env.vfs.copy_namespace(&mut env.ctx, true).unwrap();
    let locks = |env: &Env, p: &str| env.found(p).mount().locks();
    assert_eq!(locks(&env, "/"), lock::ATIME, "the first mount is on no mount point");
    assert_eq!(locks(&env, "/x"), lock::PLACE | lock::NO_EXEC | lock::ATIME);
    assert_eq!(locks(&env, "/y/in"), lock::PLACE | lock::NO_DEV | lock::ATIME);

    // (1) In place.
    for lazy in [false, true] {
        assert_eq!(env.unmount("/x", lazy), Err(Errno::EINVAL));
        assert_eq!(env.unmount("/y/in", lazy), Err(Errno::EINVAL));
        assert_eq!(env.unmount("/y", lazy), Err(Errno::EINVAL));
    }
    assert_eq!(env.vfs.umount_expire(&env.ctx, &env.root, At::Cwd, b"/x"), Err(Errno::EINVAL));
    assert_eq!(env.vfs.move_mount(&env.root, &env.found("/x"), &env.found("/c1")), Err(Errno::EINVAL));
    assert_eq!(env.vfs.move_mount(&env.root, &env.found("/y/in"), &env.found("/c1")), Err(Errno::EINVAL));

    // (2) Flags the mount had are locked on; others come and go.
    assert_eq!(env.remount_flags("/x", noexec, None), Ok(()));
    assert_eq!(env.remount_flags("/x", noexec, Some(0x10)), Ok(()));
    assert_eq!(env.remount_flags("/x", none, Some(0x10)), Err(Errno::EPERM));
    assert_eq!(env.remount_flags("/x", MountFlags { no_suid: true, ..noexec }, None), Ok(()));
    assert_eq!(env.remount_flags("/x", noexec, None), Ok(()));
    assert_eq!(env.remount_flags("/y/in", none, None), Err(Errno::EPERM));
    assert_eq!(env.remount_flags("/y/in", MountFlags { read_only: true, ..nodev }, None), Ok(()));
    assert_eq!(env.open("/y/in/g", OpenFlags::CREAT | OpenFlags::WRONLY).err(), Some(Errno::EROFS));
    assert_eq!(env.remount_flags("/y/in", nodev, None), Ok(()));
    // (3) The access time mode, whatever it was.
    assert_eq!(env.remount_flags("/x", noexec, Some(0)), Err(Errno::EPERM));
    assert_eq!(env.remount_flags("/y", none, Some(0x10)), Err(Errno::EPERM));
    assert_eq!(env.remount_flags("/y", none, Some(0x20)), Err(Errno::EPERM));
    assert_eq!(env.remount_flags("/y", none, Some(0)), Ok(()));
    assert_eq!(env.remount_flags("/y", none, None), Ok(()));
    assert_eq!(env.found("/x").mount().attrs() & 0x70, 0x10, "kept");

    // (5) A plain bind over a mount locked in place below is refused.
    assert_eq!(env.bind("/y", "/c1", false), Err(Errno::EINVAL));
    assert_eq!(env.vfs.clone_mount(&env.root, &env.found("/y"), false).err(), Some(Errno::EINVAL));
    // (4) A recursive one works: its top is the caller's, the copies below
    // stay locked onto it and keep their flag locks.
    env.bind("/y", "/c1", true).unwrap();
    assert_eq!(env.read("/c1/in/f").unwrap(), b"f");
    assert_eq!(locks(&env, "/c1"), lock::ATIME);
    assert_eq!(env.unmount("/c1/in", false), Err(Errno::EINVAL));
    assert_eq!(env.remount_flags("/c1/in", none, None), Err(Errno::EPERM));
    assert_eq!(env.unmount("/c1", false), Err(Errno::EBUSY));
    env.unmount("/c1", true).unwrap();
    assert_eq!(env.stat("/c1/in/f"), Err(Errno::ENOENT));
    // The flag locks travel with a copy, which can be unmounted.
    env.bind("/x", "/c2", false).unwrap();
    assert!(env.found("/c2").mount().flags().no_exec, "a bind has the flags of its source");
    assert_eq!(env.remount_flags("/c2", none, None), Err(Errno::EPERM));
    assert_eq!(env.remount_flags("/c2", noexec, None), Ok(()));
    env.unmount("/c2", false).unwrap();
    env.bind("/y/in", "/c3", false).unwrap();
    env.unmount("/c3", false).unwrap();

    // (6) A mount on top of a locked one is not locked.
    env.new_tmpfs("/x");
    assert_eq!(locks(&env, "/x"), 0);
    env.unmount("/x", false).unwrap();
    assert_eq!(env.unmount("/x", false), Err(Errno::EINVAL));

    // A plain copy of the namespace keeps the locks; nothing is added.
    env.mkdir("/own");
    env.new_tmpfs("/own");
    env.vfs.copy_namespace(&mut env.ctx, false).unwrap();
    assert_eq!(locks(&env, "/x"), lock::PLACE | lock::NO_EXEC | lock::ATIME);
    assert_eq!(locks(&env, "/own"), 0);
    env.unmount("/own", false).unwrap();
}

#[test]
fn copies_of_a_mount_that_pins_its_root_pin_it_too() {
    let mut env = Env::new();
    env.mkdir("/a");
    env.mkdir("/t");
    env.write("/a/f", b"file");
    env.write("/b", b"");
    // A bind of something that is not the root of its file system pins it.
    env.bind("/a/f", "/b", false).unwrap();
    let pins = |env: &Env, p: &str| env.found(p).mount().pins_root.load(core::sync::atomic::Ordering::Relaxed);
    assert!(pins(&env, "/b"));
    // A bind of the bind, a copy made by propagation, and a copy of the
    // namespace.
    env.write("/c", b"");
    env.bind("/b", "/c", false).unwrap();
    assert!(pins(&env, "/c"));
    env.new_tmpfs("/t");
    env.write("/t/f", b"");
    env.mkdir("/peer");
    let t = env.found("/t").mount().clone();
    env.vfs.set_propagation(core::slice::from_ref(&t), crate::propagation::MS_SHARED);
    env.bind("/t", "/peer", false).unwrap();
    env.bind("/a/f", "/t/f", false).unwrap();
    assert!(pins(&env, "/t/f") && pins(&env, "/peer/f"));
    let first = env.found("/b").mount().id();
    env.vfs.copy_namespace(&mut env.ctx, false).unwrap();
    assert_ne!(env.found("/b").mount().id(), first);
    assert!(pins(&env, "/b") && pins(&env, "/c") && pins(&env, "/peer/f"));
    assert_eq!(env.read("/peer/f").unwrap(), b"file");
}
