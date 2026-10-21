//! Extended attributes through the VFS on tmpfs (xattr(7), getxattr(2),
//! setxattr(2), listxattr(2), removexattr(2), acl(5)), and tmpfs's
//! directory sizes.

use super::*;
use crate::xattr::{XATTR_CREATE, XATTR_REPLACE, XATTR_SIZE_MAX, acl};
use alloc::vec;

fn node(e: &Env, path: &str, follow: bool) -> NodeRef {
    e.vfs.lookup(&e.ctx, &e.root, At::Cwd, path.as_bytes(), follow).unwrap()
}

fn user() -> Cred {
    Cred::user(1000, 1000).with_caps(CapSet::EMPTY)
}

#[test]
fn a_new_file_has_none() {
    let e = Env::new();
    e.write("/f", b"x");
    let n = node(&e, "/f", true);
    assert_eq!(n.listxattr(&e.root), Ok(Vec::new()));
    assert_eq!(n.getxattr(&e.root, b"security.selinux"), Err(Errno::ENODATA));
    assert_eq!(n.getxattr(&e.root, b"system.posix_acl_access"), Err(Errno::ENODATA));
    assert_eq!(n.getxattr(&e.root, b"system.posix_acl_default"), Err(Errno::ENODATA));
    assert_eq!(n.getxattr(&e.root, b"user.none"), Err(Errno::ENODATA));
    assert_eq!(n.removexattr(&e.root, b"user.none"), Err(Errno::ENODATA));
    // Names: unknown namespace, nothing after the prefix, sizes.
    assert_eq!(n.getxattr(&e.root, b"bogus.x"), Err(Errno::EOPNOTSUPP));
    assert_eq!(n.getxattr(&e.root, b"system.other"), Err(Errno::EOPNOTSUPP));
    assert_eq!(n.setxattr(&e.root, b"system.other", b"", 0), Err(Errno::EOPNOTSUPP));
    assert_eq!(n.getxattr(&e.root, b"user."), Err(Errno::EINVAL));
    assert_eq!(n.getxattr(&e.root, b""), Err(Errno::ERANGE));
}

#[test]
fn set_get_list_remove() {
    let e = Env::new();
    e.write("/f", b"x");
    let n = node(&e, "/f", true);
    let before = n.stat().unwrap().ctime;
    assert_eq!(n.setxattr(&e.root, b"user.b", b"two", 0), Ok(()));
    assert_ne!(n.stat().unwrap().ctime, before, "a change sets ctime");
    assert_eq!(n.setxattr(&e.root, b"user.a", b"", XATTR_CREATE), Ok(()));
    assert_eq!(n.setxattr(&e.root, b"security.label", b"lbl", 0), Ok(()));
    assert_eq!(n.setxattr(&e.root, b"trusted.t", b"tt", 0), Ok(()));
    assert_eq!(n.getxattr(&e.root, b"user.b"), Ok(b"two".to_vec()));
    assert_eq!(n.getxattr(&e.root, b"user.a"), Ok(Vec::new()));
    assert_eq!(n.listxattr(&e.root), Ok(b"security.label\0trusted.t\0user.a\0user.b\0".to_vec()));
    assert_eq!(n.setxattr(&e.root, b"user.a", b"1", XATTR_CREATE), Err(Errno::EEXIST));
    assert_eq!(n.setxattr(&e.root, b"user.c", b"1", XATTR_REPLACE), Err(Errno::ENODATA));
    assert_eq!(n.setxattr(&e.root, b"user.a", b"1", XATTR_REPLACE), Ok(()));
    assert_eq!(n.setxattr(&e.root, b"user.a", b"1", 4), Err(Errno::EINVAL));
    assert_eq!(n.setxattr(&e.root, b"user.big", &vec![0; XATTR_SIZE_MAX + 1], 0), Err(Errno::E2BIG));
    assert_eq!(n.setxattr(&e.root, b"user.big", &vec![7; XATTR_SIZE_MAX], 0), Ok(()));
    assert_eq!(n.getxattr(&e.root, b"user.big").map(|v| v.len()), Ok(XATTR_SIZE_MAX));
    assert_eq!(n.removexattr(&e.root, b"user.b"), Ok(()));
    assert_eq!(n.getxattr(&e.root, b"user.b"), Err(Errno::ENODATA));
    // The attributes go with the inode: through a hard link, and after a rename.
    e.vfs.link(&e.ctx, &e.root, At::Cwd, b"/f", At::Cwd, b"/g", false).unwrap();
    assert_eq!(node(&e, "/g", true).getxattr(&e.root, b"user.a"), Ok(b"1".to_vec()));
    e.vfs.rename(&e.ctx, &e.root, At::Cwd, b"/g", At::Cwd, b"/h", RenameFlags::default()).unwrap();
    assert_eq!(node(&e, "/h", true).getxattr(&e.root, b"security.label"), Ok(b"lbl".to_vec()));
    // Directories have them too.
    e.mkdir("/d");
    let d = node(&e, "/d", true);
    assert_eq!(d.setxattr(&e.root, b"user.dir", b"yes", 0), Ok(()));
    assert_eq!(d.listxattr(&e.root), Ok(b"user.dir\0".to_vec()));
}

#[test]
fn who_may() {
    let e = Env::new();
    e.write("/f", b"x");
    let n = node(&e, "/f", true);
    let u = user();
    n.setxattr(&e.root, b"user.a", b"1", 0).unwrap();
    n.setxattr(&e.root, b"trusted.t", b"2", 0).unwrap();
    n.setxattr(&e.root, b"security.s", b"3", 0).unwrap();
    // 0644, owned by root: others read user. attributes, cannot change them.
    assert_eq!(n.getxattr(&u, b"user.a"), Ok(b"1".to_vec()));
    assert_eq!(n.setxattr(&u, b"user.a", b"x", 0), Err(Errno::EACCES));
    assert_eq!(n.removexattr(&u, b"user.a"), Err(Errno::EACCES));
    // trusted.: hidden without CAP_SYS_ADMIN; security.: readable, not writable.
    assert_eq!(n.getxattr(&u, b"trusted.t"), Err(Errno::ENODATA));
    assert_eq!(n.listxattr(&u), Ok(b"security.s\0user.a\0".to_vec()));
    assert_eq!(n.setxattr(&u, b"trusted.t", b"x", 0), Err(Errno::EPERM));
    assert_eq!(n.getxattr(&u, b"security.s"), Ok(b"3".to_vec()));
    assert_eq!(n.setxattr(&u, b"security.s", b"x", 0), Err(Errno::EPERM));
    // Without read permission (0600).
    n.chmod(&e.root, 0o600).unwrap();
    assert_eq!(n.getxattr(&u, b"user.a"), Err(Errno::EACCES));
    // user. attributes only on regular files and directories.
    e.symlink("f", "/l");
    let l = node(&e, "/l", false);
    assert_eq!(l.setxattr(&e.root, b"user.a", b"1", 0), Err(Errno::EPERM));
    assert_eq!(l.getxattr(&e.root, b"user.a"), Err(Errno::ENODATA));
    assert_eq!(l.setxattr(&e.root, b"security.s", b"1", 0), Ok(()));
    assert_eq!(l.listxattr(&e.root), Ok(b"security.s\0".to_vec()));
    // A sticky directory: only its owner (or CAP_FOWNER).
    e.vfs.mkdir(&e.ctx, &e.root, At::Cwd, b"/t", 0o1777).unwrap();
    let t = node(&e, "/t", true);
    t.chmod(&e.root, 0o1777).unwrap();
    assert_eq!(t.setxattr(&u, b"user.a", b"1", 0), Err(Errno::EPERM));
    assert_eq!(t.setxattr(&e.root, b"user.a", b"1", 0), Ok(()));
}

#[test]
fn minimal_acls_are_the_permission_bits() {
    let e = Env::new();
    e.write("/f", b"x");
    let n = node(&e, "/f", true);
    n.chmod(&e.root, 0o600).unwrap();
    // A program copying a file's ACL to its new copy (sed -i, cp -p).
    assert_eq!(n.setxattr(&e.root, b"system.posix_acl_access", &acl::from_mode(0o754), 0), Ok(()));
    assert_eq!(n.stat().unwrap().perm(), 0o754);
    assert_eq!(n.getxattr(&e.root, b"system.posix_acl_access"), Err(Errno::ENODATA));
    assert_eq!(n.listxattr(&e.root), Ok(Vec::new()));
    // Only the owner (or CAP_FOWNER) sets an ACL.
    assert_eq!(n.setxattr(&user(), b"system.posix_acl_access", &acl::from_mode(0o777), 0), Err(Errno::EPERM));
    // Invalid values; ACLs the bits cannot express; default ACLs.
    assert_eq!(n.setxattr(&e.root, b"system.posix_acl_access", b"\x02\0\0\0xyz", 0), Err(Errno::EINVAL));
    let mut named = acl::from_mode(0o750).to_vec();
    named.splice(12..12, [2u8, 0, 5, 0, 0xe8, 3, 0, 0]);
    named.splice(28..28, [0x10u8, 0, 5, 0, 0xff, 0xff, 0xff, 0xff]);
    assert_eq!(acl::parse(&named).map(|s| s.minimal), Ok(false));
    assert_eq!(n.setxattr(&e.root, b"system.posix_acl_access", &named, 0), Err(Errno::EOPNOTSUPP));
    e.mkdir("/d");
    let d = node(&e, "/d", true);
    assert_eq!(d.setxattr(&e.root, b"system.posix_acl_default", &acl::from_mode(0o755), 0), Err(Errno::EOPNOTSUPP));
    assert_eq!(n.stat().unwrap().perm(), 0o754);
    assert_eq!(n.removexattr(&e.root, b"system.posix_acl_access"), Err(Errno::ENODATA));
}

#[test]
fn read_only_mounts_refuse_changes() {
    let e = Env::new();
    e.mkdir("/ro");
    let fs = tmpfs();
    e.vfs.mount(&e.ctx, &e.root, At::Cwd, b"/ro", fs, MountFlags { read_only: true, ..MountFlags::default() }).unwrap();
    let n = node(&e, "/ro", true);
    assert_eq!(n.setxattr(&e.root, b"user.a", b"1", 0), Err(Errno::EROFS));
    assert_eq!(n.removexattr(&e.root, b"user.a"), Err(Errno::EROFS));
    assert_eq!(n.listxattr(&e.root), Ok(Vec::new()));
}

#[test]
fn directory_sizes() {
    let e = Env::new();
    e.mkdir("/d");
    assert_eq!(e.stat("/d").map(|s| (s.size, s.blocks)), Ok((40, 0)));
    e.write("/d/a", b"x");
    assert_eq!(e.stat("/d").map(|s| s.size), Ok(60));
    e.mkdir("/d/sub");
    e.symlink("a", "/d/l");
    for i in 0..5 {
        e.write(&std::format!("/d/f{i}"), b"");
    }
    assert_eq!(e.stat("/d").map(|s| s.size), Ok(200));
    e.vfs.unlink(&e.ctx, &e.root, At::Cwd, b"/d/a").unwrap();
    assert_eq!(e.stat("/d").map(|s| s.size), Ok(180));
    // A small file takes one page (8 blocks of 512 bytes), an empty one none.
    assert_eq!(e.lstat("/d/l").map(|s| s.blocks), Ok(0));
    e.write("/d/f0", b"some bytes");
    assert_eq!(e.stat("/d/f0").map(|s| s.blocks), Ok(8));
    assert_eq!(e.stat("/d/f1").map(|s| s.blocks), Ok(0));
}
