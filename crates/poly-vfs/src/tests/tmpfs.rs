//! tmpfs and the directory-changing calls: `rename(2)` (over existing
//! names, `RENAME_NOREPLACE`, `RENAME_EXCHANGE`), `link`, `unlink`,
//! `rmdir`, link counts, space limits.

use super::{Env, TestClock, read_all};
use crate::*;
use alloc::sync::Arc;

fn rename(env: &Env, old: &str, new: &str) -> Result<()> {
    rename_flags(env, old, new, RenameFlags::default())
}

fn rename_flags(env: &Env, old: &str, new: &str, flags: RenameFlags) -> Result<()> {
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, old.as_bytes(), At::Cwd, new.as_bytes(), flags)
}

fn rmdir(env: &Env, path: &str) -> Result<()> {
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, path.as_bytes())
}

fn unlink(env: &Env, path: &str) -> Result<()> {
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, path.as_bytes())
}

fn link(env: &Env, old: &str, new: &str) -> Result<()> {
    env.vfs.link(&env.ctx, &env.root, At::Cwd, old.as_bytes(), At::Cwd, new.as_bytes(), false)
}

#[test]
fn rename_over_an_existing_file() {
    let env = Env::new();
    env.write("/src", b"new contents");
    env.write("/dst", b"old contents");
    let inodes = env.fs.inode_count();
    let src_ino = env.ino("/src").unwrap();
    rename(&env, "/src", "/dst").unwrap();
    assert_eq!(env.read("/dst").unwrap(), b"new contents");
    assert_eq!(env.ino("/dst"), Ok(src_ino));
    assert_eq!(env.stat("/src"), Err(Errno::ENOENT));
    assert_eq!(env.fs.inode_count(), inodes - 1, "the replaced file is freed");
}

#[test]
fn rename_over_an_open_file_keeps_it_for_its_reader() {
    let env = Env::new();
    env.write("/src", b"new");
    env.write("/dst", b"old");
    let reader = env.open("/dst", OpenFlags::RDONLY).unwrap();
    rename(&env, "/src", "/dst").unwrap();
    assert_eq!(read_all(&reader).unwrap(), b"old");
    assert_eq!(reader.stat().unwrap().nlink, 0);
    assert_eq!(env.read("/dst").unwrap(), b"new");
    let inodes = env.fs.inode_count();
    drop(reader);
    assert_eq!(env.fs.inode_count(), inodes - 1);
}

#[test]
fn rename_type_rules() {
    let env = Env::new();
    env.mkdir("/d1");
    env.mkdir("/d2");
    env.mkdir("/full");
    env.write("/full/x", b"");
    env.write("/f", b"");
    assert_eq!(rename(&env, "/f", "/d1"), Err(Errno::EISDIR));
    assert_eq!(rename(&env, "/d1", "/f"), Err(Errno::ENOTDIR));
    assert_eq!(rename(&env, "/d1", "/full"), Err(Errno::ENOTEMPTY));
    assert_eq!(rename(&env, "/missing", "/x"), Err(Errno::ENOENT));
    // A directory over an empty directory replaces it.
    let d1 = env.ino("/d1").unwrap();
    rename(&env, "/d1", "/d2").unwrap();
    assert_eq!(env.ino("/d2"), Ok(d1));
    // Into itself or below itself: EINVAL.
    env.mkdir("/d2/sub");
    assert_eq!(rename(&env, "/d2", "/d2/sub/x"), Err(Errno::EINVAL));
    assert_eq!(rename(&env, "/d2", "/d2/x"), Err(Errno::EINVAL));
    // "." and ".." cannot be renamed, nor renamed onto.
    assert_eq!(rename(&env, "/d2/.", "/y"), Err(Errno::EBUSY));
    assert_eq!(rename(&env, "/f", "/d2/.."), Err(Errno::EBUSY));
    assert_eq!(rename(&env, "/", "/y"), Err(Errno::EBUSY));
    // A trailing slash on a file: ENOTDIR; on a directory: fine.
    assert_eq!(rename(&env, "/f/", "/g"), Err(Errno::ENOTDIR));
    assert_eq!(rename(&env, "/f", "/g/"), Err(Errno::ENOTDIR));
    rename(&env, "/d2/", "/d3/").unwrap();
    // Two names of one file: nothing happens, both stay.
    link(&env, "/f", "/f2").unwrap();
    rename(&env, "/f", "/f2").unwrap();
    assert!(env.stat("/f").is_ok() && env.stat("/f2").is_ok());
    // Renaming onto itself is fine.
    rename(&env, "/f", "/f").unwrap();
}

#[test]
fn renameat2_flags() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/a", b"A");
    env.write("/d/b", b"B");
    let noreplace = RenameFlags { noreplace: true, ..RenameFlags::default() };
    let exchange = RenameFlags { exchange: true, ..RenameFlags::default() };
    assert_eq!(rename_flags(&env, "/a", "/d/b", noreplace), Err(Errno::EEXIST));
    assert_eq!(rename_flags(&env, "/a", "/a", noreplace), Err(Errno::EEXIST), "also onto itself");
    env.vfs.link(&env.ctx, &env.root, At::Cwd, b"/a", At::Cwd, b"/a2", false).unwrap();
    assert_eq!(rename_flags(&env, "/a", "/a2", noreplace), Err(Errno::EEXIST), "and onto its own hard link");
    unlink(&env, "/a2").unwrap();
    rename_flags(&env, "/a", "/a", exchange).unwrap();
    rename_flags(&env, "/a", "/d/c", noreplace).unwrap();
    rename_flags(&env, "/d/c", "/a", noreplace).unwrap();
    // Exchange two files.
    rename_flags(&env, "/a", "/d/b", exchange).unwrap();
    assert_eq!(env.read("/a").unwrap(), b"B");
    assert_eq!(env.read("/d/b").unwrap(), b"A");
    assert_eq!(rename_flags(&env, "/a", "/nope", exchange), Err(Errno::ENOENT));
    // Exchange a file and a directory across parents: link counts follow.
    env.mkdir("/e");
    env.mkdir("/e/sub");
    assert_eq!(env.stat("/e").unwrap().nlink, 3);
    assert_eq!(env.stat("/d").unwrap().nlink, 2);
    rename_flags(&env, "/e/sub", "/d/b", exchange).unwrap();
    assert_eq!(env.stat("/d/b").unwrap().kind(), Some(FileType::Directory));
    assert_eq!(env.stat("/e/sub").unwrap().kind(), Some(FileType::Regular));
    assert_eq!(env.stat("/e").unwrap().nlink, 2);
    assert_eq!(env.stat("/d").unwrap().nlink, 3);
    assert_eq!(env.ino("/d/b/.."), env.ino("/d"));
    // A directory cannot be exchanged with something below it.
    env.mkdir("/d/b/deep");
    assert_eq!(rename_flags(&env, "/d/b/deep", "/d", exchange), Err(Errno::EINVAL));
    assert_eq!(RenameFlags::from_linux(1), Ok(noreplace));
    assert_eq!(RenameFlags::from_linux(2), Ok(exchange));
    assert_eq!(RenameFlags::from_linux(3), Err(Errno::EINVAL));
    assert_eq!(RenameFlags::from_linux(4), Err(Errno::EINVAL));
    assert_eq!(rename_flags(&env, "/a", "/x", RenameFlags { noreplace: true, exchange: true }), Err(Errno::EINVAL));
}

#[test]
fn link_counts_and_hard_links() {
    let env = Env::new();
    assert_eq!(env.stat("/").unwrap().nlink, 2);
    env.mkdir("/d");
    env.mkdir("/d/a");
    env.mkdir("/d/b");
    assert_eq!(env.stat("/").unwrap().nlink, 3);
    assert_eq!(env.stat("/d").unwrap().nlink, 4);
    rename(&env, "/d/a", "/a").unwrap();
    assert_eq!(env.stat("/d").unwrap().nlink, 3);
    assert_eq!(env.stat("/").unwrap().nlink, 4);
    rmdir(&env, "/d/b").unwrap();
    assert_eq!(env.stat("/d").unwrap().nlink, 2);

    env.write("/f", b"shared");
    link(&env, "/f", "/d/g").unwrap();
    assert_eq!(env.stat("/f").unwrap().nlink, 2);
    assert_eq!(env.ino("/f"), env.ino("/d/g"));
    unlink(&env, "/f").unwrap();
    assert_eq!(env.read("/d/g").unwrap(), b"shared");
    assert_eq!(env.stat("/d/g").unwrap().nlink, 1);
    assert_eq!(link(&env, "/d", "/d2"), Err(Errno::EPERM));
    assert_eq!(link(&env, "/d/g", "/a"), Err(Errno::EEXIST));
    assert_eq!(link(&env, "/nope", "/x"), Err(Errno::ENOENT));
    // link() does not follow a symlink; with AT_SYMLINK_FOLLOW it does.
    env.symlink("/d/g", "/sl");
    link(&env, "/sl", "/sl2").unwrap();
    assert_eq!(env.lstat("/sl2").unwrap().kind(), Some(FileType::Symlink));
    env.vfs.link(&env.ctx, &env.root, At::Cwd, b"/sl", At::Cwd, b"/g3", true).unwrap();
    assert_eq!(env.ino("/g3"), env.ino("/d/g"));
    // An unlinked open file cannot get a new name.
    let f = env.open("/d/g", OpenFlags::RDONLY).unwrap();
    unlink(&env, "/d/g").unwrap();
    unlink(&env, "/g3").unwrap();
    assert_eq!(env.fs.link(env.fs.root(), b"back", f.node().ino()), Err(Errno::ENOENT));
}

#[test]
fn removal_rules() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/f", b"");
    env.write("/f", b"");
    assert_eq!(unlink(&env, "/d"), Err(Errno::EISDIR));
    assert_eq!(rmdir(&env, "/f"), Err(Errno::ENOTDIR));
    assert_eq!(rmdir(&env, "/d"), Err(Errno::ENOTEMPTY));
    assert_eq!(rmdir(&env, "/d/."), Err(Errno::EINVAL));
    assert_eq!(rmdir(&env, "/d/.."), Err(Errno::ENOTEMPTY));
    assert_eq!(rmdir(&env, "/"), Err(Errno::EBUSY));
    assert_eq!(unlink(&env, "/"), Err(Errno::EISDIR));
    assert_eq!(unlink(&env, "/d/."), Err(Errno::EISDIR));
    assert_eq!(unlink(&env, "/nope"), Err(Errno::ENOENT));
    assert_eq!(rmdir(&env, "/nope"), Err(Errno::ENOENT));
    let ctx = &env.ctx;
    assert_eq!(env.vfs.mkdir(ctx, &env.root, At::Cwd, b"/", 0o755), Err(Errno::EEXIST));
    assert_eq!(env.vfs.mkdir(ctx, &env.root, At::Cwd, b"/d/.", 0o755), Err(Errno::EEXIST));
    assert_eq!(env.vfs.mkdir(ctx, &env.root, At::Cwd, b"/d/..", 0o755), Err(Errno::EEXIST));
    assert_eq!(env.vfs.mkdir(ctx, &env.root, At::Cwd, b"/nope/x", 0o755), Err(Errno::ENOENT));
    unlink(&env, "/d/f").unwrap();
    rmdir(&env, "/d").unwrap();
    assert_eq!(env.stat("/d"), Err(Errno::ENOENT));
    assert_eq!(env.ls("/"), ["f"]);
}

#[test]
fn rmdir_of_an_open_directory() {
    let env = Env::new();
    env.mkdir("/d");
    let d = env.open("/d", OpenFlags::RDONLY).unwrap();
    let inodes = env.fs.inode_count();
    rmdir(&env, "/d").unwrap();
    assert_eq!(d.stat().unwrap().nlink, 0);
    let mut n = 0;
    d.read_dir(&mut |_| {
        n += 1;
        true
    })
    .unwrap();
    assert_eq!(n, 0, "a removed directory lists nothing");
    assert_eq!(env.fs.inode_count(), inodes);
    drop(d);
    assert_eq!(env.fs.inode_count(), inodes - 1);
}

#[test]
fn space_limit() {
    let fs = Arc::new(Tmpfs::with_limit(Arc::new(TestClock::default()), 3 * 4096));
    let vfs = Vfs::new(fs.clone());
    let env = Env { ctx: vfs.root_context().unwrap(), vfs, fs, root: Cred::root() };
    let f = env.open("/f", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    assert_eq!(f.write(&[1u8; 8192]), Ok(8192));
    assert_eq!(f.write(&[1u8; 8192]), Err(Errno::ENOSPC));
    assert_eq!(f.write(&[1u8; 4096]), Ok(4096));
    assert_eq!(f.write(b"x"), Err(Errno::ENOSPC));
    // Rewriting allocated pages needs no space.
    assert_eq!(f.write_at(0, &[2u8; 100]), Ok(100));
    // Holes need no space either.
    f.truncate(1 << 30).unwrap();
    assert_eq!(env.fs.bytes_used(), 3 * 4096);
    f.truncate(4096).unwrap();
    assert_eq!(env.fs.bytes_used(), 4096);
    drop(f);
    unlink(&env, "/f").unwrap();
    assert_eq!(env.fs.bytes_used(), 0);
}

#[test]
fn direct_file_system_calls() {
    let fs = super::tmpfs();
    let root = fs.root();
    let info = fs.info();
    assert_eq!(info.name, "tmpfs");
    assert!(!info.cache_lookups);
    let dir = fs.mkdir(root, b"d", 0o750, 5, 6).unwrap();
    let a = fs.getattr(dir).unwrap();
    assert_eq!((a.kind, a.perm, a.uid, a.gid, a.nlink), (FileType::Directory, 0o750, 5, 6, 2));
    assert_eq!(fs.lookup(dir, b".."), Ok(root));
    assert_eq!(fs.lookup(root, b".."), Ok(root));
    assert_eq!(fs.mkdir(root, b"d", 0o750, 5, 6), Err(Errno::EEXIST));
    assert_eq!(fs.mkdir(root, b"a/b", 0o750, 5, 6), Err(Errno::EINVAL));
    let file = fs
        .create(dir, b"f", &NewNode { kind: FileType::Regular, perm: 0o600, uid: 1, gid: 2, rdev: DeviceId::new(9, 9) })
        .unwrap();
    assert_eq!(fs.getattr(file).unwrap().rdev, DeviceId::default());
    assert_eq!(
        fs.create(file, b"x", &NewNode { kind: FileType::Regular, perm: 0, uid: 0, gid: 0, rdev: DeviceId::default() }),
        Err(Errno::ENOTDIR)
    );
    assert_eq!(
        fs.create(
            dir,
            b"y",
            &NewNode { kind: FileType::Directory, perm: 0, uid: 0, gid: 0, rdev: DeviceId::default() }
        ),
        Err(Errno::EINVAL)
    );
    assert_eq!(fs.write(file, 10, b"abc"), Ok(3));
    let mut buf = [9u8; 20];
    assert_eq!(fs.read(file, 0, &mut buf), Ok(13));
    assert_eq!(&buf[..13], b"\0\0\0\0\0\0\0\0\0\0abc");
    assert_eq!(fs.read(file, 13, &mut buf), Ok(0));
    assert_eq!(fs.read(file, 1000, &mut buf), Ok(0));
    assert_eq!(fs.read(dir, 0, &mut buf), Err(Errno::EISDIR));
    assert_eq!(fs.append(file, b"de"), Ok((13, 2)));
    assert_eq!(fs.readlink(file), Err(Errno::EINVAL));
    let l = fs.symlink(dir, b"l", b"target", 0, 0).unwrap();
    assert_eq!(fs.readlink(l).unwrap(), b"target");
    assert_eq!(fs.getattr(l).unwrap().size, 6);
    assert_eq!(fs.symlink(dir, b"l2", b"", 0, 0), Err(Errno::ENOENT));
    assert_eq!(fs.lookup(file, b"x"), Err(Errno::ENOTDIR));
    assert_eq!(fs.getattr(12345), Err(Errno::ENOENT));
    assert_eq!(fs.open(12345), Err(Errno::ENOENT));
    assert_eq!(fs.truncate(dir, 0), Err(Errno::EISDIR));
    assert_eq!(fs.truncate(l, 0), Err(Errno::EINVAL));
    // Inode numbers are never reused.
    fs.unlink(dir, b"f").unwrap();
    let again = fs
        .create(dir, b"f", &NewNode { kind: FileType::Regular, perm: 0o600, uid: 1, gid: 2, rdev: DeviceId::default() })
        .unwrap();
    assert!(again > file);
}
