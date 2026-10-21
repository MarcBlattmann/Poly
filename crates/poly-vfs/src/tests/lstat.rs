//! `lstat(2)`: a final symbolic link is reported itself, links earlier in
//! the path (or before a trailing slash or "/.") are followed. The cases
//! follow the spec sheet `linux/syscall/x86_64/lstat` and its visible tests.

use super::Env;
use crate::*;
use alloc::vec;

const USER: u32 = 1001;

fn user() -> Cred {
    Cred::user(USER, USER)
}

impl Env {
    fn lstat_as(&self, cred: &Cred, path: &str) -> Result<Stat> {
        self.vfs.stat(&self.ctx, cred, At::Cwd, path.as_bytes(), false)
    }

    fn hard_link(&self, old: &str, new: &str) -> Result<()> {
        self.vfs.link(&self.ctx, &self.root, At::Cwd, old.as_bytes(), At::Cwd, new.as_bytes(), false)
    }
}

fn kind(st: &Stat) -> u32 {
    st.mode & 0o170000
}

const S_IFLNK: u32 = 0o120000;
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const S_IFIFO: u32 = 0o010000;
const S_IFSOCK: u32 = 0o140000;

#[test]
fn dangling_and_looping_links_are_links() {
    let env = Env::new();
    env.mkdir("/d");
    env.symlink("missing-target", "/d/dang");
    env.symlink("b", "/d/a");
    env.symlink("a", "/d/b");
    env.symlink("self", "/d/self");
    let st = env.lstat("/d/dang").unwrap();
    assert_eq!(kind(&st), S_IFLNK);
    assert_eq!(st.mode & 0o7777, 0o777);
    assert_eq!(st.size, 14);
    assert_eq!(st.nlink, 1);
    assert_eq!(st.rdev, DeviceId::default());
    assert_eq!(env.stat("/d/dang"), Err(Errno::ENOENT));
    assert_eq!(env.lstat("/d/dang/"), Err(Errno::ENOENT));
    assert_eq!(env.lstat("/d/dang/x"), Err(Errno::ENOENT));
    assert_eq!(env.lstat("/d/a").map(|s| (kind(&s), s.size)), Ok((S_IFLNK, 1)));
    assert_eq!(env.lstat("/d/self").map(|s| s.size), Ok(4));
    assert_eq!(env.stat("/d/a"), Err(Errno::ELOOP));
    assert_eq!(env.lstat("/d/a/"), Err(Errno::ELOOP));
    assert_eq!(env.lstat("/d/a/x"), Err(Errno::ELOOP));
    assert_eq!(env.lstat("/d/self/x"), Err(Errno::ELOOP));
}

#[test]
fn links_before_the_last_name_and_trailing_slashes() {
    let env = Env::new();
    env.mkdir("/d");
    env.mkdir("/d/sub");
    env.mkdir("/d/sub/inner");
    env.write("/d/sub/inner/f", b"12345");
    env.symlink("sub/inner", "/d/rel");
    env.symlink("/d/sub/inner", "/d/abs");
    env.symlink("sub/inner/f", "/d/flink");
    env.symlink("rel", "/d/chain");
    let sub = env.lstat("/d/sub").unwrap().ino;
    let inner = env.lstat("/d/sub/inner").unwrap().ino;
    let file = env.lstat("/d/sub/inner/f").unwrap().ino;
    let rel = env.lstat("/d/rel").unwrap();
    assert_eq!((kind(&rel), rel.size), (S_IFLNK, 9));
    assert_eq!(env.lstat("/d/rel/f").map(|s| (s.ino, s.size)), Ok((file, 5)));
    assert_eq!(env.lstat("/d/abs/f").map(|s| s.ino), Ok(file));
    assert_eq!(env.lstat("/d/abs").map(|s| s.size), Ok(12));
    let st = env.lstat("/d/rel/").unwrap();
    assert_eq!((st.ino, kind(&st)), (inner, S_IFDIR));
    assert_eq!(env.lstat("/d/rel/.").map(|s| s.ino), Ok(inner));
    assert_eq!(env.lstat("/d/rel/..").map(|s| s.ino), Ok(sub));
    assert_eq!(env.lstat("/d/flink").map(|s| kind(&s)), Ok(S_IFLNK));
    assert_eq!(env.lstat("/d/flink/"), Err(Errno::ENOTDIR));
    let chain = env.lstat("/d/chain").unwrap();
    assert_eq!((kind(&chain), chain.size), (S_IFLNK, 3));
    assert_ne!(chain.ino, rel.ino);
    assert_eq!(env.lstat("/d/chain/").map(|s| s.ino), Ok(inner));
    assert_eq!(env.lstat("/d/chain/f").map(|s| s.ino), Ok(file));
    assert_eq!(env.lstat("/d//sub///inner//").map(|s| s.ino), Ok(inner));
    // "./l" and "sub/../l" still name the link itself.
    let mut env = env;
    env.cd("/d").unwrap();
    assert_eq!(env.lstat("./rel").map(|s| s.ino), Ok(rel.ino));
    assert_eq!(env.lstat("sub/../rel").map(|s| s.ino), Ok(rel.ino));
}

#[test]
fn path_errors_in_walk_order() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/f", b"x");
    env.symlink("loop", "/d/loop");
    let long = "n".repeat(256);
    assert_eq!(env.lstat("/d/nope"), Err(Errno::ENOENT));
    assert_eq!(env.lstat("/d/nope/x"), Err(Errno::ENOENT));
    assert_eq!(env.lstat(""), Err(Errno::ENOENT));
    assert_eq!(env.lstat("/d/f/x"), Err(Errno::ENOTDIR));
    assert_eq!(env.lstat("/d/f/"), Err(Errno::ENOTDIR));
    assert_eq!(env.lstat("/d/f/."), Err(Errno::ENOTDIR));
    assert_eq!(env.lstat(&alloc::format!("/d/{}", &long[..255])), Err(Errno::ENOENT));
    assert_eq!(env.lstat(&alloc::format!("/d/{long}")), Err(Errno::ENAMETOOLONG));
    assert_eq!(env.lstat(&alloc::format!("/d/nope/{long}")), Err(Errno::ENOENT));
    assert_eq!(env.lstat(&alloc::format!("/d/nope/../{long}")), Err(Errno::ENOENT));
    assert_eq!(env.lstat(&alloc::format!("/d/f/{long}")), Err(Errno::ENOTDIR));
    assert_eq!(env.lstat(&alloc::format!("/d/loop/{long}")), Err(Errno::ELOOP));
    assert_eq!(env.lstat(&alloc::format!("/d/{long}/nope")), Err(Errno::ENAMETOOLONG));
    // "/./././..." of 4095 bytes is the root.
    let mut p = vec![b'/'; 4095];
    for (i, c) in p.iter_mut().enumerate() {
        if i % 2 == 0 && i > 0 {
            *c = b'.';
        }
    }
    let st = env.vfs.stat(&env.ctx, &env.root, At::Cwd, &p, false).unwrap();
    assert_eq!(kind(&st), S_IFDIR);
}

#[test]
fn file_kinds_and_the_root() {
    let mut env = Env::new();
    env.ctx.umask = 0o022;
    env.vfs.mkdir(&env.ctx, &env.root, At::Cwd, b"/d", 0o750).unwrap();
    let st = env.lstat("/d").unwrap();
    assert_eq!((st.mode, st.nlink, st.rdev), (S_IFDIR | 0o750, 2, DeviceId::default()));
    assert_eq!(env.lstat("/d/").map(|s| s.mode), Ok(S_IFDIR | 0o750));
    let none = DeviceId::default();
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/d/fifo", FileType::Fifo, 0o640, none).unwrap();
    let st = env.lstat("/d/fifo").unwrap();
    assert_eq!((st.mode, st.size, st.nlink), (S_IFIFO | 0o640, 0, 1));
    assert_eq!(env.lstat("/d/fifo/"), Err(Errno::ENOTDIR));
    assert_eq!(env.lstat("/d/fifo/."), Err(Errno::ENOTDIR));
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/d/sock", FileType::Socket, 0o777, none).unwrap();
    let st = env.lstat("/d/sock").unwrap();
    assert_eq!((st.mode, st.size), (S_IFSOCK | 0o755, 0));

    let root = env.lstat("/").unwrap();
    assert_eq!(kind(&root), S_IFDIR);
    for p in ["/..", "/../../..", "//", "."] {
        let st = env.lstat(p).unwrap();
        assert_eq!((st.ino, st.dev), (root.ino, root.dev), "{p}");
    }
}

#[test]
fn hard_links_and_directory_link_counts() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/f", b"abc");
    let f = env.lstat("/d/f").unwrap();
    assert_eq!(f.nlink, 1);
    env.hard_link("/d/f", "/d/g").unwrap();
    assert_eq!(env.lstat("/d/f").map(|s| s.nlink), Ok(2));
    let g = env.lstat("/d/g").unwrap();
    assert_eq!((g.ino, g.dev, g.nlink), (f.ino, f.dev, 2));
    env.hard_link("/d/g", "/d/h").unwrap();
    assert_eq!(env.lstat("/d/f").map(|s| s.nlink), Ok(3));
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/f").unwrap();
    assert_eq!(env.lstat("/d/f"), Err(Errno::ENOENT));
    assert_eq!(env.lstat("/d/h").map(|s| (s.ino, s.nlink)), Ok((f.ino, 2)));

    env.symlink("h", "/d/sl");
    env.hard_link("/d/sl", "/d/sl2").unwrap();
    let (a, b) = (env.lstat("/d/sl").unwrap(), env.lstat("/d/sl2").unwrap());
    assert_eq!(kind(&b), S_IFLNK);
    assert_eq!((a.ino, a.nlink), (b.ino, 2));
    assert_eq!(env.lstat("/d/h").map(|s| s.nlink), Ok(2));

    assert_eq!(env.lstat("/d").map(|s| s.nlink), Ok(2));
    env.mkdir("/d/a");
    assert_eq!(env.lstat("/d").map(|s| s.nlink), Ok(3));
    env.mkdir("/d/b");
    assert_eq!(env.lstat("/d").map(|s| s.nlink), Ok(4));
    assert_eq!(env.lstat("/d/a").map(|s| s.nlink), Ok(2));
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/d/a").unwrap();
    assert_eq!(env.lstat("/d").map(|s| s.nlink), Ok(3));
    env.write("/d/x", b"");
    assert_eq!(env.lstat("/d").map(|s| s.nlink), Ok(3));
}

#[test]
fn search_permission_only_on_the_directories() {
    let env = Env::new();
    env.mkdir("/d");
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/d", 0o777, true).unwrap();
    let u = user();
    env.vfs.mkdir(&env.ctx, &u, At::Cwd, b"/d/sub", 0o755).unwrap();
    let f = env.vfs.open(&env.ctx, &u, At::Cwd, b"/d/f", OpenFlags::WRONLY | OpenFlags::CREAT, 0).unwrap();
    assert_eq!(f.write(b"secret"), Ok(6));
    let st = env.lstat_as(&u, "/d/f").unwrap();
    assert_eq!((st.mode, st.size), (S_IFREG, 6));
    env.vfs.open(&env.ctx, &u, At::Cwd, b"/d/sub/g", OpenFlags::WRONLY | OpenFlags::CREAT, 0o644).unwrap();
    env.vfs.symlink(&env.ctx, &u, b"g", At::Cwd, b"/d/sub/l").unwrap();
    let l = env.lstat_as(&u, "/d/sub/l").unwrap();
    assert_eq!((l.uid, l.gid), (USER, USER));

    let chmod = |perm| env.vfs.chmod(&env.ctx, &u, At::Cwd, b"/d/sub", perm, true).unwrap();
    chmod(0);
    assert_eq!(env.lstat_as(&u, "/d/sub").map(|s| s.mode), Ok(S_IFDIR));
    assert_eq!(env.lstat_as(&u, "/d/sub/g"), Err(Errno::EACCES));
    assert_eq!(env.lstat_as(&u, "/d/sub/l"), Err(Errno::EACCES));
    assert_eq!(env.lstat_as(&u, "/d/sub/nope"), Err(Errno::EACCES));
    assert!(env.lstat_as(&u, "/d/sub/").is_ok());
    assert!(env.lstat("/d/sub/g").is_ok());
    assert_eq!(env.lstat("/d/sub/nope"), Err(Errno::ENOENT));
    chmod(0o600);
    assert_eq!(env.lstat_as(&u, "/d/sub/g"), Err(Errno::EACCES));
    chmod(0o100);
    assert!(env.lstat_as(&u, "/d/sub/g").is_ok());
    assert_eq!(env.lstat_as(&u, "/d/sub/l").map(|s| kind(&s)), Ok(S_IFLNK));
}

#[test]
fn sizes_and_blocks() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/f", b"");
    let st = env.lstat("/d/f").unwrap();
    assert_eq!((st.size, st.blocks), (0, 0));
    assert!(st.blksize > 0);
    env.write("/d/f", &[b'p'; 4096]);
    let st = env.lstat("/d/f").unwrap();
    assert_eq!(st.size, 4096);
    assert!(st.blocks >= 8);
    env.vfs.truncate(&env.ctx, &env.root, At::Cwd, b"/d/f", 3).unwrap();
    assert_eq!(env.lstat("/d/f").map(|s| s.size), Ok(3));
    env.vfs.truncate(&env.ctx, &env.root, At::Cwd, b"/d/f", 1 << 20).unwrap();
    let st = env.lstat("/d/f").unwrap();
    assert_eq!(st.size, 1 << 20);
    assert!(st.blocks < 64);
    env.write("/d/s", b"");
    env.vfs.truncate(&env.ctx, &env.root, At::Cwd, b"/d/s", 10 << 20).unwrap();
    assert_eq!(env.lstat("/d/s").map(|s| (s.size, s.blocks)), Ok((10 << 20, 0)));
    env.symlink("s", "/d/l");
    assert_eq!(env.lstat("/d/l").map(|s| s.size), Ok(1));
}

#[test]
fn link_size_is_the_target_length() {
    let env = Env::new();
    env.mkdir("/d");
    for n in [1, 59, 60, 61, 255, 256, 1000, 4095] {
        let target: alloc::string::String = (0..n).map(|i| if i % 7 == 6 { '/' } else { 'x' }).collect();
        env.symlink(&target, "/d/l");
        assert_eq!(env.lstat("/d/l").map(|s| s.size), Ok(n as u64), "{n}");
        env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/l").unwrap();
    }
    let target = "x".repeat(4096);
    let r = env.vfs.symlink(&env.ctx, &env.root, target.as_bytes(), At::Cwd, b"/d/l");
    assert_eq!(r, Err(Errno::ENAMETOOLONG));
    env.symlink("\u{e9}t\u{e9}", "/d/u");
    assert_eq!(env.lstat("/d/u").map(|s| s.size), Ok(5));
    env.symlink("/", "/d/r");
    assert_eq!(env.lstat("/d/r").map(|s| s.size), Ok(1));
    assert_eq!(env.lstat("/d/r/").map(|s| kind(&s)), Ok(S_IFDIR));
}

#[test]
fn a_link_has_its_own_times_and_lstat_changes_none() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/f", b"data");
    env.symlink("f", "/d/l");
    let at = |s, n| Some(SetTime::At(Timespec::new(s, n)));
    let set = |p: &[u8], a, m, follow| env.vfs.utimens(&env.ctx, &env.root, At::Cwd, p, a, m, follow).unwrap();
    set(b"/d/l", at(1_000_000_000, 123_456_789), at(1_100_000_000, 987_654_321), false);
    set(b"/d/f", at(1_200_000_000, 5), at(1_300_000_000, 7), true);
    let l = env.lstat("/d/l").unwrap();
    assert_eq!(l.atime, Timespec::new(1_000_000_000, 123_456_789));
    assert_eq!(l.mtime, Timespec::new(1_100_000_000, 987_654_321));
    let f = env.lstat("/d/f").unwrap();
    assert_eq!(f.atime, Timespec::new(1_200_000_000, 5));
    assert_eq!(f.mtime, Timespec::new(1_300_000_000, 7));
    assert_eq!(env.stat("/d/l").map(|s| s.mtime), Ok(f.mtime));
    let d = env.lstat("/d").unwrap();
    for _ in 0..3 {
        assert_eq!(env.lstat("/d/l"), Ok(l));
        assert_eq!(env.lstat("/d/f"), Ok(f));
        assert_eq!(env.lstat("/d"), Ok(d));
    }
}

#[test]
fn removed_working_directory() {
    let mut env = Env::new();
    env.mkdir("/p");
    env.mkdir("/p/w");
    let p = env.lstat("/p").unwrap().ino;
    env.cd("/p/w").unwrap();
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/p/w").unwrap();
    let st = env.lstat(".").unwrap();
    assert_eq!((kind(&st), st.nlink), (S_IFDIR, 0));
    assert_eq!(env.lstat("..").map(|s| s.ino), Ok(p));
    assert_eq!(env.lstat("x"), Err(Errno::ENOENT));
}
