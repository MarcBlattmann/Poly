//! The file-system behaviour the second batch of Linux spec sheets pins
//! down (specs/linux/x86_64: unlink, rmdir, link, chown, fallocate,
//! pwritev2, fchmod): the order of unlink's and rmdir's errors, the
//! `fs.protected_hardlinks` rule, chown of another user's set-user-ID file,
//! tmpfs fallocate, writes with `RWF_APPEND` / `RWF_NOAPPEND`, and
//! anonymous inodes without a file type.

use super::{Env, TestClock, read_all};
use crate::*;
use alloc::sync::Arc;

const ALICE: u32 = 1000;
const BOB: u32 = 2000;

fn alice() -> Cred {
    Cred::user(ALICE, ALICE)
}

impl Env {
    fn chmod_b(&self, path: &str, perm: u16) {
        self.vfs.chmod(&self.ctx, &self.root, At::Cwd, path.as_bytes(), perm, true).unwrap();
    }

    fn chown_b(&self, path: &str, uid: u32, gid: u32) {
        self.vfs.chown(&self.ctx, &self.root, At::Cwd, path.as_bytes(), Some(uid), Some(gid), true).unwrap();
    }

    fn unlink_as(&self, cred: &Cred, path: &str) -> Result<()> {
        self.vfs.unlink(&self.ctx, cred, At::Cwd, path.as_bytes())
    }

    fn rmdir_as(&self, cred: &Cred, path: &str) -> Result<()> {
        self.vfs.rmdir(&self.ctx, cred, At::Cwd, path.as_bytes())
    }

    fn link_as(&self, cred: &Cred, old: &str, new: &str) -> Result<()> {
        self.vfs.link(&self.ctx, cred, At::Cwd, old.as_bytes(), At::Cwd, new.as_bytes(), false)
    }

    /// A tmpfs mounted at `path` (made first), read-only if asked.
    fn mount_tmpfs(&self, path: &str, read_only: bool) {
        self.mkdir(path);
        let fs = Arc::new(Tmpfs::new(Arc::new(TestClock::default())));
        self.vfs.mount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), fs, MountFlags::default()).unwrap();
        if read_only {
            let flags = MountFlags { read_only: true, ..MountFlags::default() };
            self.vfs.remount(&self.ctx, &self.root, At::Cwd, path.as_bytes(), flags).unwrap();
        }
    }
}

#[test]
fn unlink_checks_the_mount_and_the_name_before_write_permission() {
    let env = Env::new();
    env.mkdir("/ro");
    env.write("/ro/mine", b"x");
    env.mkdir("/ro/subdir");
    env.chown_b("/ro/mine", ALICE, ALICE);
    env.chmod_b("/ro", 0o755);
    let a = alice();
    // Missing beats the parent's write permission; a trailing slash after
    // a file is ENOTDIR; a directory in a parent the user may not write is
    // EACCES, not EISDIR; a last "." is EISDIR before everything.
    assert_eq!(env.unlink_as(&a, "/ro/nope"), Err(Errno::ENOENT));
    assert_eq!(env.unlink_as(&a, "/ro/mine/"), Err(Errno::ENOTDIR));
    assert_eq!(env.unlink_as(&a, "/ro/subdir"), Err(Errno::EACCES));
    assert_eq!(env.unlink_as(&a, "/ro/subdir/."), Err(Errno::EISDIR));
    assert_eq!(env.unlink_as(&a, "/ro/mine"), Err(Errno::EACCES));
    assert_eq!(env.unlink_as(&env.root, "/ro/subdir"), Err(Errno::EISDIR));
    // On a read-only mount: EROFS, also for a missing name; "." still EISDIR.
    env.mount_tmpfs("/m", true);
    assert_eq!(env.unlink_as(&env.root, "/m/nope"), Err(Errno::EROFS));
    assert_eq!(env.unlink_as(&env.root, "/m/."), Err(Errno::EISDIR));
}

#[test]
fn rmdir_checks_the_mount_before_the_name() {
    let env = Env::new();
    env.mount_tmpfs("/m", true);
    assert_eq!(env.rmdir_as(&env.root, "/m/nope"), Err(Errno::EROFS));
    assert_eq!(env.rmdir_as(&env.root, "/m/."), Err(Errno::EINVAL));
    // Write permission on the parent comes before "not a directory".
    env.mkdir("/ro");
    env.write("/ro/f", b"x");
    assert_eq!(env.rmdir_as(&alice(), "/ro/f"), Err(Errno::EACCES));
    assert_eq!(env.rmdir_as(&alice(), "/ro/nope"), Err(Errno::ENOENT));
    assert_eq!(env.rmdir_as(&env.root, "/ro/f"), Err(Errno::ENOTDIR));
}

#[test]
fn protected_hardlinks_need_ownership_or_read_and_write() {
    let env = Env::new();
    env.mkdir("/w");
    env.chmod_b("/w", 0o777);
    for (name, perm) in [("rw", 0o666), ("ro", 0o644), ("suid", 0o4666), ("sgidx", 0o2676), ("sgid", 0o2666)] {
        let path = alloc::format!("/{name}");
        env.write(&path, b"x");
        env.chown_b(&path, BOB, BOB);
        env.chmod_b(&path, perm);
    }
    env.write("/mine", b"x");
    env.chown_b("/mine", ALICE, ALICE);
    env.chmod_b("/mine", 0);
    let a = alice();
    assert_eq!(env.link_as(&a, "/mine", "/w/mine"), Ok(()));
    assert_eq!(env.link_as(&a, "/rw", "/w/rw"), Ok(()));
    assert_eq!(env.link_as(&a, "/sgid", "/w/sgid"), Ok(()));
    assert_eq!(env.link_as(&a, "/ro", "/w/ro"), Err(Errno::EPERM));
    assert_eq!(env.link_as(&a, "/suid", "/w/suid"), Err(Errno::EPERM));
    assert_eq!(env.link_as(&a, "/sgidx", "/w/sgidx"), Err(Errno::EPERM));
    // The new parent's write permission comes first.
    assert_eq!(env.link_as(&a, "/ro", "/x"), Err(Errno::EACCES));
    // Root (CAP_FOWNER) may link anything but a directory.
    assert_eq!(env.link_as(&env.root, "/ro", "/w/ro"), Ok(()));
}

#[test]
fn chown_minus_one_on_another_users_setuid_file_is_eperm() {
    let env = Env::new();
    env.write("/f", b"x");
    env.chown_b("/f", BOB, BOB);
    env.chmod_b("/f", 0o4755);
    let chown = |cred: &Cred| env.vfs.chown(&env.ctx, cred, At::Cwd, b"/f", None, None, true);
    assert_eq!(chown(&alice()), Err(Errno::EPERM));
    assert_eq!(env.stat("/f").unwrap().mode & 0o7777, 0o4755);
    // Without set-id bits it is a no-op anyone may do.
    env.chmod_b("/f", 0o755);
    assert_eq!(chown(&alice()), Ok(()));
    // Root clears set-user-ID; a directory keeps its bits.
    env.chmod_b("/f", 0o6755);
    assert_eq!(chown(&env.root), Ok(()));
    assert_eq!(env.stat("/f").unwrap().mode & 0o7777, 0o755);
    env.mkdir("/d");
    env.chmod_b("/d", 0o7755);
    assert_eq!(env.vfs.chown(&env.ctx, &env.root, At::Cwd, b"/d", None, None, true), Ok(()));
    assert_eq!(env.stat("/d").unwrap().mode & 0o7777, 0o7755);
}

#[test]
fn tmpfs_fallocate_allocates_keeps_the_size_and_punches_holes() {
    let env = Env::new();
    env.write("/f", b"hello");
    let f = env.open("/f", OpenFlags::RDWR).unwrap();
    // Mode 0 grows the size; the new bytes read as zeros; blocks grow.
    assert_eq!(f.fallocate(0, 0, 16384), Ok(()));
    let st = f.stat().unwrap();
    assert_eq!((st.size, st.blocks), (16384, 32));
    let data = read_all(&f).unwrap();
    assert_eq!(&data[..5], b"hello");
    assert!(data[5..].iter().all(|&b| b == 0));
    // KEEP_SIZE: blocks, no size.
    assert_eq!(f.fallocate(FALLOC_FL_KEEP_SIZE, 16384, 8192), Ok(()));
    let st = f.stat().unwrap();
    assert_eq!((st.size, st.blocks), (16384, 48));
    // PUNCH_HOLE | KEEP_SIZE: whole pages freed, edges zeroed.
    assert_eq!(f.write_at(0, &[b'x'; 16384]), Ok(16384));
    assert_eq!(f.fallocate(FALLOC_FL_KEEP_SIZE | FALLOC_FL_PUNCH_HOLE, 4096, 8192), Ok(()));
    let st = f.stat().unwrap();
    assert_eq!((st.size, st.blocks), (16384, 32));
    assert_eq!(f.fallocate(FALLOC_FL_KEEP_SIZE | FALLOC_FL_PUNCH_HOLE, 1, 2), Ok(()));
    let mut back = [0u8; 16384];
    assert_eq!(f.read_at(0, &mut back), Ok(16384));
    assert_eq!(&back[..4], b"x\0\0x");
    assert!(back[4096..12288].iter().all(|&b| b == 0));
    assert!(back[12288..].iter().all(|&b| b == b'x'));
    // Other modes, a range past the largest size.
    assert_eq!(f.fallocate(0x10, 0, 10), Err(Errno::EOPNOTSUPP));
    assert_eq!(f.fallocate(0, MAX_FILE_SIZE - 10, 4096), Err(Errno::EFBIG));
    // Not open for writing; not a regular file.
    let ro = env.open("/f", OpenFlags::RDONLY).unwrap();
    assert_eq!(ro.fallocate(0, 0, 10), Err(Errno::EBADF));
}

#[test]
fn tmpfs_fallocate_beyond_the_size_limit_is_enospc() {
    let env = Env::new();
    env.mkdir("/small");
    let fs = Arc::new(Tmpfs::with_limits(Arc::new(TestClock::default()), 64 * 1024, 100));
    env.vfs.mount(&env.ctx, &env.root, At::Cwd, b"/small", fs, MountFlags::default()).unwrap();
    env.write("/small/f", b"x");
    let f = env.open("/small/f", OpenFlags::RDWR).unwrap();
    assert_eq!(f.fallocate(0, 0, 1 << 20), Err(Errno::ENOSPC));
    assert_eq!(f.stat().unwrap().size, 1);
    assert_eq!(f.fallocate(0, 0, 32 * 1024), Ok(()));
}

#[test]
fn write_with_appends_or_not_for_one_call() {
    let env = Env::new();
    env.write("/f", b"data");
    let f = env.open("/f", OpenFlags::RDWR).unwrap();
    // RWF_APPEND at an explicit offset: the end, the offset stays.
    assert_eq!(f.write_with(Some(0), Some(true), b"AB"), Ok(2));
    assert_eq!(env.read("/f").unwrap(), b"dataAB");
    assert_eq!(f.seek(0, Whence::Cur), Ok(0));
    // At the file offset: it moves to the new end; the file is not O_APPEND.
    assert_eq!(f.write_with(None, Some(true), b"CD"), Ok(2));
    assert_eq!(f.seek(0, Whence::Cur), Ok(8));
    assert!(!f.flags().contains(OpenFlags::APPEND));
    // RWF_NOAPPEND on an O_APPEND file: at the offset given.
    let ap = env.open("/f", OpenFlags::WRONLY | OpenFlags::APPEND).unwrap();
    assert_eq!(ap.write_with(Some(4), Some(false), b"XY"), Ok(2));
    assert_eq!(env.read("/f").unwrap(), b"dataXYCD");
    assert_eq!(ap.write_with(None, Some(false), b"zz"), Ok(2));
    assert_eq!(env.read("/f").unwrap(), b"zztaXYCD");
    assert_eq!(ap.seek(0, Whence::Cur), Ok(2));
    assert!(ap.flags().contains(OpenFlags::APPEND));
    // Without either, the file's own mode.
    assert_eq!(ap.write_with(Some(0), None, b"!"), Ok(1));
    assert_eq!(env.read("/f").unwrap(), b"zztaXYCD!");
}

#[test]
fn anonymous_inodes_have_no_type_and_a_fixed_mode() {
    let env = Env::new();
    let node = env.vfs.anon_inode(0o600, &env.root).unwrap();
    assert_eq!(node.stat().unwrap().mode, 0o600);
    assert_eq!(node.chmod(&env.root, 0o644), Err(Errno::EOPNOTSUPP));
    assert_eq!(node.stat().unwrap().mode, 0o600);
    // Typed ones (a pipe's) take a new mode.
    let pipe = env.vfs.anon_node(FileType::Fifo, 0o600, &env.root).unwrap();
    assert_eq!(pipe.chmod(&env.root, 0o640), Ok(()));
    assert_eq!(pipe.stat().unwrap().mode, S_IFIFO | 0o640);
}

#[test]
fn anonymous_inodes_refuse_chown_even_minus_one() {
    let env = Env::new();
    let node = env.vfs.anon_inode(0o600, &env.root).unwrap();
    assert_eq!(node.chown(&env.root, None, None), Err(Errno::EOPNOTSUPP));
    assert_eq!(node.chown(&env.root, Some(5), Some(6)), Err(Errno::EOPNOTSUPP));
    let st = node.stat().unwrap();
    assert_eq!((st.uid, st.gid), (0, 0));
}

#[test]
fn a_running_program_cannot_be_written_or_truncated() {
    let env = Env::new();
    env.write("/prog", b"\x7fELF");
    env.chmod_b("/prog", 0o755);
    let exe = env.vfs.open_exec(&env.ctx, &env.root, At::Cwd, b"/prog").unwrap();
    assert_eq!(env.open("/prog", OpenFlags::WRONLY).map(|_| ()), Err(Errno::ETXTBSY));
    assert_eq!(env.open("/prog", OpenFlags::RDWR).map(|_| ()), Err(Errno::ETXTBSY));
    assert!(env.open("/prog", OpenFlags::RDONLY).is_ok());
    assert_eq!(env.vfs.truncate(&env.ctx, &env.root, At::Cwd, b"/prog", 0), Err(Errno::ETXTBSY));
    // Write permission comes first: a user who may not write gets EACCES.
    assert_eq!(env.vfs.truncate(&env.ctx, &alice(), At::Cwd, b"/prog", 0), Err(Errno::EACCES));
    drop(exe);
    assert!(env.open("/prog", OpenFlags::WRONLY).is_ok());
    assert_eq!(env.vfs.truncate(&env.ctx, &env.root, At::Cwd, b"/prog", 0), Ok(()));
}

#[test]
fn a_file_open_for_writing_cannot_be_run() {
    let env = Env::new();
    env.write("/prog", b"#!/bin/sh\n");
    env.chmod_b("/prog", 0o755);
    let writer = env.open("/prog", OpenFlags::WRONLY).unwrap();
    let run = |env: &Env| env.vfs.open_exec(&env.ctx, &env.root, At::Cwd, b"/prog").map(|_| ());
    assert_eq!(run(&env), Err(Errno::ETXTBSY));
    drop(writer);
    assert_eq!(run(&env), Ok(()));
    // execveat(2): AT_SYMLINK_NOFOLLOW on a link is ELOOP; the file a
    // descriptor is open on runs as a program by itself.
    env.vfs.symlink(&env.ctx, &env.root, b"prog", At::Cwd, b"/link").unwrap();
    let nofollow = env.vfs.open_exec_with(&env.ctx, &env.root, At::Cwd, b"/link", false);
    assert_eq!(nofollow.map(|_| ()), Err(Errno::ELOOP));
    assert!(env.vfs.open_exec_with(&env.ctx, &env.root, At::Cwd, b"/link", true).is_ok());
    let file = env.open("/prog", OpenFlags::RDONLY).unwrap();
    assert!(env.vfs.open_exec_file(&env.root, &file).is_ok());
    let dir = env.open("/", OpenFlags::RDONLY).unwrap();
    assert_eq!(env.vfs.open_exec_file(&env.root, &dir).map(|_| ()), Err(Errno::EACCES));
}

#[test]
fn rename_checks_exdev_busy_and_erofs_before_the_names() {
    let env = Env::new();
    env.mkdir("/d");
    env.mkdir("/d/sub");
    env.mount_tmpfs("/mnt", false);
    env.mount_tmpfs("/ro", true);
    let rename_as = |cred: &Cred, old: &str, new: &str| {
        env.vfs.rename(&env.ctx, cred, At::Cwd, old.as_bytes(), At::Cwd, new.as_bytes(), RenameFlags::default())
    };
    let rename = |old: &str, new: &str| rename_as(&env.root, old, new);
    // EXDEV before EBUSY for "." and before a missing source.
    assert_eq!(rename("/mnt/.", "/d/x"), Err(Errno::EXDEV));
    assert_eq!(rename("/d/missing", "/mnt/x"), Err(Errno::EXDEV));
    assert_eq!(rename("/d/missing", "/d/sub/."), Err(Errno::EBUSY));
    // EROFS before the last name.
    assert_eq!(rename("/ro/missing", "/ro/x"), Err(Errno::EROFS));
    // A directory over its own ancestor: ENOTEMPTY.
    assert_eq!(rename("/d/sub", "/d"), Err(Errno::ENOTEMPTY));
    // Write permission on the parent before the kinds.
    env.write("/d/f", b"x");
    assert_eq!(rename_as(&alice(), "/d/f", "/d/sub"), Err(Errno::EACCES));
    assert_eq!(rename("/d/f", "/d/sub"), Err(Errno::EISDIR));
}

#[test]
fn optional_limits_bound_the_data() {
    let fs = Tmpfs::with_optional_limits(Arc::new(TestClock::default()), Some(64 * 1024), None);
    assert_eq!(fs.statfs().unwrap().blocks, 16);
}
