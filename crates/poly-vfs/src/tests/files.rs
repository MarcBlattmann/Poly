//! Open file descriptions: `open(2)` flags, `read`/`write`/`lseek`,
//! `pread`/`pwrite`, `O_APPEND`, `ftruncate`, `getdents`, and the life of
//! an unlinked file.

use super::{Env, read_all};
use crate::*;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

#[test]
fn create_write_read_and_offsets() {
    let env = Env::new();
    let f = env.open("/f", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    assert_eq!(f.write(b"hello "), Ok(6));
    assert_eq!(f.write(b"world"), Ok(5));
    assert_eq!(f.offset(), 11);
    let mut buf = [0u8; 32];
    assert_eq!(f.read(&mut buf), Ok(0)); // at the end
    assert_eq!(f.seek(0, Whence::Set), Ok(0));
    assert_eq!(f.read(&mut buf[..5]), Ok(5));
    assert_eq!(&buf[..5], b"hello");
    assert_eq!(f.seek(-5, Whence::End), Ok(6));
    assert_eq!(f.read(&mut buf), Ok(5));
    assert_eq!(&buf[..5], b"world");
    assert_eq!(f.seek(-1, Whence::Set), Err(Errno::EINVAL));
    assert_eq!(f.seek(-20, Whence::Cur), Err(Errno::EINVAL));
    assert_eq!(f.seek(i64::MAX, Whence::End), Err(Errno::EOVERFLOW));
    assert_eq!(f.stat().unwrap().size, 11);
    assert_eq!(f.flags().intersection(OpenFlags::ACCMODE), OpenFlags::RDWR);
    // O_CREAT, O_TRUNC and O_CLOEXEC are not kept as status flags.
    assert!(!f.flags().intersects(OpenFlags::CREAT | OpenFlags::TRUNC | OpenFlags::CLOEXEC));
}

#[test]
fn holes_read_as_zeros() {
    let env = Env::new();
    let f = env.open("/sparse", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    f.seek(1 << 20, Whence::Set).unwrap();
    f.write(b"end").unwrap();
    let st = f.stat().unwrap();
    assert_eq!(st.size, (1 << 20) + 3);
    // One page holds the data; the hole costs nothing.
    assert_eq!(st.blocks, 8);
    assert_eq!(env.fs.bytes_used(), 4096);
    let mut buf = vec![0xffu8; 10];
    assert_eq!(f.read_at(4000, &mut buf), Ok(10));
    assert_eq!(buf, vec![0u8; 10]);
    assert_eq!(f.read_at((1 << 20) - 2, &mut buf), Ok(5));
    assert_eq!(&buf[..5], b"\0\0end");
    // Data/hole seeking treats the file as all data.
    assert_eq!(f.seek(10, Whence::Data), Ok(10));
    assert_eq!(f.seek(10, Whence::Hole), Ok(st.size));
    assert_eq!(f.seek(st.size as i64, Whence::Data), Err(Errno::ENXIO));
}

#[test]
fn append_and_positional_io() {
    let env = Env::new();
    env.write("/log", b"one\n");
    let a = env.open("/log", OpenFlags::WRONLY | OpenFlags::APPEND).unwrap();
    let b = env.open("/log", OpenFlags::WRONLY).unwrap();
    assert_eq!(b.write(b"XX"), Ok(2)); // overwrites at offset 0
    assert_eq!(a.write(b"two\n"), Ok(4)); // always at the end
    assert_eq!(a.offset(), 8);
    // pwrite on an O_APPEND file appends too (Linux, pwrite(2) BUGS).
    assert_eq!(a.write_at(0, b"three\n"), Ok(6));
    assert_eq!(env.read("/log").unwrap(), b"XXe\ntwo\nthree\n");
    // pread/pwrite leave the offset alone.
    let r = env.open("/log", OpenFlags::RDWR).unwrap();
    let mut buf = [0u8; 3];
    assert_eq!(r.read_at(4, &mut buf), Ok(3));
    assert_eq!(&buf, b"two");
    assert_eq!(r.write_at(0, b"ab"), Ok(2));
    assert_eq!(r.offset(), 0);
    assert_eq!(r.read_at(u64::MAX, &mut buf), Err(Errno::EINVAL));
    // F_SETFL turns O_APPEND off, and cannot change the access mode.
    a.set_flags(OpenFlags::RDWR);
    assert!(!a.flags().contains(OpenFlags::APPEND));
    assert!(!a.readable());
    a.seek(0, Whence::Set).unwrap();
    a.write(b"Z").unwrap();
    assert_eq!(&env.read("/log").unwrap()[..3], b"Zbe");
}

#[test]
fn access_modes_are_enforced_per_description() {
    let env = Env::new();
    env.write("/f", b"data");
    let ro = env.open("/f", OpenFlags::RDONLY).unwrap();
    assert_eq!(ro.write(b"x"), Err(Errno::EBADF));
    assert_eq!(ro.truncate(0), Err(Errno::EINVAL));
    let wo = env.open("/f", OpenFlags::WRONLY).unwrap();
    assert_eq!(wo.read(&mut [0u8; 4]), Err(Errno::EBADF));
    assert_eq!(env.open("/f", OpenFlags::from_bits(3)).map(|_| ()), Err(Errno::EINVAL));
}

#[test]
fn open_flag_rules() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/f", b"0123456789");
    // O_EXCL on an existing file.
    assert_eq!(env.open("/f", OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::EXCL).map(|_| ()), Err(Errno::EEXIST));
    // Without O_CREAT a missing file is ENOENT.
    assert_eq!(env.open("/missing", OpenFlags::RDONLY).map(|_| ()), Err(Errno::ENOENT));
    // O_TRUNC empties.
    env.open("/f", OpenFlags::WRONLY | OpenFlags::TRUNC).unwrap();
    assert_eq!(env.stat("/f").unwrap().size, 0);
    // Directories: readable, not writable, O_DIRECTORY checks the type.
    assert!(env.open("/d", OpenFlags::RDONLY).is_ok());
    assert_eq!(env.open("/d", OpenFlags::WRONLY).map(|_| ()), Err(Errno::EISDIR));
    assert_eq!(env.open("/d", OpenFlags::RDWR).map(|_| ()), Err(Errno::EISDIR));
    assert_eq!(env.open("/d", OpenFlags::RDONLY | OpenFlags::CREAT).map(|_| ()), Err(Errno::EISDIR));
    assert_eq!(env.open("/f", OpenFlags::RDONLY | OpenFlags::DIRECTORY).map(|_| ()), Err(Errno::ENOTDIR));
    assert_eq!(
        env.open("/g", OpenFlags::RDONLY | OpenFlags::DIRECTORY | OpenFlags::CREAT).map(|_| ()),
        Err(Errno::EINVAL)
    );
    assert_eq!(env.open("/", OpenFlags::RDONLY | OpenFlags::CREAT).map(|_| ()), Err(Errno::EISDIR));
    assert_eq!(env.open("/g", OpenFlags::RDWR | OpenFlags::TMPFILE).map(|_| ()), Err(Errno::EOPNOTSUPP));
    let d = env.open("/d", OpenFlags::RDONLY).unwrap();
    assert_eq!(d.read(&mut [0u8; 4]), Err(Errno::EISDIR));
    // The new file's mode is masked by the umask (022).
    let ctx = &env.ctx;
    env.vfs.open(ctx, &env.root, At::Cwd, b"/m", OpenFlags::WRONLY | OpenFlags::CREAT, 0o777).unwrap();
    assert_eq!(env.stat("/m").unwrap().perm(), 0o755);
}

#[test]
fn o_path_names_without_opening() {
    let env = Env::new();
    env.write("/f", b"data");
    env.symlink("/f", "/link");
    let p = env.open("/f", OpenFlags::PATH).unwrap();
    assert!(p.is_path_only());
    assert_eq!(p.read(&mut [0u8; 4]), Err(Errno::EBADF));
    assert_eq!(p.write(b"x"), Err(Errno::EBADF));
    assert_eq!(p.seek(0, Whence::Set), Err(Errno::EBADF));
    assert_eq!(p.stat().unwrap().size, 4);
    // With O_NOFOLLOW it opens the link itself.
    let l = env.open("/link", OpenFlags::PATH | OpenFlags::NOFOLLOW).unwrap();
    assert_eq!(l.kind(), FileType::Symlink);
    assert_eq!(l.node().readlink().unwrap(), b"/f");
}

#[test]
fn unlinked_file_lives_until_last_close() {
    let env = Env::new();
    env.write("/f", &[7u8; 10000]);
    let inodes = env.fs.inode_count();
    let f = env.open("/f", OpenFlags::RDWR).unwrap();
    let g = f.clone(); // a dup'd descriptor shares the description
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/f").unwrap();
    assert_eq!(env.stat("/f"), Err(Errno::ENOENT));
    assert_eq!(f.stat().unwrap().nlink, 0);
    assert_eq!(read_all(&f).unwrap(), vec![7u8; 10000]);
    g.write(b"more").unwrap();
    assert_eq!(f.stat().unwrap().size, 10004);
    assert_eq!(env.fs.inode_count(), inodes);
    assert_eq!(env.fs.bytes_used(), 3 * 4096);
    drop(f);
    assert_eq!(env.fs.inode_count(), inodes);
    drop(g);
    assert_eq!(env.fs.inode_count(), inodes - 1);
    assert_eq!(env.fs.bytes_used(), 0);
    // A new file with the old name is a new inode.
    env.write("/f", b"new");
    assert_eq!(env.read("/f").unwrap(), b"new");
}

#[test]
fn ftruncate_shrinks_and_grows_with_zeros() {
    let env = Env::new();
    let f = env.open("/t", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    f.write(&[1u8; 6000]).unwrap();
    f.truncate(100).unwrap();
    assert_eq!(f.stat().unwrap().size, 100);
    assert_eq!(env.fs.bytes_used(), 4096);
    f.truncate(5000).unwrap();
    let mut buf = vec![9u8; 5000];
    assert_eq!(f.read_at(0, &mut buf), Ok(5000));
    assert!(buf[..100].iter().all(|&b| b == 1));
    assert!(buf[100..].iter().all(|&b| b == 0));
    assert_eq!(f.truncate(MAX_FILE_SIZE + 1), Err(Errno::EFBIG));
    // truncate(2) by path: directories are EISDIR.
    env.mkdir("/d");
    assert_eq!(env.vfs.truncate(&env.ctx, &env.root, At::Cwd, b"/d", 0), Err(Errno::EISDIR));
    env.vfs.truncate(&env.ctx, &env.root, At::Cwd, b"/t", 1).unwrap();
    assert_eq!(env.stat("/t").unwrap().size, 1);
    // Writes past the largest offset: EFBIG.
    assert_eq!(f.write_at(MAX_FILE_SIZE, b"x"), Err(Errno::EFBIG));
}

#[test]
fn readdir_resumes_and_survives_changes() {
    let env = Env::new();
    env.mkdir("/d");
    for i in 0..200 {
        env.write(&alloc::format!("/d/f{i:03}"), b"");
    }
    let d = env.open("/d", OpenFlags::RDONLY).unwrap();
    // Read in small chunks: an emitter that takes 7 entries per call.
    let mut seen: Vec<String> = Vec::new();
    loop {
        let mut taken = 0;
        let n = d
            .read_dir(&mut |e| {
                if taken == 7 {
                    return false;
                }
                taken += 1;
                seen.push(String::from_utf8(e.name.to_vec()).unwrap());
                true
            })
            .unwrap();
        if n == 0 {
            break;
        }
        // Remove an entry already listed and one not yet listed partway.
        if seen.len() == 49 {
            env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/f010").unwrap();
            env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/f150").unwrap();
        }
    }
    assert_eq!(seen[0], ".");
    assert_eq!(seen[1], "..");
    assert_eq!(seen.len(), 2 + 199, "every entry once, the removed unlisted one never");
    assert!(seen.contains(&String::from("f010")));
    assert!(!seen.contains(&String::from("f150")));
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), seen.len(), "no duplicates");
    // Seeking back to the start lists again.
    assert_eq!(d.seek(0, Whence::Set), Ok(0));
    let mut again = 0;
    d.read_dir(&mut |_| {
        again += 1;
        true
    })
    .unwrap();
    assert_eq!(again, 2 + 198);
    assert_eq!(d.seek(0, Whence::End), Err(Errno::EINVAL));
    // Listing a file: ENOTDIR.
    let f = env.open("/d/f000", OpenFlags::RDONLY).unwrap();
    assert_eq!(f.read_dir(&mut |_| true), Err(Errno::ENOTDIR));
}

#[test]
fn readdir_reports_types_and_inodes() {
    let env = Env::new();
    env.mkdir("/d");
    env.mkdir("/d/sub");
    env.write("/d/file", b"");
    env.symlink("file", "/d/link");
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/d/fifo", FileType::Fifo, 0o644, DeviceId::default()).unwrap();
    let d = env.open("/d", OpenFlags::RDONLY).unwrap();
    let mut entries = Vec::new();
    d.read_dir(&mut |e| {
        entries.push((String::from_utf8(e.name.to_vec()).unwrap(), e.kind, e.ino));
        true
    })
    .unwrap();
    let get = |n: &str| entries.iter().find(|e| e.0 == n).cloned().unwrap();
    assert_eq!(get(".").2, env.ino("/d").unwrap());
    assert_eq!(get("..").2, env.ino("/").unwrap());
    assert_eq!(get("sub").1, FileType::Directory);
    assert_eq!(get("file").1, FileType::Regular);
    assert_eq!(get("link").1, FileType::Symlink);
    assert_eq!(get("fifo").1, FileType::Fifo);
    assert_eq!(FileType::Regular.dirent_type(), 8);
    assert_eq!(FileType::Directory.dirent_type(), 4);
    assert_eq!(FileType::Symlink.dirent_type(), 10);
    // A FIFO cannot be opened before pipes exist.
    assert_eq!(env.open("/d/fifo", OpenFlags::RDONLY).map(|_| ()), Err(Errno::ENXIO));
}

#[test]
fn timestamps_follow_changes() {
    let env = Env::new();
    env.mkdir("/d");
    let d0 = env.stat("/d").unwrap();
    env.write("/d/f", b"x");
    let d1 = env.stat("/d").unwrap();
    assert!(d1.mtime > d0.mtime && d1.ctime > d0.ctime);
    let f0 = env.stat("/d/f").unwrap();
    let f = env.open("/d/f", OpenFlags::WRONLY).unwrap();
    f.write(b"y").unwrap();
    let f1 = env.stat("/d/f").unwrap();
    assert!(f1.mtime > f0.mtime);
    let set = Timespec::new(42, 7);
    let node = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/d/f", true).unwrap();
    node.set_times(&env.root, Some(SetTime::At(set)), None).unwrap();
    let f2 = env.stat("/d/f").unwrap();
    assert_eq!(f2.atime, set);
    assert_eq!(f2.mtime, f1.mtime);
    assert!(f2.ctime > f1.ctime);
}

#[test]
fn shared_offsets_between_dups() {
    let env = Env::new();
    env.write("/f", b"abcdef");
    let f = env.open("/f", OpenFlags::RDONLY).unwrap();
    let mut table = FdTable::new();
    let a = table.install(f, false).unwrap();
    let b = table.dup(a).unwrap();
    let mut buf = [0u8; 2];
    table.get(a).unwrap().read(&mut buf).unwrap();
    table.get(b).unwrap().read(&mut buf).unwrap();
    assert_eq!(&buf, b"cd");
    // A separate open has its own offset.
    let other = env.open("/f", OpenFlags::RDONLY).unwrap();
    other.read(&mut buf).unwrap();
    assert_eq!(&buf, b"ab");
    assert!(Arc::ptr_eq(table.get(a).unwrap(), table.get(b).unwrap()));
}
