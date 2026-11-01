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
    assert_eq!(env.vfs.move_mount(&env.root, &env.found("/"), &env.found("/a")), Err(Errno::EINVAL));
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

#[test]
fn pivot_root_in_a_copied_namespace() {
    let mut env = Env::new();
    env.mkdir("/new");
    env.write("/top", b"t");
    let name = env.vfs.copy_namespace(&mut env.ctx).unwrap();
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
    env.vfs.copy_namespace(&mut env.ctx).unwrap();
    env.new_tmpfs("/new");
    env.cd("/new").unwrap();
    let new = env.found("/new");
    let mut ctx = env.ctx.clone();
    env.vfs.pivot_root(&mut ctx, &env.root, &new, &new).unwrap();
    env.ctx = ctx;
    assert!(env.stat("/new").is_ok(), "the old root is on top");
    env.vfs.umount(&env.ctx, &env.root, At::Cwd, b".", true).unwrap();
    assert_eq!(env.cwd(), b"/");
    assert_eq!(env.stat("/new"), Err(Errno::ENOENT));
}

#[test]
fn copies_keep_attributes() {
    let mut env = Env::new();
    env.mkdir("/a");
    env.new_tmpfs("/a");
    let m = env.found("/a").mount().clone();
    m.set_attrs(0x80);
    m.set_propagation(1 << 20);
    assert_eq!(env.found("/a").statfs().unwrap().flags & ST_NODIRATIME_BIT, ST_NODIRATIME_BIT);
    env.vfs.copy_namespace(&mut env.ctx).unwrap();
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
