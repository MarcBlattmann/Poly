//! Inode flags (file_getattr(2), file_setattr(2)) and file handles
//! (name_to_handle_at(2), open_by_handle_at(2)) on tmpfs.

use super::*;
use crate::fileattr::{FileAttr, FileHandle, xflag};

fn node(e: &Env, path: &str, follow: bool) -> NodeRef {
    e.vfs.lookup(&e.ctx, &e.root, At::Cwd, path.as_bytes(), follow).unwrap()
}

fn flags(x: u32) -> FileAttr {
    FileAttr { xflags: u64::from(x), ..FileAttr::default() }
}

#[test]
fn flags_start_clear_and_replace() {
    let e = Env::new();
    e.write("/f", b"x");
    let n = node(&e, "/f", true);
    assert_eq!(n.file_getattr(), Ok(FileAttr::default()));
    let before = n.stat().unwrap().ctime;
    assert_eq!(n.file_setattr(&e.root, &flags(xflag::NODUMP)), Ok(()));
    assert_ne!(n.stat().unwrap().ctime, before, "a change sets ctime");
    assert_eq!(n.file_getattr().unwrap().xflags, u64::from(xflag::NODUMP));
    // nextents is ignored; the bits replace the old ones.
    let mut a = flags(xflag::NOATIME);
    a.nextents = 5;
    assert_eq!(n.file_setattr(&e.root, &a), Ok(()));
    assert_eq!(n.file_getattr(), Ok(flags(xflag::NOATIME)));
    // Directories have them too.
    e.mkdir("/d");
    assert_eq!(node(&e, "/d", true).file_getattr(), Ok(FileAttr::default()));
}

#[test]
fn flag_errors() {
    let e = Env::new();
    e.write("/f", b"x");
    e.symlink("/f", "/l");
    let n = node(&e, "/f", true);
    assert_eq!(n.file_setattr(&e.root, &FileAttr { xflags: 1 << 40, ..FileAttr::default() }), Err(Errno::EINVAL));
    assert_eq!(n.file_setattr(&e.root, &flags(xflag::REALTIME)), Err(Errno::EOPNOTSUPP));
    assert_eq!(n.file_setattr(&e.root, &FileAttr { projid: 5, ..FileAttr::default() }), Err(Errno::EOPNOTSUPP));
    assert_eq!(n.file_setattr(&e.root, &FileAttr { extsize: 4096, ..FileAttr::default() }), Err(Errno::EOPNOTSUPP));
    let link = node(&e, "/l", false);
    assert_eq!(link.file_getattr(), Err(Errno::EOPNOTSUPP));
    assert_eq!(link.file_setattr(&e.root, &flags(0)), Err(Errno::EOPNOTSUPP));
    // Not the owner: EPERM; the owner without CAP_LINUX_IMMUTABLE may
    // change the others but not APPEND / IMMUTABLE.
    let user = Cred::user(1000, 1000).with_caps(CapSet::EMPTY);
    assert_eq!(n.file_setattr(&user, &flags(xflag::NODUMP)), Err(Errno::EPERM));
    n.chown(&e.root, Some(1000), Some(1000)).unwrap();
    assert_eq!(n.file_setattr(&user, &flags(xflag::NODUMP)), Ok(()));
    assert_eq!(n.file_setattr(&user, &flags(xflag::APPEND)), Err(Errno::EPERM));
    assert_eq!(n.file_setattr(&e.root, &flags(xflag::APPEND)), Ok(()));
    assert_eq!(n.file_setattr(&user, &flags(0)), Err(Errno::EPERM), "clearing needs it too");
}

#[test]
fn append_and_immutable_take_effect() {
    let e = Env::new();
    e.write("/a", b"x");
    e.write("/i", b"x");
    node(&e, "/a", true).file_setattr(&e.root, &flags(xflag::APPEND)).unwrap();
    node(&e, "/i", true).file_setattr(&e.root, &flags(xflag::IMMUTABLE)).unwrap();
    assert_eq!(e.open("/a", OpenFlags::WRONLY).err(), Some(Errno::EPERM));
    assert!(e.open("/a", OpenFlags::WRONLY | OpenFlags::APPEND).is_ok());
    assert!(e.open("/a", OpenFlags::RDONLY).is_ok());
    assert_eq!(e.open("/i", OpenFlags::WRONLY | OpenFlags::APPEND).err(), Some(Errno::EPERM));
    assert_eq!(e.open("/i", OpenFlags::RDONLY | OpenFlags::TRUNC).err(), Some(Errno::EPERM));
    assert!(e.open("/i", OpenFlags::RDONLY).is_ok());
    assert_eq!(e.vfs.unlink(&e.ctx, &e.root, At::Cwd, b"/i"), Err(Errno::EPERM));
    assert_eq!(e.vfs.unlink(&e.ctx, &e.root, At::Cwd, b"/a"), Err(Errno::EPERM));
    node(&e, "/i", true).file_setattr(&e.root, &flags(0)).unwrap();
    assert_eq!(e.vfs.unlink(&e.ctx, &e.root, At::Cwd, b"/i"), Ok(()));
}

#[test]
fn handles_name_inodes() {
    let e = Env::new();
    e.write("/f", b"abc");
    e.write("/g", b"xyz");
    let h = node(&e, "/f", true).file_handle().unwrap();
    assert_eq!(h.bytes().len(), 12);
    assert_eq!(node(&e, "/f", true).file_handle(), Ok(h), "stable");
    assert_ne!(node(&e, "/g", true).file_handle(), Ok(h));
    // A rename keeps it; a new file under the old name gets another.
    e.vfs.rename(&e.ctx, &e.root, At::Cwd, b"/f", At::Cwd, b"/r", crate::RenameFlags::default()).unwrap();
    assert_eq!(node(&e, "/r", true).file_handle(), Ok(h));
    e.write("/f", b"new");
    assert_ne!(node(&e, "/f", true).file_handle(), Ok(h));
    // Open it by handle, whatever its name.
    let mount = node(&e, "/", true).mount().clone();
    let ro = e.vfs.open_handle(&e.root, &mount, &h, OpenFlags::RDONLY).unwrap();
    assert_eq!(read_all(&ro), Ok(b"abc".to_vec()));
    drop(ro);
    let file = e.vfs.open_handle(&e.root, &mount, &h, OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    assert_eq!(file.write_at(0, b"A"), Ok(1));
    assert_eq!(e.read("/r"), Ok(b"Abc".to_vec()));
    // Corrupt bytes, another tmpfs's handle: ESTALE.
    let mut bad = h.bytes().to_vec();
    for b in &mut bad {
        *b ^= 0xff;
    }
    let bad = FileHandle::new(h.kind, &bad);
    assert_eq!(e.vfs.open_handle(&e.root, &mount, &bad, OpenFlags::RDONLY).err(), Some(Errno::ESTALE));
    let other = Env::new();
    other.write("/f", b"abc");
    let foreign = node(&other, "/f", true).file_handle().unwrap();
    assert_eq!(e.vfs.open_handle(&e.root, &mount, &foreign, OpenFlags::RDONLY).err(), Some(Errno::ESTALE));
    // A removed file opens while a descriptor keeps it, then is stale.
    e.vfs.unlink(&e.ctx, &e.root, At::Cwd, b"/r").unwrap();
    assert!(e.vfs.open_handle(&e.root, &mount, &h, OpenFlags::RDONLY).is_ok());
    drop(file);
    assert_eq!(e.vfs.open_handle(&e.root, &mount, &h, OpenFlags::RDONLY).err(), Some(Errno::ESTALE));
}

#[test]
fn handle_opens_directories_and_paths() {
    let e = Env::new();
    e.mkdir("/d");
    let h = node(&e, "/d", true).file_handle().unwrap();
    let mount = node(&e, "/", true).mount().clone();
    let dir = e.vfs.open_handle(&e.root, &mount, &h, OpenFlags::RDONLY | OpenFlags::DIRECTORY).unwrap();
    assert_eq!(dir.node().kind(), Ok(FileType::Directory));
    assert_eq!(e.vfs.open_handle(&e.root, &mount, &h, OpenFlags::RDWR).err(), Some(Errno::EISDIR));
    let p = e.vfs.open_handle(&e.root, &mount, &h, OpenFlags::PATH).unwrap();
    assert!(p.is_path_only());
}
