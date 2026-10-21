//! access(2) (spec `linux/syscall/x86_64/access`): the mode argument, path
//! errors and their order, symbolic links, the working directory and the
//! permission class, against a [`LinuxFs`](crate::fs::LinuxFs) over a
//! tmpfs.

extern crate std;

use super::fake::{UNMAPPED, sys};
use super::files::{Env, env, env_as, s};
use crate::{Errno, nr};
use poly_vfs::{At, Cred, DeviceId, FileType, OpenFlags};
use std::vec::Vec;

const F_OK: u64 = 0;
const X_OK: u64 = 1;
const W_OK: u64 = 2;
const R_OK: u64 = 4;

/// A normal user who owns nothing in the tree.
const USER: u32 = 1001;

fn access(e: &mut Env, path: &[u8], mode: u64) -> Result<u64, Errno> {
    let p = s(&mut e.k, 0, path);
    sys(&mut e.k, nr::ACCESS, &[p, mode])
}

/// Set-up as root: directories, files and links with exact modes.
fn mkdir(e: &Env, path: &[u8], perm: u16) {
    let (ctx, root) = (e.vfs.root_context().unwrap(), Cred::root());
    e.vfs.mkdir(&ctx, &root, At::Cwd, path, 0).unwrap();
    e.vfs.chmod(&ctx, &root, At::Cwd, path, perm, true).unwrap();
}

fn file(e: &Env, path: &[u8], perm: u16) {
    let (ctx, root) = (e.vfs.root_context().unwrap(), Cred::root());
    e.vfs.open(&ctx, &root, At::Cwd, path, OpenFlags::CREAT | OpenFlags::WRONLY, 0).unwrap();
    e.vfs.chmod(&ctx, &root, At::Cwd, path, perm, true).unwrap();
}

fn chown(e: &Env, path: &[u8], uid: u32, gid: u32) {
    let (ctx, root) = (e.vfs.root_context().unwrap(), Cred::root());
    e.vfs.chown(&ctx, &root, At::Cwd, path, Some(uid), Some(gid), true).unwrap();
}

fn symlink(e: &Env, target: &[u8], path: &[u8]) {
    let (ctx, root) = (e.vfs.root_context().unwrap(), Cred::root());
    e.vfs.symlink(&ctx, &root, target, At::Cwd, path).unwrap();
}

#[test]
fn mode_is_checked_first_and_only_its_low_32_bits_count() {
    let mut e = env();
    let f = b"/dir/file";
    for m in [8, 0x10, R_OK | 8, 0x200, 0x8000_0000, u64::MAX] {
        assert_eq!(access(&mut e, f, m), Err(Errno::EINVAL), "mode {m:#x}");
    }
    assert_eq!(access(&mut e, f, 0x1_0000_0000 | R_OK), Ok(0), "bits 32-63 ignored");
    assert_eq!(access(&mut e, f, 0x1_0000_0000), Ok(0));
    for m in 0..8 {
        assert_eq!(access(&mut e, b"/dir", m), Ok(0), "mode {m}");
    }
    for m in 8..64 {
        assert_eq!(access(&mut e, b"/dir", m), Err(Errno::EINVAL), "mode {m}");
    }
    // EINVAL before every path error.
    assert_eq!(access(&mut e, b"/dir/nope", 8), Err(Errno::EINVAL));
    assert_eq!(access(&mut e, b"", 0x10), Err(Errno::EINVAL));
    assert_eq!(sys(&mut e.k, nr::ACCESS, &[0, 8]), Err(Errno::EINVAL));
    // Registers 3 to 6 are ignored.
    let p = s(&mut e.k, 0, f);
    assert_eq!(sys(&mut e.k, nr::ACCESS, &[p, R_OK, u64::MAX, 0x200, 0x100, 7]), Ok(0));
}

#[test]
fn path_strings_and_forms() {
    let mut e = env();
    assert_eq!(access(&mut e, b"", F_OK), Err(Errno::ENOENT));
    for bad in [0, 1, UNMAPPED, 0xffff_8000_0000_0000] {
        assert_eq!(sys(&mut e.k, nr::ACCESS, &[bad, F_OK]), Err(Errno::EFAULT), "pointer {bad:#x}");
    }
    assert_eq!(access(&mut e, b"/dir/file/", F_OK), Err(Errno::ENOTDIR));
    assert_eq!(access(&mut e, b"/dir/file/x", F_OK), Err(Errno::ENOTDIR));
    assert_eq!(access(&mut e, b"/dir/file/.", F_OK), Err(Errno::ENOTDIR));
    assert_eq!(access(&mut e, b"/dir/", F_OK), Ok(0));
    assert_eq!(access(&mut e, b"/dir/.", R_OK), Ok(0));
    assert_eq!(access(&mut e, b"/dir/../dir/file", R_OK), Ok(0));
    assert_eq!(access(&mut e, b"//dir///file", R_OK), Ok(0));
    assert_eq!(access(&mut e, b"/../../dir", X_OK), Ok(0));

    let mut name = b"/dir/".to_vec();
    name.extend([b'a'; 256]);
    assert_eq!(access(&mut e, &name, F_OK), Err(Errno::ENAMETOOLONG), "256-byte name");
    name.pop();
    assert_eq!(access(&mut e, &name, F_OK), Err(Errno::ENOENT), "255-byte name");

    assert_eq!(access(&mut e, &[b'/'; 4096], F_OK), Err(Errno::ENAMETOOLONG));
    assert_eq!(access(&mut e, &[b'/'; 4095], F_OK), Ok(0));
    assert_eq!(access(&mut e, &[b'/'; 8000], F_OK), Err(Errno::ENAMETOOLONG));
    let mut dots = b"/dir".to_vec();
    while dots.len() < 4095 {
        dots.extend(b"/.");
    }
    dots.truncate(4095);
    assert_eq!(access(&mut e, &dots, X_OK), Ok(0), "4095 bytes of components");
    dots.push(b'.');
    assert_eq!(access(&mut e, &dots, X_OK), Err(Errno::ENAMETOOLONG));
}

#[test]
fn relative_paths_follow_the_working_directory() {
    let mut e = env();
    mkdir(&e, b"/a", 0o755);
    mkdir(&e, b"/b", 0o755);
    file(&e, b"/a/only_a", 0o755);
    file(&e, b"/b/only_b", 0o644);
    assert_eq!(access(&mut e, b"a/only_a", F_OK), Ok(0), "starts at /");
    let d = s(&mut e.k, 0, b"/a");
    assert_eq!(sys(&mut e.k, nr::CHDIR, &[d]), Ok(0));
    assert_eq!(access(&mut e, b"only_a", X_OK), Ok(0));
    assert_eq!(access(&mut e, b"only_b", F_OK), Err(Errno::ENOENT));
    assert_eq!(access(&mut e, b".", R_OK | X_OK), Ok(0));
    assert_eq!(access(&mut e, b"./only_a", R_OK), Ok(0));
    assert_eq!(access(&mut e, b"../b/only_b", R_OK), Ok(0));
    assert_eq!(access(&mut e, b"../b/only_b", X_OK), Err(Errno::EACCES), "root needs an x bit");
    let d = s(&mut e.k, 0, b"/b");
    assert_eq!(sys(&mut e.k, nr::CHDIR, &[d]), Ok(0));
    assert_eq!(access(&mut e, b"only_b", R_OK), Ok(0));
    assert_eq!(access(&mut e, b"only_a", F_OK), Err(Errno::ENOENT));
    assert_eq!(access(&mut e, b"/a/only_a", F_OK), Ok(0), "absolute ignores it");
}

fn links(e: &Env) {
    mkdir(e, b"/l", 0o755);
    mkdir(e, b"/l/sub", 0o755);
    file(e, b"/l/sub/f", 0o644);
    file(e, b"/l/e", 0o755);
    file(e, b"/l/ro", 0o400);
    symlink(e, b"sub/f", b"/l/lf");
    symlink(e, b"/l/e", b"/l/le");
    symlink(e, b"ro", b"/l/lro");
    symlink(e, b"sub", b"/l/ld");
    symlink(e, b"lf", b"/l/lchain");
    symlink(e, b"gone", b"/l/dang");
    symlink(e, b"loopb", b"/l/loopa");
    symlink(e, b"loopa", b"/l/loopb");
    symlink(e, b"self", b"/l/self");
}

#[test]
fn symbolic_links_are_followed_and_their_own_mode_never_counts() {
    for (cred, root) in [(Cred::root(), true), (Cred::user(USER, USER), false)] {
        let mut e = env_as(cred);
        links(&e);
        // The tree is root's: the user is checked against the other bits.
        assert_eq!(access(&mut e, b"/l/lf", F_OK), Ok(0));
        assert_eq!(access(&mut e, b"/l/lf", R_OK | W_OK), if root { Ok(0) } else { Err(Errno::EACCES) });
        assert_eq!(access(&mut e, b"/l/lf", R_OK), Ok(0));
        assert_eq!(access(&mut e, b"/l/lf", X_OK), Err(Errno::EACCES));
        assert_eq!(access(&mut e, b"/l/le", X_OK), Ok(0));
        assert_eq!(access(&mut e, b"/l/lro", W_OK), if root { Ok(0) } else { Err(Errno::EACCES) });
        assert_eq!(access(&mut e, b"/l/ld", X_OK), Ok(0));
        assert_eq!(access(&mut e, b"/l/ld/", F_OK), Ok(0));
        assert_eq!(access(&mut e, b"/l/ld/f", R_OK), Ok(0));
        assert_eq!(access(&mut e, b"/l/lchain", R_OK), Ok(0));
        assert_eq!(access(&mut e, b"/l/dang", F_OK), Err(Errno::ENOENT));
        assert_eq!(access(&mut e, b"/l/dang/", F_OK), Err(Errno::ENOENT));
        assert_eq!(access(&mut e, b"/l/loopa", F_OK), Err(Errno::ELOOP));
        assert_eq!(access(&mut e, b"/l/self", F_OK), Err(Errno::ELOOP));
        assert_eq!(access(&mut e, b"/l/loopa/x", F_OK), Err(Errno::ELOOP));
        assert_eq!(access(&mut e, b"/l/lf/", F_OK), Err(Errno::ENOTDIR));
    }
}

#[test]
fn a_hard_link_shares_its_inodes_mode() {
    let mut e = env_as(Cred::user(USER, USER));
    file(&e, b"/dir/h", 0o600);
    chown(&e, b"/dir/h", USER, USER);
    let (ctx, root) = (e.vfs.root_context().unwrap(), Cred::root());
    e.vfs.link(&ctx, &root, At::Cwd, b"/dir/h", At::Cwd, b"/dir/h2", false).unwrap();
    assert_eq!(access(&mut e, b"/dir/h2", R_OK | W_OK), Ok(0));
    e.vfs.chmod(&ctx, &root, At::Cwd, b"/dir/h", 0o200, true).unwrap();
    assert_eq!(access(&mut e, b"/dir/h2", R_OK), Err(Errno::EACCES));
    assert_eq!(access(&mut e, b"/dir/h2", W_OK), Ok(0));
}

#[test]
fn the_owner_is_checked_against_the_owner_bits_alone() {
    for (cred, root) in [(Cred::root(), true), (Cred::user(USER, USER), false)] {
        let mut e = env_as(cred);
        let den = if root { Ok(0) } else { Err(Errno::EACCES) };
        file(&e, b"/dir/g", 0o077);
        file(&e, b"/dir/o", 0o407);
        if !root {
            chown(&e, b"/dir/g", USER, USER);
            chown(&e, b"/dir/o", USER, USER);
        }
        assert_eq!(access(&mut e, b"/dir/g", R_OK), den);
        assert_eq!(access(&mut e, b"/dir/g", W_OK), den);
        assert_eq!(access(&mut e, b"/dir/g", F_OK), Ok(0));
        assert_eq!(access(&mut e, b"/dir/o", R_OK), Ok(0));
        assert_eq!(access(&mut e, b"/dir/o", W_OK), den);
        // Root: X_OK needs an execute bit in any class (both have one).
        assert_eq!(access(&mut e, b"/dir/g", X_OK), den);
        assert_eq!(access(&mut e, b"/dir/o", X_OK), den);
    }
}

#[test]
fn root_rules() {
    let mut e = env();
    for (perm, x) in [(0o001, true), (0o010, true), (0o100, true), (0o644, false), (0o000, false)] {
        file(&e, b"/dir/r", perm);
        assert_eq!(access(&mut e, b"/dir/r", R_OK | W_OK), Ok(0), "perm {perm:o}");
        assert_eq!(access(&mut e, b"/dir/r", X_OK), if x { Ok(0) } else { Err(Errno::EACCES) }, "perm {perm:o}");
        let (ctx, root) = (e.vfs.root_context().unwrap(), Cred::root());
        e.vfs.unlink(&ctx, &root, At::Cwd, b"/dir/r").unwrap();
    }
    mkdir(&e, b"/dir/z", 0o000);
    assert_eq!(access(&mut e, b"/dir/z", R_OK | W_OK | X_OK), Ok(0), "a directory is always searchable");
}

#[test]
fn group_and_other_classes_use_the_callers_groups() {
    let setup = |cred: Cred| {
        let e = env_as(cred);
        file(&e, b"/dir/grp", 0o060);
        chown(&e, b"/dir/grp", 0, 4321);
        file(&e, b"/dir/other", 0o004);
        e
    };
    let mut e = setup(Cred::user(65534, 100));
    assert_eq!(access(&mut e, b"/dir/grp", R_OK), Err(Errno::EACCES), "wrong group");
    assert_eq!(access(&mut e, b"/dir/other", R_OK), Ok(0));
    assert_eq!(access(&mut e, b"/dir/other", W_OK), Err(Errno::EACCES));
    assert_eq!(access(&mut e, b"/dir", X_OK), Ok(0));
    assert_eq!(access(&mut e, b"/dir", W_OK), Err(Errno::EACCES));
    let mut e = setup(Cred::user(65534, 4321));
    assert_eq!(access(&mut e, b"/dir/grp", R_OK | W_OK), Ok(0));
    assert_eq!(access(&mut e, b"/dir/grp", X_OK), Err(Errno::EACCES));
    let mut e = setup(Cred::user(65534, 100).with_groups(&[4321]));
    assert_eq!(access(&mut e, b"/dir/grp", R_OK | W_OK), Ok(0), "a supplementary group");
}

#[test]
fn other_kinds_are_checked_by_mode_only() {
    for (cred, root) in [(Cred::root(), true), (Cred::user(USER, USER), false)] {
        let mut e = env_as(cred);
        let (ctx, r) = (e.vfs.root_context().unwrap(), Cred::root());
        e.vfs.mknod(&ctx, &r, At::Cwd, b"/dir/p", FileType::Fifo, 0o644, DeviceId::default()).unwrap();
        chown(&e, b"/dir/p", USER, USER);
        let null = DeviceId { major: 1, minor: 3 };
        e.vfs.mknod(&ctx, &r, At::Cwd, b"/dir/null", FileType::CharDevice, 0o666, null).unwrap();
        e.vfs.chmod(&ctx, &r, At::Cwd, b"/dir/null", 0o666, true).unwrap();
        assert_eq!(access(&mut e, b"/dir/p", F_OK), Ok(0));
        assert_eq!(access(&mut e, b"/dir/p", R_OK | W_OK), Ok(0));
        assert_eq!(access(&mut e, b"/dir/p", X_OK), Err(Errno::EACCES));
        assert_eq!(access(&mut e, b"/dir/null", R_OK | W_OK), Ok(0));
        assert_eq!(access(&mut e, b"/dir/null", X_OK), Err(Errno::EACCES));
        mkdir(&e, b"/dir/ro", 0o555);
        assert_eq!(access(&mut e, b"/dir/ro", R_OK | X_OK), Ok(0));
        assert_eq!(access(&mut e, b"/dir/ro", W_OK), if root { Ok(0) } else { Err(Errno::EACCES) });
    }
}

#[test]
fn access_changes_nothing() {
    let mut e = env();
    let (ctx, root) = (e.vfs.root_context().unwrap(), Cred::root());
    let before = e.vfs.stat(&ctx, &root, At::Cwd, b"/dir/file", true).unwrap();
    for _ in 0..20 {
        assert_eq!(access(&mut e, b"/dir/file", R_OK | W_OK), Ok(0));
    }
    assert_eq!(e.vfs.stat(&ctx, &root, At::Cwd, b"/dir/file", true).unwrap(), before);
}

#[test]
fn the_first_problem_on_the_walk_decides() {
    for (cred, root) in [(Cred::root(), true), (Cred::user(USER, USER), false)] {
        let mut e = env_as(cred);
        mkdir(&e, b"/dir/ns", 0o600);
        let long = |prefix: &[u8], suffix: &[u8]| -> Vec<u8> {
            let mut p = prefix.to_vec();
            p.extend([b'b'; 256]);
            p.extend(suffix);
            p
        };
        assert_eq!(access(&mut e, &long(b"/dir/nope/", b""), F_OK), Err(Errno::ENOENT));
        assert_eq!(access(&mut e, &long(b"/dir/file/", b""), F_OK), Err(Errno::ENOTDIR));
        assert_eq!(access(&mut e, &long(b"/dir/", b"/nope"), F_OK), Err(Errno::ENAMETOOLONG));
        assert_eq!(access(&mut e, b"/dir/nope/../file", F_OK), Err(Errno::ENOENT));
        assert_eq!(access(&mut e, b"/dir/file/nope", F_OK), Err(Errno::ENOTDIR));
        let (unsearchable, long_name) =
            if root { (Errno::ENOENT, Errno::ENAMETOOLONG) } else { (Errno::EACCES, Errno::EACCES) };
        assert_eq!(access(&mut e, b"/dir/ns/nope/x", F_OK), Err(unsearchable));
        assert_eq!(access(&mut e, &long(b"/dir/ns/", b""), F_OK), Err(long_name));
        assert_eq!(access(&mut e, b"/dir/ns/x", 8), Err(Errno::EINVAL));
        assert_eq!(access(&mut e, b"/dir/nope", X_OK), Err(Errno::ENOENT));
        assert_eq!(access(&mut e, b"/dir/file/", X_OK), Err(Errno::ENOTDIR));
        let mut too_long = [b'c'; 5000];
        too_long[0] = b'/';
        assert_eq!(access(&mut e, &too_long, F_OK), Err(Errno::ENAMETOOLONG));
    }
}

#[test]
fn search_but_not_read_permission_is_needed_on_the_way() {
    let mut e = env_as(Cred::user(USER, USER));
    mkdir(&e, b"/w", 0o300);
    chown(&e, b"/w", USER, USER);
    file(&e, b"/w/f", 0o644);
    assert_eq!(access(&mut e, b"/w/f", R_OK), Ok(0));
    assert_eq!(access(&mut e, b"/w", R_OK), Err(Errno::EACCES));
    assert_eq!(access(&mut e, b"/w", W_OK | X_OK), Ok(0));
}
