//! What stat(2) reports and which lookups fail, following the cases of
//! the stat spec sheet (`specs/linux/x86_64/stat.yaml`): link limits
//! across a path, removed working directories, sizes and blocks, mode
//! bits, link counts, search permission and the order of path errors.

use super::Env;
use crate::*;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFIFO: u32 = 0o010000;

fn user() -> Cred {
    Cred::user(1001, 1001)
}

/// A directory `/d` owned by the user.
fn env() -> Env {
    let env = Env::new();
    env.mkdir("/d");
    env.vfs.chown(&env.ctx, &env.root, At::Cwd, b"/d", Some(1001), Some(1001), true).unwrap();
    env
}

fn ustat(env: &Env, path: &str) -> Result<Stat> {
    env.vfs.stat(&env.ctx, &user(), At::Cwd, path.as_bytes(), true)
}

#[test]
fn forty_links_per_lookup_across_the_path() {
    let env = env();
    env.write("/d/f", b"x");
    env.mkdir("/d/dir");
    // l0 -> f, lN -> l(N-1): following lN takes N+1 links.
    env.symlink("f", "/d/l0");
    for i in 1..=45 {
        env.symlink(&format!("l{}", i - 1), &format!("/d/l{i}"));
    }
    assert_eq!(env.stat("/d/l39").map(|s| s.mode & S_IFMT), Ok(S_IFREG), "40 links");
    assert_eq!(env.stat("/d/l40"), Err(Errno::ELOOP), "41 links");
    assert_eq!(env.stat("/d/l45"), Err(Errno::ELOOP));
    assert_eq!(env.stat("/d/l39/x"), Err(Errno::ENOTDIR));
    // m0 -> dir, mN -> m(N-1): m19 is 20 links to a directory.
    env.symlink("dir", "/d/m0");
    for i in 1..20 {
        env.symlink(&format!("m{}", i - 1), &format!("/d/m{i}"));
    }
    assert_eq!(env.stat("/d/m19").map(|s| s.mode & S_IFMT), Ok(S_IFDIR));
    assert!(env.stat("/d/m19/../l19").is_ok(), "20 + 20 links");
    assert_eq!(env.stat("/d/m19/../l20"), Err(Errno::ELOOP), "20 + 21 links");
    assert!(env.stat("/d/m19/../l0").is_ok());
}

#[test]
fn symlinks_are_followed_everywhere() {
    let env = env();
    env.mkdir("/d/sub");
    env.write("/d/sub/g", b"abc");
    env.write("/d/f", b"0123456789");
    let f = env.stat("/d/f").unwrap();
    env.symlink("sub", "/d/l_dir");
    env.symlink("/d/missing", "/d/l_dangling");
    env.symlink("/d/f", "/d/l_file");
    env.symlink("l_self", "/d/l_self");
    env.symlink("l_b", "/d/l_a");
    env.symlink("l_a", "/d/l_b");
    assert_eq!(env.stat("/d/l_dir/").map(|s| s.mode & S_IFMT), Ok(S_IFDIR));
    assert_eq!(env.stat("/d/l_dir/g").map(|s| s.size), Ok(3));
    assert_eq!(env.ino("/d/l_dir/../f"), Ok(f.ino), "physical parent");
    assert_eq!(env.stat("/d/l_dangling"), Err(Errno::ENOENT));
    assert_eq!(env.stat("/d/l_dangling/"), Err(Errno::ENOENT));
    assert_eq!(env.stat("/d/l_file/"), Err(Errno::ENOTDIR));
    assert_eq!(env.stat("/d/l_file/x"), Err(Errno::ENOTDIR));
    assert_eq!(env.stat("/d/l_self"), Err(Errno::ELOOP));
    assert_eq!(env.stat("/d/l_self/x"), Err(Errno::ELOOP));
    assert_eq!(env.stat("/d/l_a"), Err(Errno::ELOOP));
    assert!(env.lstat("/d/l_a").is_ok());
    let l = env.lstat("/d/l_file").unwrap();
    assert_eq!((l.mode, l.size), (0o120777, 4), "the target's length");
}

#[test]
fn a_removed_working_directory() {
    let mut env = env();
    env.mkdir("/d/gone");
    let d = env.ino("/d").unwrap();
    env.cd("/d/gone").unwrap();
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/d/gone").unwrap();
    let dot = env.stat(".").unwrap();
    assert_eq!((dot.mode & S_IFMT, dot.nlink), (S_IFDIR, 0));
    assert_eq!(env.stat("x"), Err(Errno::ENOENT));
    assert_eq!(env.ino(".."), Ok(d));
}

#[test]
fn directory_link_counts_and_spellings() {
    let env = env();
    let d = env.stat("/d").unwrap();
    assert_eq!(d.nlink, 2);
    assert!(d.size > 0);
    env.mkdir("/d/sub");
    assert_eq!(env.stat("/d").map(|s| s.nlink), Ok(3));
    assert_eq!(env.ino("/d/sub/.."), Ok(d.ino));
    for p in ["/d/", "/d///", "/d/.", "//d//"] {
        assert_eq!(env.ino(p), Ok(d.ino), "{p}");
    }
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/d/sub").unwrap();
    assert_eq!(env.stat("/d").map(|s| s.nlink), Ok(2));
}

#[test]
fn sizes_and_blocks() {
    let env = env();
    env.write("/d/f", b"");
    let f = env.open("/d/f", OpenFlags::WRONLY).unwrap();
    assert_eq!(env.stat("/d/f").map(|s| s.blocks), Ok(0));
    f.truncate(1 << 20).unwrap();
    let s = env.stat("/d/f").unwrap();
    assert_eq!(s.size, 1 << 20);
    assert!(s.blocks < (1 << 20) / 512);
    assert_eq!(f.write_at(0, &[b'x'; 4096]), Ok(4096));
    let s = env.stat("/d/f").unwrap();
    assert_eq!(s.size, 1 << 20);
    assert!(s.blocks >= 8);
    assert!(s.blksize > 0);
    f.truncate(5 << 30).unwrap();
    let s = env.stat("/d/f").unwrap();
    assert_eq!(s.size, 5 << 30, "past 4 GiB in full");
    assert!(s.blocks < (5 << 30) / 512);
    f.truncate(0).unwrap();
    assert_eq!(env.stat("/d/f").map(|s| (s.size, s.blocks)), Ok((0, 0)));
}

#[test]
fn mode_bits_and_kinds() {
    let env = env();
    let u = user();
    env.vfs.mkdir(&env.ctx, &u, At::Cwd, b"/d/x", 0o755).unwrap();
    for perm in [0o7777, 0o7000, 0] {
        env.vfs.chmod(&env.ctx, &u, At::Cwd, b"/d/x", perm, true).unwrap();
        assert_eq!(ustat(&env, "/d/x").map(|s| s.mode), Ok(S_IFDIR | u32::from(perm)), "{perm:o}");
    }
    // A directory made inside a set-group-ID directory inherits the bit.
    env.vfs.chmod(&env.ctx, &u, At::Cwd, b"/d/x", 0o2755, true).unwrap();
    env.vfs.mkdir(&env.ctx, &u, At::Cwd, b"/d/x/y", 0o755).unwrap();
    assert_eq!(ustat(&env, "/d/x/y").map(|s| s.mode & 0o7777), Ok(0o2755));
    env.vfs.mknod(&env.ctx, &u, At::Cwd, b"/d/fifo", FileType::Fifo, 0o640, DeviceId::default()).unwrap();
    let s = ustat(&env, "/d/fifo").unwrap();
    assert_eq!((s.mode, s.size, s.rdev, s.nlink, s.uid), (S_IFIFO | 0o640, 0, DeviceId::default(), 1, 1001));
}

#[test]
fn hard_links_and_renames() {
    let env = env();
    env.write("/d/a", b"aaaa");
    let a = env.stat("/d/a").unwrap();
    env.vfs.link(&env.ctx, &env.root, At::Cwd, b"/d/a", At::Cwd, b"/d/b", false).unwrap();
    assert_eq!(env.stat("/d/b").map(|s| (s.ino, s.nlink)), Ok((a.ino, 2)));
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/d/b", At::Cwd, b"/d/c", RenameFlags::default()).unwrap();
    assert_eq!(env.stat("/d/b"), Err(Errno::ENOENT));
    assert_eq!(env.stat("/d/c").map(|s| (s.ino, s.nlink)), Ok((a.ino, 2)));
    env.write("/d/r", b"rr");
    let r = env.ino("/d/r").unwrap();
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/d/r", At::Cwd, b"/d/c", RenameFlags::default()).unwrap();
    assert_eq!(env.stat("/d/c").map(|s| (s.ino, s.size)), Ok((r, 2)));
    assert_eq!(env.stat("/d/a").map(|s| s.nlink), Ok(1));
    assert_eq!(env.stat("/d/A"), Err(Errno::ENOENT), "case matters");
    let odd = String::from("/d/sp ace");
    env.write(&odd, b"");
    assert!(env.stat(&odd).is_ok());
    let bytes = b"/d/\x01\xff";
    env.vfs.open(&env.ctx, &env.root, At::Cwd, bytes, OpenFlags::WRONLY | OpenFlags::CREAT, 0o644).unwrap();
    assert!(env.vfs.stat(&env.ctx, &env.root, At::Cwd, bytes, true).is_ok());
    assert_eq!(env.vfs.stat(&env.ctx, &env.root, At::Cwd, b"/d/\x01", true), Err(Errno::ENOENT));
}

#[test]
fn search_permission_on_the_way() {
    let env = env();
    let u = user();
    env.vfs.mkdir(&env.ctx, &u, At::Cwd, b"/d/x", 0o755).unwrap();
    env.vfs.open(&env.ctx, &u, At::Cwd, b"/d/x/f", OpenFlags::WRONLY | OpenFlags::CREAT, 0).unwrap();
    // Mode 0 files can be stat'ed; a 0111 directory can be searched.
    env.vfs.chmod(&env.ctx, &u, At::Cwd, b"/d/x", 0o111, true).unwrap();
    assert_eq!(ustat(&env, "/d/x/f").map(|s| s.mode), Ok(S_IFREG));
    for perm in [0o666, 0] {
        env.vfs.chmod(&env.ctx, &u, At::Cwd, b"/d/x", perm, true).unwrap();
        for p in ["/d/x/f", "/d/x/nope", "/d/x/.", "/d/x/..", "/d/x/f/g"] {
            assert_eq!(ustat(&env, p), Err(Errno::EACCES), "{p} with {perm:o}");
        }
        for p in ["/d/x", "/d/x/"] {
            assert_eq!(ustat(&env, p).map(|s| s.mode), Ok(S_IFDIR | u32::from(perm)), "{p}");
        }
        assert!(env.stat("/d/x/f").is_ok(), "root searches anything");
    }
}

#[test]
fn path_errors_in_walk_order() {
    let env = env();
    env.write("/d/f", b"x");
    let name = |n: usize| String::from_utf8(alloc::vec![b'n'; n]).unwrap();
    env.write(&format!("/d/{}", name(255)), b"");
    assert!(env.stat(&format!("/d/{}", name(255))).is_ok());
    assert_eq!(env.stat(&format!("/d/{}", name(256))), Err(Errno::ENAMETOOLONG));
    assert_eq!(env.stat(&format!("/d/nope/{}", name(256))), Err(Errno::ENOENT));
    assert_eq!(env.stat(&format!("/d/f/{}", name(256))), Err(Errno::ENOTDIR));
    assert_eq!(env.stat(&format!("/d/{}/nope/x", name(256))), Err(Errno::ENAMETOOLONG));
    assert_eq!(env.stat("/d/nope/../f"), Err(Errno::ENOENT));
    assert_eq!(env.stat("/d/f/.."), Err(Errno::ENOTDIR));
    // 4095 bytes are fine whatever the components.
    let mut long: Vec<u8> = b"/d".to_vec();
    long.resize(4093, b'/');
    long.extend_from_slice(b"/f");
    assert_eq!(long.len(), 4095);
    assert_eq!(env.vfs.stat(&env.ctx, &env.root, At::Cwd, &long, true).map(|s| s.size), Ok(1));
}

#[test]
fn stat_moves_no_timestamp() {
    let env = env();
    env.write("/d/f", b"x");
    let before = (env.stat("/d/f").unwrap(), env.stat("/d").unwrap());
    let after = (env.stat("/d/f").unwrap(), env.stat("/d").unwrap());
    assert_eq!(before, after);
    assert_eq!(before.0.mtime, before.0.ctime, "a new file");
}
