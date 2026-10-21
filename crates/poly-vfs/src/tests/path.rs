//! Path resolution: `path_resolution(7)` and the ENOENT / ENOTDIR /
//! ELOOP / ENAMETOOLONG cases of `open(2)` and `stat(2)`.

use super::Env;
use crate::*;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

fn tree() -> Env {
    let env = Env::new();
    env.mkdir("/a");
    env.mkdir("/a/b");
    env.mkdir("/x");
    env.mkdir("/x/y");
    env.write("/a/b/file", b"in b");
    env.write("/x/y/target", b"in y");
    env.write("/top", b"top");
    env
}

#[test]
fn absolute_relative_dot_and_dotdot() {
    let mut env = tree();
    let b = env.ino("/a/b").unwrap();
    assert_eq!(env.ino("/a/b/."), Ok(b));
    assert_eq!(env.ino("/a/./b"), Ok(b));
    assert_eq!(env.ino("//a///b//"), Ok(b));
    assert_eq!(env.ino("/a/b/../b"), Ok(b));
    assert_eq!(env.ino("/x/../a/b"), Ok(b));
    let root = env.ino("/").unwrap();
    assert_eq!(env.ino("/.."), Ok(root));
    assert_eq!(env.ino("/../../.."), Ok(root));
    assert_eq!(env.ino("/a/../.."), Ok(root));
    assert_eq!(env.ino("."), Ok(root));
    assert_eq!(env.ino(".."), Ok(root));

    env.cd("/a").unwrap();
    assert_eq!(env.ino("b"), Ok(b));
    assert_eq!(env.ino("./b/"), Ok(b));
    assert_eq!(env.read("b/file").unwrap(), b"in b");
    assert_eq!(env.ino(".."), Ok(root));
    assert_eq!(env.ino("../a/b"), Ok(b));
    assert_eq!(env.read("/top").unwrap(), b"top");
}

#[test]
fn relative_to_a_directory_node() {
    let env = tree();
    let dir = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/a", true).unwrap();
    let st = env.vfs.stat(&env.ctx, &env.root, At::Node(&dir), b"b/file", true).unwrap();
    assert_eq!(st.size, 4);
    // An absolute path ignores the node.
    let st = env.vfs.stat(&env.ctx, &env.root, At::Node(&dir), b"/top", true).unwrap();
    assert_eq!(st.size, 3);
    // A relative path from a file is ENOTDIR.
    let file = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/top", true).unwrap();
    assert_eq!(env.vfs.stat(&env.ctx, &env.root, At::Node(&file), b"x", true), Err(Errno::ENOTDIR));
    assert_eq!(env.vfs.stat(&env.ctx, &env.root, At::Node(&file), b".", true), Err(Errno::ENOTDIR));
}

#[test]
fn missing_and_non_directory_components() {
    let env = tree();
    assert_eq!(env.stat("/nope"), Err(Errno::ENOENT));
    assert_eq!(env.stat("/nope/x"), Err(Errno::ENOENT));
    assert_eq!(env.stat("/a/nope/x"), Err(Errno::ENOENT));
    assert_eq!(env.stat("/top/x"), Err(Errno::ENOTDIR));
    assert_eq!(env.stat("/top/."), Err(Errno::ENOTDIR));
    assert_eq!(env.stat("/top/.."), Err(Errno::ENOTDIR));
    assert_eq!(env.stat("/a/b/file/x/y"), Err(Errno::ENOTDIR));
    assert_eq!(env.stat(""), Err(Errno::ENOENT));
}

#[test]
fn trailing_slashes() {
    let env = tree();
    assert!(env.stat("/a/").is_ok());
    assert!(env.stat("/a//").is_ok());
    assert_eq!(env.stat("/top/"), Err(Errno::ENOTDIR));
    assert_eq!(env.stat("/top//"), Err(Errno::ENOTDIR));
    assert_eq!(env.stat("/nope/"), Err(Errno::ENOENT));
    assert_eq!(env.open("/top/", OpenFlags::RDONLY).map(|_| ()), Err(Errno::ENOTDIR));
    // Creating a regular file with a trailing slash: EISDIR (Linux).
    assert_eq!(env.open("/new/", OpenFlags::WRONLY | OpenFlags::CREAT).map(|_| ()), Err(Errno::EISDIR));
    // mkdir accepts one.
    env.vfs.mkdir(&env.ctx, &env.root, At::Cwd, b"/newdir/", 0o755).unwrap();
    assert!(env.stat("/newdir").is_ok());
    // Removing: a file with a slash is ENOTDIR, a directory is fine.
    assert_eq!(env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/top/"), Err(Errno::ENOTDIR));
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/newdir/").unwrap();
    // symlink / mknod / link onto a name with a slash: ENOENT.
    assert_eq!(env.vfs.symlink(&env.ctx, &env.root, b"/top", At::Cwd, b"/sl/"), Err(Errno::ENOENT));
    assert_eq!(env.vfs.link(&env.ctx, &env.root, At::Cwd, b"/top", At::Cwd, b"/hl/", false), Err(Errno::ENOENT));
    // symlink onto an existing file with a slash: the file is used as a
    // directory, ENOTDIR (spec sheet linux/syscall/x86_64/symlinkat), also
    // through a link to it; a directory (or a link to one) is EEXIST.
    let sl = |path: &[u8]| env.vfs.symlink(&env.ctx, &env.root, b"x", At::Cwd, path);
    assert_eq!(sl(b"/top/"), Err(Errno::ENOTDIR));
    assert_eq!(sl(b"/top//"), Err(Errno::ENOTDIR));
    assert_eq!(sl(b"/top"), Err(Errno::EEXIST));
    env.symlink("top", "/to_top");
    env.symlink("a", "/to_a");
    assert_eq!(sl(b"/to_top/"), Err(Errno::ENOTDIR));
    assert_eq!(sl(b"/to_a/"), Err(Errno::EEXIST));
    assert_eq!(sl(b"/a/"), Err(Errno::EEXIST));
}

#[test]
fn symlinks_to_files_and_directories() {
    let env = tree();
    env.symlink("/x/y", "/a/abs");
    env.symlink("../../x/y/target", "/a/b/rel");
    env.symlink("b", "/a/tob");
    assert_eq!(env.read("/a/abs/target").unwrap(), b"in y");
    assert_eq!(env.read("/a/b/rel").unwrap(), b"in y");
    assert_eq!(env.read("/a/tob/file").unwrap(), b"in b");
    // stat follows, lstat does not.
    assert_eq!(env.stat("/a/b/rel").unwrap().kind(), Some(FileType::Regular));
    let l = env.lstat("/a/b/rel").unwrap();
    assert_eq!(l.kind(), Some(FileType::Symlink));
    assert_eq!(l.size, 16);
    assert_eq!(l.perm(), 0o777);
    // A trailing slash follows even for lstat.
    assert_eq!(env.lstat("/a/abs/").unwrap().kind(), Some(FileType::Directory));
    assert_eq!(env.lstat("/a/abs").unwrap().kind(), Some(FileType::Symlink));
    // Through a link to a file with a slash: ENOTDIR.
    assert_eq!(env.stat("/a/b/rel/"), Err(Errno::ENOTDIR));
    // readlink returns the target as stored.
    assert_eq!(env.vfs.readlink(&env.ctx, &env.root, At::Cwd, b"/a/b/rel").unwrap(), b"../../x/y/target");
    assert_eq!(env.vfs.readlink(&env.ctx, &env.root, At::Cwd, b"/top"), Err(Errno::EINVAL));
}

#[test]
fn dotdot_after_a_symlink_is_physical() {
    let env = tree();
    env.symlink("/x/y", "/a/link");
    // /a/link/.. is the parent of /x/y, i.e. /x, not /a.
    assert_eq!(env.ino("/a/link/.."), env.ino("/x"));
    assert_eq!(env.ino("/a/link/../.."), env.ino("/"));
}

#[test]
fn dangling_links() {
    let env = tree();
    env.symlink("/nowhere", "/dangle");
    assert_eq!(env.stat("/dangle"), Err(Errno::ENOENT));
    assert_eq!(env.lstat("/dangle").unwrap().kind(), Some(FileType::Symlink));
    // Creating through a dangling link creates its target.
    let f = env.open("/dangle", OpenFlags::WRONLY | OpenFlags::CREAT).unwrap();
    f.write(b"made").unwrap();
    assert_eq!(env.read("/nowhere").unwrap(), b"made");
    // ... but not with O_EXCL, which refuses any existing name.
    env.symlink("/nowhere2", "/dangle2");
    assert_eq!(
        env.open("/dangle2", OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::EXCL).map(|_| ()),
        Err(Errno::EEXIST)
    );
    assert_eq!(env.stat("/nowhere2"), Err(Errno::ENOENT));
    // An empty target cannot be made.
    assert_eq!(env.vfs.symlink(&env.ctx, &env.root, b"", At::Cwd, b"/empty"), Err(Errno::ENOENT));
}

#[test]
fn symlink_loops_and_the_limit() {
    let env = tree();
    env.symlink("/loop2", "/loop1");
    env.symlink("/loop1", "/loop2");
    env.symlink("self", "/self");
    assert_eq!(env.stat("/loop1"), Err(Errno::ELOOP));
    assert_eq!(env.stat("/loop1/x"), Err(Errno::ELOOP));
    assert_eq!(env.stat("/self"), Err(Errno::ELOOP));
    // Not following the last component: no loop.
    assert_eq!(env.lstat("/loop1").unwrap().kind(), Some(FileType::Symlink));
    // O_NOFOLLOW on a link: ELOOP.
    assert_eq!(env.open("/self", OpenFlags::RDONLY | OpenFlags::NOFOLLOW).map(|_| ()), Err(Errno::ELOOP));

    // A chain of exactly MAX_SYMLINKS links resolves; one more does not.
    let n = MAX_SYMLINKS as usize;
    for i in 0..=n {
        let target = if i == 0 { String::from("/top") } else { alloc::format!("/l{}", i - 1) };
        env.symlink(&target, &alloc::format!("/l{i}"));
    }
    assert_eq!(env.read(&alloc::format!("/l{}", n - 1)).unwrap(), b"top");
    assert_eq!(env.read(&alloc::format!("/l{n}")), Err(Errno::ELOOP));
    // Links in the middle of a path count too.
    for i in 0..=n {
        let target = if i == 0 { String::from("/a") } else { alloc::format!("/d{}", i - 1) };
        env.symlink(&target, &alloc::format!("/d{i}"));
    }
    assert!(env.stat(&alloc::format!("/d{}/b/file", n - 1)).is_ok());
    assert_eq!(env.stat(&alloc::format!("/d{n}/b/file")), Err(Errno::ELOOP));
}

#[test]
fn name_and_path_length_limits() {
    let env = tree();
    let ok = "n".repeat(NAME_MAX);
    env.write(&alloc::format!("/{ok}"), b"long");
    assert_eq!(env.read(&alloc::format!("/{ok}")).unwrap(), b"long");
    let too_long = "n".repeat(NAME_MAX + 1);
    assert_eq!(env.stat(&alloc::format!("/{too_long}")), Err(Errno::ENAMETOOLONG));
    assert_eq!(env.stat(&alloc::format!("/{too_long}/x")), Err(Errno::ENAMETOOLONG));
    assert_eq!(
        env.vfs.mkdir(&env.ctx, &env.root, At::Cwd, alloc::format!("/{too_long}").as_bytes(), 0o755),
        Err(Errno::ENAMETOOLONG)
    );

    // PATH_MAX counts the terminating NUL: 4095 bytes is the longest path.
    let mut longest: Vec<u8> = vec![b'/'; PATH_MAX - 1];
    assert_eq!(env.vfs.stat(&env.ctx, &env.root, At::Cwd, &longest, true).map(|s| s.ino), env.ino("/"));
    longest.push(b'/');
    assert_eq!(env.vfs.stat(&env.ctx, &env.root, At::Cwd, &longest, true), Err(Errno::ENAMETOOLONG));
}

#[test]
fn getcwd_and_chdir() {
    let mut env = tree();
    assert_eq!(env.cwd(), b"/");
    env.cd("/a/b").unwrap();
    assert_eq!(env.cwd(), b"/a/b");
    env.cd("..").unwrap();
    assert_eq!(env.cwd(), b"/a");
    // Through a symlink: the physical path.
    env.symlink("/x/y", "/a/link");
    env.cd("link").unwrap();
    assert_eq!(env.cwd(), b"/x/y");
    assert_eq!(env.cd("/top"), Err(Errno::ENOTDIR));
    assert_eq!(env.cd("/nope"), Err(Errno::ENOENT));
    assert_eq!(env.cwd(), b"/x/y");

    // Removing the working directory: getcwd fails, ".." still works,
    // nothing can be created in it.
    env.mkdir("/gone");
    env.cd("/gone").unwrap();
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/gone").unwrap();
    assert_eq!(env.vfs.getcwd(&env.ctx), Err(Errno::ENOENT));
    assert_eq!(env.stat(".").unwrap().nlink, 0);
    assert_eq!(env.open("new", OpenFlags::WRONLY | OpenFlags::CREAT).map(|_| ()), Err(Errno::ENOENT));
    let dot = env.open(".", OpenFlags::RDONLY | OpenFlags::DIRECTORY).unwrap();
    assert_eq!(dot.read_dir(&mut |_| true), Err(Errno::ENOENT), "listing it: getdents64(2) ENOENT");
    env.cd("..").unwrap();
    assert_eq!(env.cwd(), b"/");
}

#[test]
fn chroot_confines_dotdot_and_absolute_paths() {
    let mut env = tree();
    let cred = env.root.clone();
    env.vfs.chroot(&mut env.ctx, &cred, At::Cwd, b"/x").unwrap();
    // chroot leaves the working directory alone; "/" now means /x.
    env.cd("/").unwrap();
    assert_eq!(env.cwd(), b"/");
    assert_eq!(env.read("/y/target").unwrap(), b"in y");
    assert_eq!(env.read("/../../y/target").unwrap(), b"in y");
    assert_eq!(env.stat("/a"), Err(Errno::ENOENT));
    env.cd("y").unwrap();
    assert_eq!(env.cwd(), b"/y");
    // An absolute symlink resolves inside the new root.
    env.symlink("/y/target", "/y/abs");
    assert_eq!(env.read("/y/abs").unwrap(), b"in y");
    // Only root may chroot.
    let user = Cred::user(1000, 1000);
    assert_eq!(env.vfs.chroot(&mut env.ctx, &user, At::Cwd, b"/y"), Err(Errno::EPERM));
}
