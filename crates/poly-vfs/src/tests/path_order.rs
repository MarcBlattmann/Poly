//! The order of path errors (`path.rs`): when a path has several problems,
//! which one a call reports. The rules and examples are the spec sheets'
//! `path_order` (chdir, faccessat, faccessat2, fchmodat, fchownat, truncate,
//! linkat, mkdirat, newfstatat, openat, readlinkat, renameat2, statfs,
//! statx, unlinkat, utimensat) and symlinkat's `precedence`, in
//! `specs/linux/x86_64/`. Each rule is checked for every call that takes a
//! path, through one table of calls.

use super::Env;
use crate::*;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// A name of 256 bytes: one over `NAME_MAX`.
fn long() -> String {
    "n".repeat(NAME_MAX + 1)
}

/// `/d` with a file, a directory, a dangling link and links to the file and
/// the directory.
fn tree() -> Env {
    let env = Env::new();
    env.mkdir("/d");
    env.mkdir("/d/sub");
    env.write("/d/file", b"f");
    env.write("/d/victim", b"v");
    env.symlink("/nowhere", "/d/dangling");
    env.symlink("file", "/d/tofile");
    env.symlink("sub", "/d/todir");
    env
}

type Call = fn(&Env, At<'_>, &[u8]) -> Result<()>;

/// Every path-taking operation, with the path under test in the place the
/// call takes it (for link and rename: either path, the other one good).
const CALLS: &[(&str, Call)] = &[
    ("stat", |e, at, p| e.vfs.stat(&e.ctx, &e.root, at, p, true).map(drop)),
    ("lstat", |e, at, p| e.vfs.stat(&e.ctx, &e.root, at, p, false).map(drop)),
    ("open", |e, at, p| e.vfs.open(&e.ctx, &e.root, at, p, OpenFlags::RDONLY, 0).map(drop)),
    ("open_creat", |e, at, p| {
        e.vfs.open(&e.ctx, &e.root, at, p, OpenFlags::WRONLY | OpenFlags::CREAT, 0o644).map(drop)
    }),
    ("mkdir", |e, at, p| e.vfs.mkdir(&e.ctx, &e.root, at, p, 0o755)),
    ("mknod", |e, at, p| e.vfs.mknod(&e.ctx, &e.root, at, p, FileType::Fifo, 0o644, DeviceId::default())),
    ("symlink", |e, at, p| e.vfs.symlink(&e.ctx, &e.root, b"x", at, p)),
    ("unlink", |e, at, p| e.vfs.unlink(&e.ctx, &e.root, at, p)),
    ("rmdir", |e, at, p| e.vfs.rmdir(&e.ctx, &e.root, at, p)),
    ("readlink", |e, at, p| e.vfs.readlink(&e.ctx, &e.root, at, p).map(drop)),
    ("access", |e, at, p| e.vfs.access(&e.ctx, &e.root, at, p, Access::NONE, true)),
    ("chmod", |e, at, p| e.vfs.chmod(&e.ctx, &e.root, at, p, 0o644, true)),
    ("chown", |e, at, p| e.vfs.chown(&e.ctx, &e.root, at, p, None, None, true)),
    ("utimens", |e, at, p| e.vfs.utimens(&e.ctx, &e.root, at, p, None, Some(SetTime::Now), true)),
    ("truncate", |e, at, p| e.vfs.truncate(&e.ctx, &e.root, at, p, 0)),
    ("chdir", |e, at, p| e.vfs.chdir(&mut e.ctx.clone(), &e.root, at, p)),
    ("statfs", |e, at, p| e.vfs.lookup(&e.ctx, &e.root, at, p, true).and_then(|n| n.statfs()).map(drop)),
    ("link_old", |e, at, p| e.vfs.link(&e.ctx, &e.root, at, p, At::Cwd, b"/d/made", false)),
    ("link_new", |e, at, p| e.vfs.link(&e.ctx, &e.root, At::Cwd, b"/d/file", at, p, false)),
    ("rename_old", |e, at, p| e.vfs.rename(&e.ctx, &e.root, at, p, At::Cwd, b"/d/made", RenameFlags::default())),
    ("rename_new", |e, at, p| e.vfs.rename(&e.ctx, &e.root, At::Cwd, b"/d/victim", at, p, RenameFlags::default())),
];

/// Checks `path` against every call: each must fail with `want`.
fn every_call(env: &Env, at: At<'_>, path: &[u8], want: Errno) {
    for (name, call) in CALLS {
        assert_eq!(call(env, at, path), Err(want), "{name}({:?})", String::from_utf8_lossy(path));
    }
}

#[test]
fn the_string_as_a_whole_comes_first() {
    let env = tree();
    // Empty: ENOENT, even from a file as the start.
    every_call(&env, At::Cwd, b"", Errno::ENOENT);
    let file = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/d/file", true).unwrap();
    every_call(&env, At::Node(&file), b"", Errno::ENOENT);
    // PATH_MAX bytes with its NUL or more: ENAMETOOLONG, whatever its
    // components are (missing, a file, a long name).
    for start in ["/d/nope/", "/d/file/", "d/", "/"] {
        let mut path = Vec::from(start.as_bytes());
        path.extend_from_slice(long().as_bytes());
        path.resize(PATH_MAX, b'x');
        every_call(&env, At::Cwd, &path, Errno::ENAMETOOLONG);
        every_call(&env, At::Node(&file), &path, Errno::ENAMETOOLONG);
    }
}

#[test]
fn a_start_on_a_non_directory_comes_before_the_components() {
    let env = tree();
    let file = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/d/file", true).unwrap();
    // symlinkat("x", <fd on a file>, <256 bytes>) is ENOTDIR (the spec's
    // file_dirfd_before_long_name_enotdir), and so for every call and
    // whatever the path's components would give.
    for path in [long(), format!("{}/x", long()), String::from("nope"), String::from("nope/x"), String::from(".")] {
        every_call(&env, At::Node(&file), path.as_bytes(), Errno::ENOTDIR);
    }
    // An absolute path does not look at the start.
    let abs = format!("/d/{}", long());
    every_call(&env, At::Node(&file), abs.as_bytes(), Errno::ENAMETOOLONG);
}

#[test]
fn the_first_failing_component_decides() {
    let env = tree();
    let l = long();
    let sub = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/d", true).unwrap();
    let cases: Vec<(String, Errno)> = vec![
        // A missing name before a long one: ENOENT.
        (format!("/d/nope/{l}/x"), Errno::ENOENT),
        (format!("/d/nope/{l}"), Errno::ENOENT),
        (format!("/d/nope/../{l}"), Errno::ENOENT),
        // A dangling link is a missing name, and ".." does not undo it.
        (format!("/d/dangling/{l}"), Errno::ENOENT),
        (String::from("/d/dangling/../file"), Errno::ENOENT),
        // A non-directory (or a link to one) with more after it: ENOTDIR,
        // before the long name after it (the spec's
        // file_before_long_name_enotdir, file_before_long_middle_enotdir).
        (format!("/d/file/{l}"), Errno::ENOTDIR),
        (format!("/d/file/{l}/x"), Errno::ENOTDIR),
        (format!("/d/file/nope/{l}"), Errno::ENOTDIR),
        (format!("/d/tofile/{l}"), Errno::ENOTDIR),
        // A long name whose directories all exist: ENAMETOOLONG, before
        // anything after it.
        (format!("/d/{l}"), Errno::ENAMETOOLONG),
        (format!("/d/{l}/nope/x"), Errno::ENAMETOOLONG),
        (format!("/d/{l}/file/x"), Errno::ENAMETOOLONG),
        (format!("/d/todir/{l}"), Errno::ENAMETOOLONG),
        (format!("/d/sub/../{l}"), Errno::ENAMETOOLONG),
    ];
    for (path, want) in &cases {
        every_call(&env, At::Cwd, path.as_bytes(), *want);
        // The same relative to a directory start.
        let rel = path.strip_prefix("/d/").unwrap();
        every_call(&env, At::Node(&sub), rel.as_bytes(), *want);
    }
}

#[test]
fn symlink_checks_its_target_before_its_linkpath() {
    let env = tree();
    let sl = |target: &[u8], path: &str| env.vfs.symlink(&env.ctx, &env.root, target, At::Cwd, path.as_bytes());
    // symlinkat("", D/existing) is ENOENT; a target of PATH_MAX bytes or
    // more is ENAMETOOLONG even where the linkpath would fail.
    assert_eq!(sl(b"", "/d/file"), Err(Errno::ENOENT));
    assert_eq!(sl(b"", &format!("/d/file/{}", long())), Err(Errno::ENOENT));
    let target = vec![b't'; PATH_MAX];
    assert_eq!(sl(&target, "/d/nope/x"), Err(Errno::ENAMETOOLONG));
    assert_eq!(sl(&target, "/d/file"), Err(Errno::ENAMETOOLONG));
    // The longest target is fine; then the linkpath decides.
    let target = vec![b't'; PATH_MAX - 1];
    assert_eq!(sl(&target, &format!("/d/file/{}", long())), Err(Errno::ENOTDIR));
    assert_eq!(sl(&target, "/d/file"), Err(Errno::EEXIST));
    assert_eq!(sl(&target, "/d/longest"), Ok(()));
}

/// A second path given as a [`PathArg`]: its string or start may be bad.
fn arg<'a>(path: Result<&'a [u8]>, at: Result<At<'a>>) -> PathArg<'a> {
    PathArg { path, at }
}

#[test]
fn link_resolves_its_old_path_completely_first() {
    let env = tree();
    let l = long();
    let link = |old: &str, new: PathArg<'_>| {
        env.vfs.link_paths(&env.ctx, &env.root, PathArg::new(At::Cwd, old.as_bytes()), new, false)
    };
    let too_long = vec![b'/'; PATH_MAX];
    let bad_strings: [Result<&[u8]>; 4] = [Err(Errno::EFAULT), Err(Errno::ENAMETOOLONG), Ok(b""), Ok(&too_long)];
    // Any error of oldpath wins over every error of newpath: its string,
    // its start, the components of its directories and its last name.
    for new in bad_strings {
        assert_eq!(link("/d/missing", arg(new, Ok(At::Cwd))), Err(Errno::ENOENT));
        assert_eq!(link(&format!("/d/{l}"), arg(new, Ok(At::Cwd))), Err(Errno::ENAMETOOLONG));
    }
    assert_eq!(link(&format!("/d/{l}"), arg(Ok(b"x"), Err(Errno::EBADF))), Err(Errno::ENAMETOOLONG));
    assert_eq!(link("/d/file/x", arg(Ok(b"x"), Err(Errno::EBADF))), Err(Errno::ENOTDIR));
    assert_eq!(link("/d/sub", arg(Ok(b"/d/new"), Ok(At::Cwd))), Err(Errno::EPERM));
    let bad_old = env.vfs.link_paths(
        &env.ctx,
        &env.root,
        arg(Ok(b"file"), Err(Errno::EBADF)),
        arg(Err(Errno::EFAULT), Ok(At::Cwd)),
        false,
    );
    assert_eq!(bad_old, Err(Errno::EBADF), "olddirfd before newpath's string");
    // Then newpath: its string, then its start (for a relative path), then
    // its components, then EEXIST.
    assert_eq!(link("/d/file", arg(Err(Errno::EFAULT), Err(Errno::EBADF))), Err(Errno::EFAULT));
    assert_eq!(link("/d/file", arg(Ok(b""), Err(Errno::EBADF))), Err(Errno::ENOENT));
    assert_eq!(link("/d/file", arg(Ok(&too_long), Err(Errno::EBADF))), Err(Errno::ENAMETOOLONG));
    assert_eq!(link("/d/file", arg(Ok(l.as_bytes()), Err(Errno::EBADF))), Err(Errno::EBADF));
    assert_eq!(link("/d/file", arg(Ok(b"/d/sub"), Err(Errno::EBADF))), Err(Errno::EEXIST), "absolute");
    assert_eq!(link("/d/file", arg(Ok(format!("/d/nope/{l}").as_bytes()), Ok(At::Cwd))), Err(Errno::ENOENT));
    assert_eq!(link("/d/file", arg(Ok(format!("/d/sub/{l}").as_bytes()), Ok(At::Cwd))), Err(Errno::ENAMETOOLONG));
    assert_eq!(link("/d/file", arg(Ok(b"/d/sub"), Ok(At::Cwd))), Err(Errno::EEXIST));
    assert_eq!(link("/d/file", arg(Ok(b"/d/linked"), Err(Errno::EBADF))), Ok(()));
    // AT_EMPTY_PATH: the new path's errors after the file's.
    let file = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/d/file", true).unwrap();
    let node = |new| env.vfs.link_node(&env.ctx, &env.root, &file, new);
    assert_eq!(node(arg(Err(Errno::EFAULT), Ok(At::Cwd))), Err(Errno::EFAULT));
    assert_eq!(node(arg(Ok(b"x"), Err(Errno::EBADF))), Err(Errno::EBADF));
    assert_eq!(node(arg(Ok(format!("/d/file/{l}").as_bytes()), Ok(At::Cwd))), Err(Errno::ENOTDIR));
}

#[test]
fn rename_checks_both_directories_before_either_last_name() {
    let env = tree();
    let l = long();
    let rename = |old: &str, new: PathArg<'_>| {
        let old = PathArg::new(At::Cwd, old.as_bytes());
        env.vfs.rename_paths(&env.ctx, &env.root, old, new, RenameFlags::default())
    };
    let good = |p: &'static str| arg(Ok(p.as_bytes()), Ok(At::Cwd));
    let too_long = vec![b'/'; PATH_MAX];
    let long_old = format!("/d/{l}");
    // The spec's examples: newpath's directories come before oldpath's
    // last name ...
    let long_new = format!("/d/{l}");
    assert_eq!(rename(&long_old, good("/d/nope/x")), Err(Errno::ENOENT));
    assert_eq!(rename(&long_old, good("/d/file/x")), Err(Errno::ENOTDIR));
    assert_eq!(rename("/d/missing", arg(Ok(b"x"), Err(Errno::EBADF))), Err(Errno::EBADF));
    assert_eq!(rename("/d/missing", arg(Ok(&too_long), Ok(At::Cwd))), Err(Errno::ENAMETOOLONG));
    assert_eq!(rename("/d/missing", arg(Err(Errno::EFAULT), Ok(At::Cwd))), Err(Errno::EFAULT));
    // ... oldpath's last name before newpath's ...
    assert_eq!(rename("/d/missing", arg(Ok(long_new.as_bytes()), Ok(At::Cwd))), Err(Errno::ENOENT));
    assert_eq!(rename(&long_old, arg(Ok(long_new.as_bytes()), Ok(At::Cwd))), Err(Errno::ENAMETOOLONG));
    // ... and oldpath's directories before anything of newpath.
    assert_eq!(rename("/d/nope/x", arg(Ok(&too_long), Ok(At::Cwd))), Err(Errno::ENOENT));
    assert_eq!(rename("/d/nope/x", arg(Err(Errno::EFAULT), Err(Errno::EBADF))), Err(Errno::ENOENT));
    assert_eq!(rename("/d/file/x", arg(Ok(b"x"), Err(Errno::EBADF))), Err(Errno::ENOTDIR));
    assert_eq!(rename(&format!("/d/{l}/x"), arg(Ok(b"x"), Err(Errno::EBADF))), Err(Errno::ENAMETOOLONG));
    // newpath's last name: over 255 bytes after oldpath is found.
    assert_eq!(rename("/d/victim", arg(Ok(long_new.as_bytes()), Ok(At::Cwd))), Err(Errno::ENAMETOOLONG));
    // A good pair still renames; an absolute newpath ignores its start.
    assert_eq!(rename("/d/victim", arg(Ok(b"/d/renamed"), Err(Errno::EBADF))), Ok(()));
    assert!(env.stat("/d/renamed").is_ok());
}
